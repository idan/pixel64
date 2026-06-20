//! pixel64 — RP2350 / Pico 2 W.
//!
//! Bring-up milestone 2b: BLE peripheral on the cyw43 radio via trouble-host. Proves the BLE
//! controller swap (esp-radio BleConnector → cyw43 BtDriver, both behind bt-hci's
//! ExternalController) independently of the Improv logic. Advertises as `pixel64` with a battery
//! service — connect with a BLE scanner (nRF Connect / LightBlue) to verify. Wi-Fi join (M2a) is
//! intentionally dropped here to isolate BLE; M2c brings both back together with the Improv port
//! and the concurrency test. See docs/pico-port.md.

#![no_std]
#![no_main]
// trouble's #[gatt_service] macro expands to code with redundant borrows on each characteristic.
#![allow(clippy::needless_borrows_for_generic_args)]

use cyw43_pio::{PioSpi, RM2_CLOCK_DIVIDER};
use embassy_executor::Spawner;
use embassy_futures::select::{select, Either};
use embassy_rp::bind_interrupts;
use embassy_rp::dma::{Channel, InterruptHandler as DmaInterruptHandler};
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIO0, USB};
use embassy_rp::pio::{InterruptHandler as PioInterruptHandler, Pio};
use embassy_rp::usb::{Driver, InterruptHandler as UsbInterruptHandler};
use log::{info, warn};
use static_cell::StaticCell;
use trouble_host::prelude::*;

const DEVICE_NAME: &str = "pixel64";
const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 2;

/// Minimal GATT server: a standard Battery Service so the device shows up recognizably in a BLE
/// scanner. (M2c replaces this with the Improv service.)
#[gatt_server]
struct Server {
    battery: BatteryService,
}

#[gatt_service(uuid = "0000180f-0000-1000-8000-00805f9b34fb")]
struct BatteryService {
    #[characteristic(uuid = "00002a19-0000-1000-8000-00805f9b34fb", read, notify)]
    level: u8,
}

// Program metadata for `picotool info`. The embassy-rp `binary-info` feature emits the RP2350 boot
// image-def block (.start_block) on its own, so no manual `ImageDef` is needed.
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [embassy_rp::binary_info::EntryAddr; 3] = [
    embassy_rp::binary_info::rp_program_name!(c"pixel64"),
    embassy_rp::binary_info::rp_cargo_version!(),
    embassy_rp::binary_info::rp_program_build_attribute!(),
];

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => UsbInterruptHandler<USB>;
    PIO0_IRQ_0 => PioInterruptHandler<PIO0>;
    DMA_IRQ_0 => DmaInterruptHandler<DMA_CH0>;
});

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    // No probe → no defmt/RTT; just halt. (A panic before USB enumerates is invisible over
    // USB-serial — a known no-probe limitation, see docs/pico-port.md.)
    loop {}
}

/// Runs the USB device + CDC-ACM serial class and pumps `log` records out over it.
#[embassy_executor::task]
async fn logger_task(driver: Driver<'static, USB>) {
    embassy_usb_logger::run!(1024, log::LevelFilter::Info, driver);
}

/// Drives the cyw43 chip's low-level SPI event loop (Wi-Fi + BLE traffic). Runs forever.
#[embassy_executor::task]
async fn cyw43_task(
    runner: cyw43::Runner<'static, cyw43::SpiBus<Output<'static>, PioSpi<'static, PIO0, 0>>>,
) -> ! {
    runner.run().await
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // USB-serial logging on the same cable that flashes the board.
    let driver = Driver::new(p.USB, Irqs);
    spawner.spawn(logger_task(driver).unwrap());

    info!("pixel64: bringing up cyw43 radio (Wi-Fi + BT)…");

    // cyw43 firmware blobs (vendored). `fw`/`btfw`/`nvram` need 4-byte alignment (aligned_bytes!);
    // `clm` is passed to control.init() as a plain slice. `btfw` is the Bluetooth firmware.
    let fw = cyw43::aligned_bytes!("../../cyw43-firmware/43439A0.bin");
    let btfw = cyw43::aligned_bytes!("../../cyw43-firmware/43439A0_btfw.bin");
    let nvram = cyw43::aligned_bytes!("../../cyw43-firmware/nvram_rp2040.bin");
    let clm: &[u8] = include_bytes!("../../cyw43-firmware/43439A0_clm.bin");

    // CYW43439 on GP23 (power) + PIO-emulated SPI on GP24/25/29 (see docs/pico-pinout.md).
    let pwr = Output::new(p.PIN_23, Level::Low);
    let cs = Output::new(p.PIN_25, Level::High);
    let Pio {
        mut common,
        sm0,
        irq0,
        ..
    } = Pio::new(p.PIO0, Irqs);
    let spi = PioSpi::new(
        &mut common,
        sm0,
        RM2_CLOCK_DIVIDER,
        irq0,
        cs,
        p.PIN_24, // DIO
        p.PIN_29, // CLK
        Channel::new(p.DMA_CH0, Irqs),
    );

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());
    // new_with_bluetooth: same runner drives Wi-Fi + BT; bt_device is the bt-hci transport.
    let (_net_device, bt_device, mut control, runner) =
        cyw43::new_with_bluetooth(state, pwr, spi, fw, btfw, nvram).await;
    spawner.spawn(cyw43_task(runner).unwrap());
    control.init(clm).await;
    info!("pixel64: cyw43 up — starting BLE peripheral");

    run_ble(bt_device).await
}

/// Bring up the trouble-host stack on cyw43's BT transport and serve the GATT peripheral forever.
async fn run_ble(bt_device: cyw43::bluetooth::BtDriver<'static>) -> ! {
    let controller: ExternalController<_, 1> = ExternalController::new(bt_device);
    let address = Address::random([0xf0, 0x64, 0x1a, 0x05, 0x8f, 0xff]);
    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let stack = trouble_host::new(controller, &mut resources).set_random_address(address);
    let Host {
        mut peripheral,
        mut runner,
        ..
    } = stack.build();

    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: DEVICE_NAME,
        appearance: &appearance::power_device::GENERIC_POWER_DEVICE,
    }))
    .unwrap();
    let _ = server.set(&server.battery.level, &100u8);

    match select(runner.run(), advertise_loop(&mut peripheral, &server)).await {
        Either::First(_) => panic!("pixel64: BLE runner stopped"),
        Either::Second(never) => never,
    }
}

/// Advertise connectably, accept a client, log GATT activity until it disconnects, then re-advertise.
async fn advertise_loop(
    peripheral: &mut Peripheral<'_, ExternalController<cyw43::bluetooth::BtDriver<'static>, 1>, DefaultPacketPool>,
    server: &Server<'_>,
) -> ! {
    let mut adv_data = [0u8; 31];
    let adv_len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::CompleteLocalName(DEVICE_NAME.as_bytes()),
        ],
        &mut adv_data[..],
    )
    .unwrap();
    let params = AdvertisementParameters::default();

    loop {
        info!("pixel64: advertising as '{}'", DEVICE_NAME);
        let advertiser = peripheral
            .advertise(
                &params,
                Advertisement::ConnectableScannableUndirected {
                    adv_data: &adv_data[..adv_len],
                    scan_data: &[],
                },
            )
            .await
            .unwrap();
        let conn = match advertiser.accept().await {
            Ok(c) => match c.with_attribute_server(server) {
                Ok(conn) => conn,
                Err(e) => {
                    warn!("pixel64: attribute server error: {:?}", e);
                    continue;
                }
            },
            Err(e) => {
                warn!("pixel64: accept error: {:?}", e);
                continue;
            }
        };
        info!("pixel64: BLE client connected");
        loop {
            match conn.next().await {
                GattConnectionEvent::Disconnected { reason } => {
                    info!("pixel64: BLE client disconnected: {:?}", reason);
                    break;
                }
                GattConnectionEvent::Gatt { event } => {
                    // Acknowledge reads/writes so the client sees a working GATT server.
                    if let Ok(reply) = event.accept() {
                        reply.send().await;
                    }
                }
                _ => {}
            }
        }
    }
}
