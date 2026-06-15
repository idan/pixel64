#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use core::fmt::Write as _;
use core::sync::atomic::{AtomicU32, Ordering};

use embassy_executor::{task, Spawner};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer};
use embedded_graphics::{
    mono_font::{ascii::FONT_5X7, MonoTextStyleBuilder},
    pixelcolor::RgbColor,
    prelude::*,
    primitives::{Circle, PrimitiveStyleBuilder},
    text::Text,
};
use esp_hal::clock::CpuClock;
use esp_hal::gpio::Pin;
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hub75::{
    framebuffer::{bitplane::plain::DmaFrameBuffer, compute_rows},
    Color, Hub75, Hub75Pins16,
};
use heapless::String;
use log::info;

#[panic_handler]
fn panic(panic_info: &core::panic::PanicInfo) -> ! {
    log::error!("{}", panic_info);
    loop {}
}

esp_bootloader_esp_idf::esp_app_desc!();

const ROWS: usize = 64;
const COLS: usize = 64;
const NROWS: usize = compute_rows(ROWS);
const PLANES: usize = 4;
type FrameBuffer = DmaFrameBuffer<NROWS, COLS, PLANES>;
type Hub75Driver = Hub75<'static, esp_hal::Async>;

/// Hands a framebuffer back and forth between the draw task and the refresh task.
type FbExchange = Signal<CriticalSectionRawMutex, &'static mut FrameBuffer>;

/// DMA refreshes pushed to the panel per second (how fast the image is *held* lit).
static REFRESH_HZ: AtomicU32 = AtomicU32::new(0);
/// Animation frames drawn per second (how fast the *content* changes).
static DRAW_HZ: AtomicU32 = AtomicU32::new(0);

/// Allocate `$val` in a `'static` cell so it can be handed to a spawned task.
macro_rules! mk_static {
    ($t:ty, $val:expr) => {{
        static CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        CELL.uninit().write($val)
    }};
}

/// High-priority task: do nothing but stream the current framebuffer to the panel
/// over and over. Because the panel holds no image of its own, this is what keeps
/// it lit and flicker-free. When the draw task offers a fresh buffer, swap to it
/// and hand the old one back.
#[task]
async fn refresh_task(
    mut hub75: Hub75Driver,
    incoming: &'static FbExchange,
    outgoing: &'static FbExchange,
    mut fb: &'static mut FrameBuffer,
) {
    info!("refresh_task: started");
    let mut count = 0u32;
    let mut window = Instant::now();

    loop {
        if incoming.signaled() {
            let next = incoming.wait().await;
            outgoing.signal(fb);
            fb = next;
        }

        let mut xfer = hub75
            .render(&*fb)
            .map_err(|(e, _hub75)| e)
            .expect("failed to start render");
        xfer.wait_for_done().await.expect("DMA transfer failed");
        let (result, returned) = xfer.wait();
        hub75 = returned;
        result.expect("transfer failed");

        count += 1;
        if window.elapsed() >= Duration::from_secs(1) {
            REFRESH_HZ.store(count, Ordering::Relaxed);
            count = 0;
            window = Instant::now();
        }
    }
}

/// Normal-priority task: draw the animation into the back buffer, hand it to the
/// refresh task, and take back the other buffer to draw the next frame into.
#[task]
async fn draw_task(
    outgoing: &'static FbExchange,
    incoming: &'static FbExchange,
    mut fb: &'static mut FrameBuffer,
) {
    info!("draw_task: started");
    let text_style = MonoTextStyleBuilder::new()
        .font(&FONT_5X7)
        .text_color(Color::YELLOW)
        .background_color(Color::BLACK)
        .build();

    // Bouncing-ball state.
    const R: i32 = 6;
    let (mut x, mut y) = (R, R);
    let (mut dx, mut dy) = (1i32, 1i32);

    let mut count = 0u32;
    let mut window = Instant::now();

    loop {
        fb.clear(Color::BLACK).unwrap();

        // The ball — colour cycles with the refresh rate so movement is obvious.
        Circle::new(Point::new(x - R, y - R), (2 * R) as u32)
            .into_styled(PrimitiveStyleBuilder::new().fill_color(Color::CYAN).build())
            .draw(fb)
            .unwrap();

        // Live counters: R = DMA refresh Hz, D = animation draw Hz.
        let mut line: String<32> = String::new();
        let _ = write!(
            line,
            "R{:4} D{:3}",
            REFRESH_HZ.load(Ordering::Relaxed),
            DRAW_HZ.load(Ordering::Relaxed)
        );
        Text::new(&line, Point::new(1, 61), text_style)
            .draw(fb)
            .unwrap();

        // Advance and bounce.
        x += dx;
        y += dy;
        if x <= R || x >= COLS as i32 - R {
            dx = -dx;
        }
        if y <= R || y >= ROWS as i32 - R {
            dy = -dy;
        }

        // Publish this frame, reclaim the other buffer for the next one.
        outgoing.signal(fb);
        fb = incoming.wait().await;

        count += 1;
        if window.elapsed() >= Duration::from_secs(1) {
            DRAW_HZ.store(count, Ordering::Relaxed);
            count = 0;
            window = Instant::now();
        }

        // Cap the animation at ~60 fps so the ball moves at a sane speed; the
        // refresh task keeps the panel lit at its full rate in the meantime.
        Timer::after(Duration::from_millis(16)).await;
    }
}

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);
    info!("Embassy initialized!");

    // HUB75 wiring (matches the wiring table; the "GND"-silked pin 12 is D).
    let pins = Hub75Pins16 {
        red1: peripherals.GPIO19.degrade(),
        grn1: peripherals.GPIO20.degrade(),
        blu1: peripherals.GPIO21.degrade(),
        red2: peripherals.GPIO22.degrade(),
        grn2: peripherals.GPIO23.degrade(),
        blu2: peripherals.GPIO15.degrade(),
        addr0: peripherals.GPIO2.degrade(), // A
        addr1: peripherals.GPIO8.degrade(), // B
        addr2: peripherals.GPIO1.degrade(), // C
        addr3: peripherals.GPIO0.degrade(), // D  (pin 12, silk says "GND")
        addr4: peripherals.GPIO3.degrade(), // E
        blank: peripherals.GPIO5.degrade(), // OE
        clock: peripherals.GPIO7.degrade(), // CLK
        latch: peripherals.GPIO6.degrade(), // LAT
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

    // Two framebuffers ping-pong between the draw and refresh tasks.
    static TO_REFRESH: FbExchange = FbExchange::new();
    static TO_DRAW: FbExchange = FbExchange::new();
    let fb0 = mk_static!(FrameBuffer, FrameBuffer::new());
    let fb1 = mk_static!(FrameBuffer, FrameBuffer::new());

    // The HUB75 driver is built on esp_hal::Async, which is !Send, so it can't be
    // handed to an interrupt executor's SendSpawner — both tasks run cooperatively
    // on this thread-mode executor. The refresh task yields during each DMA
    // transfer, leaving ample time for the lightweight, throttled draw task.
    spawner.spawn(refresh_task(hub75, &TO_REFRESH, &TO_DRAW, fb1).unwrap());
    spawner.spawn(draw_task(&TO_REFRESH, &TO_DRAW, fb0).unwrap());

    // Also log the rates to the serial monitor once a second.
    loop {
        Timer::after(Duration::from_secs(1)).await;
        info!(
            "refresh = {} Hz, draw = {} Hz",
            REFRESH_HZ.load(Ordering::Relaxed),
            DRAW_HZ.load(Ordering::Relaxed)
        );
    }
}
