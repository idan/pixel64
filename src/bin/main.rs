#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use embassy_executor::Spawner;
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyleBuilder},
    pixelcolor::RgbColor,
    prelude::*,
    primitives::{PrimitiveStyleBuilder, Rectangle},
    text::{Alignment, Text},
};
use esp_hal::clock::CpuClock;
use esp_hal::gpio::Pin;
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hub75::{
    framebuffer::{bitplane::plain::DmaFrameBuffer, compute_rows},
    Color, Hub75, Hub75Pins16,
};
use log::{error, info};

#[panic_handler]
fn panic(panic_info: &core::panic::PanicInfo) -> ! {
    error!("{}", panic_info);
    loop {}
}

// This creates a default app-descriptor required by the esp-idf bootloader.
esp_bootloader_esp_idf::esp_app_desc!();

// 64x64 panel. PLANES = color-depth bits per channel that the PARL_IO path can
// fit in one DMA buffer (4 is the crate's recommended value for this peripheral).
const ROWS: usize = 64;
const COLS: usize = 64;
const NROWS: usize = compute_rows(ROWS);
const PLANES: usize = 4;
type FrameBuffer = DmaFrameBuffer<NROWS, COLS, PLANES>;

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
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    info!("Embassy initialized!");

    // --- HUB75 pin map (matches the wiring table; tested by the esp-hub75 author) ---
    let pins = Hub75Pins16 {
        red1: peripherals.GPIO19.degrade(),
        grn1: peripherals.GPIO20.degrade(),
        blu1: peripherals.GPIO21.degrade(),
        red2: peripherals.GPIO22.degrade(),
        grn2: peripherals.GPIO23.degrade(),
        blu2: peripherals.GPIO15.degrade(),
        addr0: peripherals.GPIO2.degrade(), // A
        addr1: peripherals.GPIO8.degrade(), // B  (strapping pin — fine as plain output)
        addr2: peripherals.GPIO1.degrade(), // C
        addr3: peripherals.GPIO0.degrade(), // D
        addr4: peripherals.GPIO3.degrade(), // E
        blank: peripherals.GPIO5.degrade(),  // OE
        clock: peripherals.GPIO7.degrade(),  // CLK
        latch: peripherals.GPIO6.degrade(),  // LAT / STB
    };

    let tx_descriptors = esp_hub75::hub75_dma_descriptors!(FrameBuffer);
    let mut hub75 = Hub75::new_async(
        peripherals.PARL_IO,
        pins,
        peripherals.DMA_CH0,
        tx_descriptors,
        Rate::from_mhz(20),
    )
    .expect("failed to create Hub75 driver");

    // --- Draw a test pattern into the framebuffer with embedded-graphics ---
    let mut fb = FrameBuffer::new();
    draw_test_pattern(&mut fb);

    info!("Rendering test pattern. The panel has no memory, so we refresh continuously.");

    // The panel holds no image of its own: it only lights up while data is being
    // clocked out. So we render the framebuffer over and over, forever.
    loop {
        let xfer = hub75
            .render(&fb)
            .map_err(|(e, _hub75)| e)
            .expect("failed to start render");
        let (result, returned) = xfer.wait();
        hub75 = returned;
        result.expect("DMA transfer failed");

        let _ = spawner;
    }
}

/// A simple, unambiguous first-light pattern:
/// - a 1px white border (confirms all 64x64 pixels and edges)
/// - R / G / B horizontal bars (confirms color channels aren't swapped)
/// - centered text (confirms orientation / addressing)
fn draw_test_pattern(fb: &mut FrameBuffer) {
    fb.clear(Color::BLACK).unwrap();

    let border = PrimitiveStyleBuilder::new()
        .stroke_color(Color::WHITE)
        .stroke_width(1)
        .build();
    Rectangle::new(Point::new(0, 0), Size::new(COLS as u32, ROWS as u32))
        .into_styled(border)
        .draw(fb)
        .unwrap();

    let bar = |fb: &mut FrameBuffer, y: i32, color: Color| {
        Rectangle::new(Point::new(2, y), Size::new(COLS as u32 - 4, 6))
            .into_styled(PrimitiveStyleBuilder::new().fill_color(color).build())
            .draw(fb)
            .unwrap();
    };
    bar(fb, 4, Color::RED);
    bar(fb, 12, Color::GREEN);
    bar(fb, 20, Color::BLUE);

    let text_style = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(Color::YELLOW)
        .build();
    Text::with_alignment("64x64", Point::new(32, 44), text_style, Alignment::Center)
        .draw(fb)
        .unwrap();
}
