//! HUB75 first-light — bit-banged wiring test (Stage A of M3, throwaway/diagnostic).
//!
//! Drives the 14 HUB75 signals as plain GPIOs from a CPU scan loop (no PIO/DMA) to verify the
//! wiring + level shifter before building the real PIO driver. Cycles static test patterns every
//! few seconds; the panel will be dim/slightly flickery (CPU bit-bang) — that's expected here, the
//! point is *correctness of geometry and color*, not refresh rate.
//!
//! Pins per docs/hub75-pico-wiring.md: R1=GP0 G1=GP1 B1=GP2 R2=GP3 G2=GP4 B2=GP5, CLK=GP6 LAT=GP7
//! OE=GP8, A=GP9 B=GP10 C=GP11 D=GP12 E=GP13.
//!
//! Run: `cargo run --bin firstlight`  (hold BOOTSEL). Watch the panel; serial narrates the pattern.

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_futures::yield_now;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::USB;
use embassy_rp::usb::{Driver, InterruptHandler as UsbInterruptHandler};
use embassy_time::{Duration, Instant};
use log::info;

#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [embassy_rp::binary_info::EntryAddr; 2] = [
    embassy_rp::binary_info::rp_program_name!(c"pixel64-firstlight"),
    embassy_rp::binary_info::rp_program_build_attribute!(),
];

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => UsbInterruptHandler<USB>;
});

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[embassy_executor::task]
async fn logger_task(driver: Driver<'static, USB>) {
    embassy_usb_logger::run!(1024, log::LevelFilter::Info, driver);
}

const COLS: usize = 64;
const NADDR: usize = 32; // 1/32 scan: address selects row `a` (top) and `a+32` (bottom)
const NUM_PATTERNS: u32 = 6;

#[inline]
fn lvl(on: bool) -> Level {
    if on { Level::High } else { Level::Low }
}

fn pattern_name(p: u32) -> &'static str {
    match p {
        0 => "solid RED",
        1 => "solid GREEN",
        2 => "solid BLUE",
        3 => "top half RED / bottom half BLUE",
        4 => "8-row color bands (R,G,B,W ×2)",
        5 => "corner markers (TL=R TR=G BL=B BR=W)",
        _ => "?",
    }
}

/// Color for a logical pixel at (col, row) where row is 0..64. Returns (r, g, b) as on/off.
fn pixel(pattern: u32, col: usize, row: usize) -> (bool, bool, bool) {
    match pattern {
        0 => (true, false, false),
        1 => (false, true, false),
        2 => (false, false, true),
        3 => {
            if row < 32 {
                (true, false, false)
            } else {
                (false, false, true)
            }
        }
        4 => match (row / 8) % 4 {
            0 => (true, false, false),
            1 => (false, true, false),
            2 => (false, false, true),
            _ => (true, true, true),
        },
        5 => {
            let tl = col == 0 && row == 0;
            let tr = col == COLS - 1 && row == 0;
            let bl = col == 0 && row == 63;
            let br = col == COLS - 1 && row == 63;
            if tl {
                (true, false, false)
            } else if tr {
                (false, true, false)
            } else if bl {
                (false, false, true)
            } else if br {
                (true, true, true)
            } else {
                (false, false, false)
            }
        }
        _ => (false, false, false),
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let driver = Driver::new(p.USB, Irqs);
    spawner.spawn(logger_task(driver).unwrap());

    // HUB75 pins (see docs/hub75-pico-wiring.md).
    let mut r1 = Output::new(p.PIN_0, Level::Low);
    let mut g1 = Output::new(p.PIN_1, Level::Low);
    let mut b1 = Output::new(p.PIN_2, Level::Low);
    let mut r2 = Output::new(p.PIN_3, Level::Low);
    let mut g2 = Output::new(p.PIN_4, Level::Low);
    let mut b2 = Output::new(p.PIN_5, Level::Low);
    let mut clk = Output::new(p.PIN_6, Level::Low);
    let mut lat = Output::new(p.PIN_7, Level::Low);
    let mut oe = Output::new(p.PIN_8, Level::High); // active-low; start blanked
    let mut addr_a = Output::new(p.PIN_9, Level::Low);
    let mut addr_b = Output::new(p.PIN_10, Level::Low);
    let mut addr_c = Output::new(p.PIN_11, Level::Low);
    let mut addr_d = Output::new(p.PIN_12, Level::Low);
    let mut addr_e = Output::new(p.PIN_13, Level::Low);

    info!("firstlight: HUB75 bit-bang wiring test");
    info!("firstlight: pattern 0 = {}", pattern_name(0));

    let mut pattern = 0u32;
    let mut since = Instant::now();

    loop {
        // Scan one frame. The currently-latched row stays displayed (OE low) while we shift the
        // next row's data into the shift register (the latched output is unaffected by shifting),
        // so the dwell ≈ the shift time and there's no smear.
        for a in 0..NADDR {
            for col in 0..COLS {
                let (tr, tg, tb) = pixel(pattern, col, a); // top half (rows 0..31)
                let (br, bg, bb) = pixel(pattern, col, a + NADDR); // bottom half (rows 32..63)
                r1.set_level(lvl(tr));
                g1.set_level(lvl(tg));
                b1.set_level(lvl(tb));
                r2.set_level(lvl(br));
                g2.set_level(lvl(bg));
                b2.set_level(lvl(bb));
                clk.set_high();
                clk.set_low();
            }
            oe.set_high(); // blank during latch + address change
            addr_a.set_level(lvl(a & 0b00001 != 0));
            addr_b.set_level(lvl(a & 0b00010 != 0));
            addr_c.set_level(lvl(a & 0b00100 != 0));
            addr_d.set_level(lvl(a & 0b01000 != 0));
            addr_e.set_level(lvl(a & 0b10000 != 0));
            lat.set_high();
            lat.set_low();
            oe.set_low(); // display this row during the next iteration's shift
        }

        // Let the USB logger task make progress between frames.
        yield_now().await;

        if since.elapsed() > Duration::from_millis(2500) {
            since = Instant::now();
            pattern = (pattern + 1) % NUM_PATTERNS;
            info!("firstlight: pattern {} = {}", pattern, pattern_name(pattern));
        }
    }
}
