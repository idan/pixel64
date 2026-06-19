//! HUB75 display: double-buffered continuous refresh + a status-screen renderer.
//!
//! The refresh task streams the framebuffer to the panel forever (the panel has no memory);
//! the draw task renders whatever [`Screen`] is current. The orchestrator changes screens with
//! [`set_screen`].
//!
//! KEEP IN SYNC: docs/performance.md tabulates the refresh math for ROWS/COLS/PLANES below.

use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle, MonoTextStyleBuilder},
    pixelcolor::RgbColor,
    prelude::*,
    text::{Alignment, Text},
};
use esp_hub75::{
    framebuffer::{bitplane::plain::DmaFrameBuffer, compute_rows},
    Color, Hub75,
};
use heapless::String;
use static_cell::StaticCell;

pub const ROWS: usize = 64;
pub const COLS: usize = 64;
const NROWS: usize = compute_rows(ROWS);
const PLANES: usize = 4;

pub type FrameBuffer = DmaFrameBuffer<NROWS, COLS, PLANES>;
pub type Hub75Driver = Hub75<'static, esp_hal::Async>;

type FbExchange = Signal<CriticalSectionRawMutex, &'static mut FrameBuffer>;

/// What the panel should currently show. Short strings keep within the 64-px width.
#[derive(Clone)]
pub enum Screen {
    Booting,
    Connecting(String<32>),
    Setup,
    Online(core::net::Ipv4Addr),
    Failed(String<24>),
}

static SCREEN: Signal<CriticalSectionRawMutex, Screen> = Signal::new();

/// Update what the panel shows (latest wins).
pub fn set_screen(screen: Screen) {
    SCREEN.signal(screen);
}

/// Spawn the refresh + draw tasks for an already-configured [`Hub75`] driver.
pub fn start(hub75: Hub75Driver, spawner: Spawner) {
    static TO_REFRESH: FbExchange = FbExchange::new();
    static TO_DRAW: FbExchange = FbExchange::new();
    static FB0: StaticCell<FrameBuffer> = StaticCell::new();
    static FB1: StaticCell<FrameBuffer> = StaticCell::new();
    let fb0 = FB0.init(FrameBuffer::new());
    let fb1 = FB1.init(FrameBuffer::new());

    spawner.spawn(refresh_task(hub75, &TO_REFRESH, &TO_DRAW, fb1).unwrap());
    spawner.spawn(draw_task(&TO_REFRESH, &TO_DRAW, fb0).unwrap());
}

/// Continuously stream the current framebuffer to the panel; swap when a fresh one arrives.
#[embassy_executor::task]
async fn refresh_task(
    mut hub75: Hub75Driver,
    incoming: &'static FbExchange,
    outgoing: &'static FbExchange,
    mut fb: &'static mut FrameBuffer,
) {
    loop {
        if incoming.signaled() {
            let next = incoming.wait().await;
            outgoing.signal(fb);
            fb = next;
        }
        let mut xfer = hub75
            .render(&*fb)
            .map_err(|(e, _)| e)
            .expect("failed to start render");
        xfer.wait_for_done().await.expect("DMA transfer failed");
        let (result, returned) = xfer.wait();
        hub75 = returned;
        result.expect("transfer failed");
    }
}

/// Redraw the current screen into the back buffer and hand it to the refresh task.
#[embassy_executor::task]
async fn draw_task(
    outgoing: &'static FbExchange,
    incoming: &'static FbExchange,
    mut fb: &'static mut FrameBuffer,
) {
    let mut screen = Screen::Booting;
    loop {
        if SCREEN.signaled() {
            screen = SCREEN.wait().await;
        }
        fb.clear(Color::BLACK).unwrap();
        render(fb, &screen);
        outgoing.signal(fb);
        fb = incoming.wait().await;
        Timer::after(Duration::from_millis(33)).await; // ~30 fps redraw; refresh runs full-speed
    }
}

fn style(color: Color) -> MonoTextStyle<'static, Color> {
    MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(color)
        .build()
}

/// Draw one horizontally-centered line of text with its baseline at `y`.
fn line(fb: &mut FrameBuffer, text: &str, y: i32, color: Color) {
    // FONT_6X10 is 6 px wide; truncate to what fits across 64 px so it stays on-panel.
    let max = COLS / 6;
    let text = if text.len() > max { &text[..max] } else { text };
    Text::with_alignment(
        text,
        Point::new(COLS as i32 / 2, y),
        style(color),
        Alignment::Center,
    )
    .draw(fb)
    .unwrap();
}

fn render(fb: &mut FrameBuffer, screen: &Screen) {
    match screen {
        Screen::Booting => line(fb, "pixel64", 36, Color::YELLOW),
        Screen::Connecting(ssid) => {
            line(fb, "Wi-Fi", 18, Color::CYAN);
            line(fb, "joining", 32, Color::WHITE);
            line(fb, ssid, 46, Color::WHITE);
        }
        Screen::Setup => {
            line(fb, "SETUP", 14, Color::YELLOW);
            line(fb, "Chrome:", 30, Color::WHITE);
            line(fb, "improv", 42, Color::CYAN);
            line(fb, "pixel64", 56, Color::CYAN);
        }
        Screen::Online(ip) => {
            line(fb, "ONLINE", 22, Color::GREEN);
            // Render the dotted-quad IP across two lines so it fits.
            let o = ip.octets();
            let mut top: String<16> = String::new();
            let mut bot: String<16> = String::new();
            let _ = core::fmt::write(&mut top, format_args!("{}.{}.", o[0], o[1]));
            let _ = core::fmt::write(&mut bot, format_args!("{}.{}", o[2], o[3]));
            line(fb, &top, 40, Color::WHITE);
            line(fb, &bot, 52, Color::WHITE);
        }
        Screen::Failed(why) => {
            line(fb, "FAILED", 26, Color::RED);
            line(fb, why, 42, Color::WHITE);
        }
    }
}
