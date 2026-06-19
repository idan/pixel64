#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use core::net::Ipv4Addr;

use embassy_executor::Spawner;
use embassy_net::Stack;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pin, Pull};
use esp_hal::system::software_reset;
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hub75::{Hub75, Hub75Pins16};
use esp_radio::ble::controller::BleConnector;
use esp_radio::wifi::WifiController;
use heapless::String;
use log::{error, info, warn};
use trouble_host::prelude::ExternalController;

use pixel64::display::{self, FrameBuffer, Screen};
use pixel64::storage::CredStore;
use pixel64::{improv, net};

#[panic_handler]
fn panic(panic_info: &core::panic::PanicInfo) -> ! {
    error!("{}", panic_info);
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // Wi-Fi + BLE coexistence (provisioning connects while advertising) wants a generous heap.
    esp_alloc::heap_allocator!(size: 128 * 1024);

    // GPIO8 drives the onboard WS2812 RGB LED, which we don't use — hold it low so it stays dark.
    let _onboard_led_off = Output::new(peripherals.GPIO8, Level::Low, OutputConfig::default());

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);
    info!("Embassy initialized!");

    // --- Display ---
    // KEEP IN SYNC: docs/hardware-wiring.md holds the full pin map (the "GND"-silked pin 12 is D,
    // and B is on GPIO14 because GPIO8 is the onboard LED).
    let pins = Hub75Pins16 {
        red1: peripherals.GPIO19.degrade(),
        grn1: peripherals.GPIO20.degrade(),
        blu1: peripherals.GPIO21.degrade(),
        red2: peripherals.GPIO22.degrade(),
        grn2: peripherals.GPIO23.degrade(),
        blu2: peripherals.GPIO15.degrade(),
        addr0: peripherals.GPIO2.degrade(),  // A
        addr1: peripherals.GPIO14.degrade(), // B  (off GPIO8 — that's the onboard RGB LED)
        addr2: peripherals.GPIO1.degrade(),  // C
        addr3: peripherals.GPIO0.degrade(),  // D  (pin 12, silk says "GND")
        addr4: peripherals.GPIO3.degrade(),  // E
        blank: peripherals.GPIO5.degrade(),  // OE
        clock: peripherals.GPIO7.degrade(),  // CLK
        latch: peripherals.GPIO6.degrade(),  // LAT
    };
    let tx_descriptors = esp_hub75::hub75_dma_descriptors!(FrameBuffer);
    let hub75 = Hub75::new_async(
        peripherals.PARL_IO,
        pins,
        peripherals.DMA_CH0,
        tx_descriptors,
        Rate::from_mhz(20),
    )
    .expect("failed to create Hub75 driver");
    display::start(hub75, spawner);
    display::set_screen(Screen::Booting);

    // --- Persistent storage ---
    let mut store = CredStore::new(peripherals.FLASH).expect("storage init failed");

    // --- Boot state machine: stored creds -> connect; otherwise Improv setup ---
    // Wi-Fi is NOT started up front: during BLE setup the radio stays uncontended (no coex
    // starving the GATT stack or strobing the display). It's brought up lazily on the first
    // credential attempt — see improv::NetSource.
    // If stored creds connect directly, this holds the live (controller, stack, ip).
    let mut online_net: Option<(WifiController<'static>, Stack<'static>, Ipv4Addr)> = None;
    let net_source;
    if let Some(creds) = store.load().await {
        info!("found stored credentials for '{}', connecting", creds.ssid);
        let mut showing: String<32> = String::new();
        let _ = showing.push_str(&creds.ssid);
        display::set_screen(Screen::Connecting(showing));
        let (mut wifi, stack) = net::start(peripherals.WIFI, spawner).expect("wifi start failed");
        match net::connect(&mut wifi, &stack, &creds.ssid, &creds.password).await {
            Ok(ip) => {
                online_net = Some((wifi, stack, ip));
                net_source = None;
            }
            Err(e) => {
                warn!("stored credentials failed to connect: {:?}", e);
                net_source = Some(improv::NetSource::Ready { wifi, stack });
            }
        }
    } else {
        net_source = Some(improv::NetSource::Lazy {
            wifi: peripherals.WIFI,
            spawner,
        });
    }

    // Keep the Wi-Fi controller + stack bound for the whole program (main never returns) so the
    // connection persists after onboarding. `_wifi`/`_stack` are intentionally held, not dropped.
    let (_wifi, _stack, ip) = match online_net {
        Some(online) => online,
        None => {
            info!("no working credentials — entering Improv setup mode");
            display::set_screen(Screen::Setup);
            let connector = BleConnector::new(peripherals.BT, Default::default()).unwrap();
            let ble: ExternalController<_, 1> = ExternalController::new(connector);
            improv::run_setup(ble, net_source.unwrap(), &mut store).await
        }
    };

    info!("online at {}", ip);
    display::set_screen(Screen::Online(ip));

    // Factory reset: hold BOOT (GPIO9) for ~3 s to wipe stored credentials and restart into
    // setup mode. (BOOT is a strapping pin, so we poll it at runtime — holding it across the
    // power-on reset would instead enter download mode. We also wait for release before
    // resetting so the restart itself doesn't strap into download mode.)
    let boot = Input::new(peripherals.GPIO9, InputConfig::default().with_pull(Pull::Up));
    let mut held_ms = 0u32;
    loop {
        Timer::after(Duration::from_millis(100)).await;
        if boot.is_low() {
            held_ms += 100;
            if held_ms >= 3000 {
                warn!("BOOT held — wiping Wi-Fi credentials; release BOOT to restart");
                let _ = store.clear().await;
                while boot.is_low() {
                    Timer::after(Duration::from_millis(50)).await;
                }
                Timer::after(Duration::from_millis(100)).await; // let the log flush
                software_reset();
            }
        } else {
            held_ms = 0;
        }
    }
}
