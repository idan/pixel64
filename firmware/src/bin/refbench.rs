//! Refresh-rate benchmark.
//!
//! The panel refresh is autonomous (PIO + DMA, no CPU), so we measure it by watching the framebuffer
//! DMA: its read address ramps across the active buffer once per refreshed frame and resets at each
//! frame boundary. `Display::fb_read_addr` exposes that address read-only; we busy-poll it, count the
//! wrap-arounds (a drop in the value) over a fixed wall-clock window, and report frames/sec. This
//! measures whatever `B` / `DATA_DIV` / `OE_DIV` are compiled in (see src/hub75.rs).
//!
//! A static mid-gray field is shown so you can also eyeball brightness / flicker while it runs.
//!
//! Run: `cargo run --bin refbench` (hold BOOTSEL), then `screen /dev/tty.usbmodem*` for the numbers.

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::{PIO1, USB};
use embassy_rp::pio::{InterruptHandler as PioInterruptHandler, Pio};
use embassy_rp::usb::{Driver, InterruptHandler as UsbInterruptHandler};
use embassy_time::{Duration, Instant, Timer};
use embedded_graphics::pixelcolor::Rgb888;
use log::info;
use static_cell::StaticCell;

use pixel64::hub75::{self, Display, DisplayMemory, Hub75Dma, Hub75Pins};

#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [embassy_rp::binary_info::EntryAddr; 2] = [
    embassy_rp::binary_info::rp_program_name!(c"pixel64-refbench"),
    embassy_rp::binary_info::rp_program_build_attribute!(),
];

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => UsbInterruptHandler<USB>;
    PIO1_IRQ_0 => PioInterruptHandler<PIO1>;
});

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[embassy_executor::task]
async fn logger_task(driver: Driver<'static, USB>) {
    embassy_usb_logger::run!(1024, log::LevelFilter::Info, driver);
}

/// Busy-poll the framebuffer read address for `window`, counting frame wrap-arounds. Returns the
/// frame count and the *exact* elapsed time, so the caller divides one by the other (the rate is
/// constant, so a short window is plenty and keeps the executor free between windows). Must stay a
/// tight loop with no `.await` inside — yielding mid-count would let the read address wrap unseen.
fn measure(display: &Display, window: Duration) -> (u32, Duration) {
    let mut prev = display.fb_read_addr();
    let mut frames: u32 = 0;
    let start = Instant::now();
    loop {
        let elapsed = start.elapsed();
        if elapsed >= window {
            return (frames, elapsed);
        }
        let cur = display.fb_read_addr();
        if cur < prev {
            frames += 1; // read addr dropped back to the buffer base → one frame elapsed
        }
        prev = cur;
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let driver = Driver::new(p.USB, Irqs);
    spawner.spawn(logger_task(driver).unwrap());

    let pio = Pio::new(p.PIO1, Irqs);
    static MEM: StaticCell<DisplayMemory> = StaticCell::new();
    let mem = MEM.init(DisplayMemory::new());

    let mut display = Display::new(
        mem,
        pio.common,
        pio.sm0,
        pio.sm1,
        pio.sm2,
        Hub75Pins {
            r1: p.PIN_0,
            g1: p.PIN_1,
            b1: p.PIN_2,
            r2: p.PIN_3,
            g2: p.PIN_4,
            b2: p.PIN_5,
            clk: p.PIN_6,
            lat: p.PIN_7,
            oe: p.PIN_8,
            a: p.PIN_9,
            b: p.PIN_10,
            c: p.PIN_11,
            d: p.PIN_12,
            e: p.PIN_13,
        },
        Hub75Dma {
            fb: p.DMA_CH2,
            fb_loop: p.DMA_CH3,
            oe: p.DMA_CH4,
            oe_loop: p.DMA_CH5,
        },
    );

    // Static mid-gray field — visible brightness/flicker reference while measuring.
    for y in 0..hub75::H {
        for x in 0..hub75::W {
            display.set_pixel(x, y, Rgb888::new(128, 128, 128));
        }
    }
    display.commit();

    info!(
        "refbench: B={} DATA_DIV={} OE_DIV={} — measuring...",
        hub75::B,
        hub75::DATA_DIV,
        hub75::OE_DIV,
    );
    // Yield so USB enumerates and this first line flushes before we start busy-measuring.
    Timer::after_secs(2).await;

    let window = Duration::from_millis(500);
    loop {
        // Busy-measure a short window (USB just NAKs during it), then yield so the logger task
        // drains the buffer to the host. Rate = frames / exact elapsed.
        let (frames, elapsed) = measure(&display, window);
        let ms = elapsed.as_millis().max(1);
        let hz_x10 = (frames as u64 * 10_000 / ms) as u32; // frames per second, ×10 for one decimal
        info!(
            "refbench: {} frames in {} ms -> {}.{} Hz",
            frames,
            ms,
            hz_x10 / 10,
            hz_x10 % 10,
        );
        Timer::after_millis(200).await;
    }
}
