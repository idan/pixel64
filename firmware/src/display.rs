//! Status-screen renderer over the HUB75 PIO+DMA driver.
//!
//! Ported from the ESP build, but simpler: the PIO+DMA loop refreshes the panel in hardware
//! (`hub75`), so there's no refresh task — just a draw task that renders the current [`Screen`] into
//! the inactive buffer and `commit()`s. The orchestrator changes screens with [`set_screen`].

use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use embedded_graphics::{
    mono_font::{MonoTextStyle, MonoTextStyleBuilder, ascii::FONT_6X10},
    pixelcolor::Rgb888,
    prelude::*,
    text::{Alignment, Text},
};
use heapless::String;

use crate::hub75::{self, Display};

const COLS: usize = hub75::W;

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

/// Spawn the draw task for an already-initialized [`Display`]. Refresh is handled in hardware.
pub fn start(display: Display, spawner: Spawner) {
    spawner.spawn(draw_task(display).unwrap());
}

/// Render the current screen into the inactive buffer and flip it; ~30 fps. `commit()` zeroes the
/// next inactive buffer, so each frame starts blank without an explicit clear.
#[embassy_executor::task]
async fn draw_task(mut display: Display) {
    let mut screen = Screen::Booting;
    loop {
        if SCREEN.signaled() {
            screen = SCREEN.wait().await;
        }
        render(&mut display, &screen);
        display.commit();
        Timer::after(Duration::from_millis(33)).await;
    }
}

fn style(color: Rgb888) -> MonoTextStyle<'static, Rgb888> {
    MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(color)
        .build()
}

/// Draw one horizontally-centered line of text with its baseline at `y`.
fn line(d: &mut Display, text: &str, y: i32, color: Rgb888) {
    // FONT_6X10 is 6 px wide; truncate to what fits across 64 px so it stays on-panel.
    let max = COLS / 6;
    let text = if text.len() > max { &text[..max] } else { text };
    let _ = Text::with_alignment(
        text,
        Point::new(COLS as i32 / 2, y),
        style(color),
        Alignment::Center,
    )
    .draw(d);
}

fn render(d: &mut Display, screen: &Screen) {
    match screen {
        Screen::Booting => line(d, "pixel64", 36, Rgb888::YELLOW),
        Screen::Connecting(ssid) => {
            line(d, "Wi-Fi", 18, Rgb888::CYAN);
            line(d, "joining", 32, Rgb888::WHITE);
            line(d, ssid, 46, Rgb888::WHITE);
        }
        Screen::Setup => {
            line(d, "SETUP", 14, Rgb888::YELLOW);
            line(d, "Chrome:", 30, Rgb888::WHITE);
            line(d, "improv", 42, Rgb888::CYAN);
            line(d, "pixel64", 56, Rgb888::CYAN);
        }
        Screen::Online(ip) => {
            line(d, "ONLINE", 22, Rgb888::GREEN);
            // Render the dotted-quad IP across two lines so it fits.
            let o = ip.octets();
            let mut top: String<16> = String::new();
            let mut bot: String<16> = String::new();
            let _ = core::fmt::write(&mut top, format_args!("{}.{}.", o[0], o[1]));
            let _ = core::fmt::write(&mut bot, format_args!("{}.{}", o[2], o[3]));
            line(d, &top, 40, Rgb888::WHITE);
            line(d, &bot, 52, Rgb888::WHITE);
        }
        Screen::Failed(why) => {
            line(d, "FAILED", 26, Rgb888::RED);
            line(d, why, 42, Rgb888::WHITE);
        }
    }
}
