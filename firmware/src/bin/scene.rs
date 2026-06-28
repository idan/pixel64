//! Scene VM demo — runs the shared renderer (`pixel64-renderer`) on the RP2350 and animates the
//! panel with an embedded scene. Proves the shader-bytecode VM executes on-device and drives the
//! HUB75 driver; logs the measured per-frame render time so we know the on-device compute budget.
//!
//! Run: `cargo run --bin scene` (hold BOOTSEL). Logs over USB-serial (`tio /dev/cu.usbmodem*`).

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::{PIO1, USB};
use embassy_rp::pio::{InterruptHandler as PioInterruptHandler, Pio};
use embassy_rp::usb::{Driver, InterruptHandler as UsbInterruptHandler};
use embassy_time::{Duration, Instant, Timer};
use log::info;
use static_cell::StaticCell;

use embedded_graphics::pixelcolor::Rgb888;

use pixel64::hub75::{self, Display, DisplayMemory, Hub75Dma, Hub75Pins};
use pixel64::scene;

/// Shader eval resolution (≤ 64, must divide 64). 64 = full panel; 32 = ¼ the per-pixel VM work,
/// nearest-upscaled. Lower this to trade spatial detail for fps. (The VM dominates frame time, so
/// this is the primary fps knob — measured ~88% of the budget at 64×64.)
const EVAL_RES: usize = 32;

#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [embassy_rp::binary_info::EntryAddr; 2] = [
    embassy_rp::binary_info::rp_program_name!(c"pixel64-scene"),
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

    // One-time blit cost: time a full-panel set_pixel sweep with no VM, so the VM cost is
    // (per-frame render) − (this). Folded into the periodic log below since boot-time lines are lost
    // before tio connects.
    let t_blit = Instant::now();
    for y in 0..hub75::H {
        for x in 0..hub75::W {
            display.set_pixel(x, y, Rgb888::new(128, 128, 128));
        }
    }
    let blit_us = t_blit.elapsed().as_micros();

    let start = Instant::now();
    let mut frame: u32 = 0;
    let mut render_us_accum: u64 = 0;
    loop {
        // Real-time clock so animation speed is independent of render/frame rate.
        let t = start.elapsed().as_micros() as f32 / 1_000_000.0;

        let t0 = Instant::now();
        scene::render(&mut display, &scene::DEMO, t, frame, EVAL_RES);
        render_us_accum += t0.elapsed().as_micros();
        display.commit();

        frame = frame.wrapping_add(1);
        if frame.is_multiple_of(30) {
            let avg = render_us_accum / 30;
            render_us_accum = 0;
            // Render-only fps ceiling; split into VM (= render − blit) vs the BCM set_pixel blit.
            info!(
                "scene: eval_res {} — render {} us (~{} fps) | vm {} us, blit {} us",
                EVAL_RES,
                avg,
                1_000_000u64.checked_div(avg).unwrap_or(0),
                avg.saturating_sub(blit_us),
                blit_us,
            );
        }

        // Yield so the USB logger task can run (a busy loop would starve it — see docs/gotchas.md).
        Timer::after(Duration::from_millis(5)).await;
    }
}
