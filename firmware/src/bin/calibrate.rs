//! BCM / gamma calibration target.
//!
//! Draws a static pattern designed to dial in the perceptual gamma LUT (`hub75::GAMMA`) and the BCM
//! dividers (`OE_DIV`/`DATA_DIV`) so gradients show **even luminance bands**. The whole loop is
//! eyeball-driven (there's no probe and refresh is autonomous):
//!
//!   1. `cargo run --bin calibrate` (hold BOOTSEL), look at the panel.
//!   2. Edit `GAMMA` in `src/hub75.rs` (start 2.2) and reflash; repeat until the bands read evenly.
//!   3. If the dark end stays crushed / bands collapse to black, raise `B` for headroom; if tiny
//!      OE windows look non-linear at the low end, raise `OE_DIV`. See docs/performance.md.
//!
//! Layout (64×64), top→bottom: 4×8 rows of smooth gray/R/G/B ramps (0→255 left→right, for overall
//! feel + per-channel tint); 8 rows of 16 stepped gray bands (the primary "are the steps
//! perceptually even?" check); 8 rows of 8 coarser stepped bands; then a 16-row gamma-match target.
//!
//! The match target's left half is a 1px black/white checkerboard (= 50% *linear* light); its right
//! half is a solid patch whose value is `255·0.5^(1/GAMMA)`, computed so that **when GAMMA matches
//! the panel's true response** it emits the same 50% and the seam vanishes. A visible seam is the
//! error signal: right-half too dark → raise GAMMA, too bright → lower it.
//!
//! Run: `cargo run --bin calibrate` (hold BOOTSEL).

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::{PIO1, USB};
use embassy_rp::pio::{InterruptHandler as PioInterruptHandler, Pio};
use embassy_rp::usb::{Driver, InterruptHandler as UsbInterruptHandler};
use embassy_time::Timer;
use embedded_graphics::pixelcolor::Rgb888;
use log::info;
use static_cell::StaticCell;

use pixel64::hub75::{self, Display, DisplayMemory, Hub75Dma, Hub75Pins, GAMMA};

#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [embassy_rp::binary_info::EntryAddr; 2] = [
    embassy_rp::binary_info::rp_program_name!(c"pixel64-calibrate"),
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

const W: usize = hub75::W;
const H: usize = hub75::H;

fn fill_row_band(d: &mut Display, y0: usize, rows: usize, mut px: impl FnMut(usize) -> Rgb888) {
    for y in y0..y0 + rows {
        for x in 0..W {
            d.set_pixel(x, y, px(x));
        }
    }
}

fn draw_calibration(d: &mut Display) {
    // --- Smooth ramps: gray, R, G, B (8 rows each) ---
    let ramp = |x: usize| (x * 255 / (W - 1)) as u8;
    fill_row_band(d, 0, 8, |x| {
        let v = ramp(x);
        Rgb888::new(v, v, v)
    });
    fill_row_band(d, 8, 8, |x| Rgb888::new(ramp(x), 0, 0));
    fill_row_band(d, 16, 8, |x| Rgb888::new(0, ramp(x), 0));
    fill_row_band(d, 24, 8, |x| Rgb888::new(0, 0, ramp(x)));

    // --- Stepped gray bands: 16 then 8 steps. Equal code steps should read as equal perceived
    // steps once GAMMA is right (without gamma they bunch toward the dark end). ---
    let step = |x: usize, n: usize| {
        let k = x * n / W; // 0..n-1
        (k * 255 / (n - 1)) as u8
    };
    fill_row_band(d, 32, 8, |x| {
        let v = step(x, 16);
        Rgb888::new(v, v, v)
    });
    fill_row_band(d, 40, 8, |x| {
        let v = step(x, 8);
        Rgb888::new(v, v, v)
    });

    // --- Gamma-match target (16 rows). Left half: 1px checkerboard = 50% linear light. Right half:
    // solid `match_v` chosen so lut[match_v] ≈ 50% of max under GAMMA, i.e. v = 255·0.5^(1/GAMMA).
    // Seam invisible ⇒ GAMMA matches the panel. ---
    let match_v = (255.0 * libm::powf(0.5, 1.0 / GAMMA) + 0.5) as u8;
    let solid = Rgb888::new(match_v, match_v, match_v);
    for y in 48..H {
        for x in 0..W {
            let c = if x < W / 2 {
                // checkerboard: full on/off
                if (x + y) & 1 == 0 {
                    Rgb888::new(255, 255, 255)
                } else {
                    Rgb888::new(0, 0, 0)
                }
            } else {
                solid
            };
            d.set_pixel(x, y, c);
        }
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let driver = Driver::new(p.USB, Irqs);
    spawner.spawn(logger_task(driver).unwrap());

    let match_v = (255.0 * libm::powf(0.5, 1.0 / GAMMA) + 0.5) as u8;
    info!(
        "calibrate: GAMMA={}, B={}, checkerboard match value={}",
        GAMMA,
        hub75::B,
        match_v
    );

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

    draw_calibration(&mut display);
    display.commit();
    info!("calibrate: pattern committed");

    loop {
        Timer::after_secs(5).await;
    }
}
