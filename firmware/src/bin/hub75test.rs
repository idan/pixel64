//! HUB75 PIO+DMA driver test (Stage B of M3).
//!
//! Brings up the real `hub75` driver and draws a static full-color test pattern (color bars + a
//! gradient + a 1px border), refreshed by the PIO+DMA loop. Verifies the driver lights the panel
//! with correct geometry, color, and — crucially — **no flicker** (hardware-timed refresh). Tune
//! `hub75::B` / the clock dividers if color/brightness need work.
//!
//! Run: `cargo run --bin hub75test` (hold BOOTSEL).

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::{PIO1, USB};
use embassy_rp::pio::{InterruptHandler as PioInterruptHandler, Pio};
use embassy_rp::usb::{Driver, InterruptHandler as UsbInterruptHandler};
use embassy_time::Timer;
use embedded_graphics::pixelcolor::Rgb888;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use log::info;
use static_cell::StaticCell;

use pixel64::hub75::{self, Display, DisplayMemory, Hub75Dma, Hub75Pins};

#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [embassy_rp::binary_info::EntryAddr; 2] = [
    embassy_rp::binary_info::rp_program_name!(c"pixel64-hub75test"),
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

fn draw_test_pattern(d: &mut Display) {
    // 8 vertical color bars across the top half.
    let bars = [
        Rgb888::BLACK,
        Rgb888::RED,
        Rgb888::GREEN,
        Rgb888::BLUE,
        Rgb888::YELLOW,
        Rgb888::MAGENTA,
        Rgb888::CYAN,
        Rgb888::WHITE,
    ];
    let bw = hub75::W / bars.len();
    for (i, c) in bars.iter().enumerate() {
        Rectangle::new(
            Point::new((i * bw) as i32, 0),
            Size::new(bw as u32, (hub75::H / 2) as u32),
        )
        .into_styled(PrimitiveStyle::with_fill(*c))
        .draw(d)
        .unwrap();
    }
    // Bottom half: horizontal R/G/B brightness gradients (exercises BCM color depth).
    for x in 0..hub75::W {
        let v = (x * 255 / (hub75::W - 1)) as u8;
        for (band, color) in [
            Rgb888::new(v, 0, 0),
            Rgb888::new(0, v, 0),
            Rgb888::new(0, 0, v),
        ]
        .iter()
        .enumerate()
        {
            let y0 = hub75::H / 2 + band * (hub75::H / 2 / 3);
            for y in y0..y0 + (hub75::H / 2 / 3) {
                d.set_pixel(x, y, *color);
            }
        }
    }
    // 1px white border to check edges/origin.
    Rectangle::new(Point::zero(), Size::new(hub75::W as u32, hub75::H as u32))
        .into_styled(PrimitiveStyle::with_stroke(Rgb888::WHITE, 1))
        .draw(d)
        .unwrap();
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let driver = Driver::new(p.USB, Irqs);
    spawner.spawn(logger_task(driver).unwrap());

    info!("hub75test: bringing up PIO+DMA driver");

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

    draw_test_pattern(&mut display);
    display.commit();
    info!("hub75test: test pattern committed — should be lit + stable");

    let mut tick = 0u32;
    loop {
        info!("hub75test: alive — tick {}", tick);
        tick = tick.wrapping_add(1);
        Timer::after_secs(5).await;
    }
}
