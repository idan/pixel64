# Firmware

## Crate stack

| Crate | Version | Role |
|-------|---------|------|
| `esp-hal` | `~1.1` | HAL for the ESP32-C6 (`esp32c6`, `unstable`) |
| `esp-rtos` | `0.3` | Embassy executor + time driver |
| `esp-hub75` | `0.11` | HUB75 driver — DMA out via PARL_IO, `embedded-graphics` target |
| `embedded-graphics` | `0.8` | Drawing primitives, fonts, text |
| `embassy-sync` | `0.8` | `Signal` for the framebuffer hand-off |
| `heapless` | `0.9` | `String` for on-panel text formatting |

`esp-hub75` 0.11 targets esp-hal 1.1.0 exactly, so it drops onto the generated
boilerplate without version juggling.

### Why PARL_IO

The ESP32-C6 has a **Parallel IO (PARL_IO)** peripheral that streams parallel data
out with DMA. `esp-hub75` uses it to clock HUB75 pixel data with minimal CPU
involvement — the alternative (bit-banging GPIO) is far too slow for a flicker-free
64×64 panel. (On the ESP32/-S3, the same crate uses I2S or LCD_CAM instead.)

## Render architecture (double-buffered)

The panel holds no image, so something must clock pixels out *continuously*. We split
that from the drawing work using two framebuffers that ping-pong between two tasks
(see `src/bin/main.rs`):

```
            ┌───────────────┐   TO_REFRESH (Signal)    ┌────────────────┐
            │   draw_task    │ ───────────────────────► │  refresh_task   │
   draws →  │  (animation,   │                          │  (DMA stream to │ → panel
            │   ~60 fps)     │ ◄─────────────────────── │   the panel)    │
            └───────────────┘    TO_DRAW (Signal)       └────────────────┘
```

- **`refresh_task`** loops forever: `hub75.render(&fb)` → `wait_for_done().await`
  → `wait()` (hands the driver back). Counts completed transfers/sec → `REFRESH_HZ`.
  This is what keeps the panel lit and flicker-free.
- **`draw_task`** clears the back buffer, draws the frame (bouncing ball + live
  counters), publishes it via the `TO_REFRESH` signal, and reclaims the other buffer
  from `TO_DRAW`. Throttled to ~60 fps. Counts frames/sec → `DRAW_HZ`.
- The two `embassy_sync::Signal`s carry `&'static mut FrameBuffer` ownership back and
  forth, so a buffer is never half-drawn while it's being clocked out.

### Framebuffer type

```rust
const ROWS:   usize = 64;
const COLS:   usize = 64;
const NROWS:  usize = compute_rows(ROWS); // 32 (1/32 scan)
const PLANES: usize = 4;                  // BCM color-depth planes
type FrameBuffer = DmaFrameBuffer<NROWS, COLS, PLANES>;
```

`DmaFrameBuffer` (the `framebuffer::bitplane::plain` variant) implements
`embedded_graphics::DrawTarget`, so anything from the embedded-graphics ecosystem
(shapes, fonts, BMPs) draws straight into it. `PLANES` trades color depth against
refresh rate — see [performance.md](performance.md).

### Single executor (not the interrupt executor)

The `esp-hub75` example runs `refresh_task` on a high-priority **interrupt executor**.
We don't, because under esp-rtos 0.3 that executor's `SendSpawner` requires `Send`,
and the driver is built on `esp_hal::Async`, which is **`!Send`** (it holds a
`PhantomData<*const ()>`). Both tasks therefore run cooperatively on the thread-mode
executor.

This is fine for the current workload — `refresh_task` yields during every DMA
transfer, leaving ample time for the light, throttled `draw_task`. If heavier drawing
ever starts to dent the refresh rate, the fix is to keep the `!Send` driver on the
thread-mode executor while moving *other* work to an interrupt executor — not to move
the driver.

## Build & flash

The repo is preconfigured (`.cargo/config.toml` sets the RISC-V target, `build-std`,
and an `espflash` runner):

```sh
cargo run        # builds, flashes over USB, and opens the serial monitor
cargo build      # build only
```

Serial output (via `esp-println`, `ESP_LOG=info`) logs the refresh and draw rates
once per second. The same two numbers render on the panel as `R#### D##`.

## First-light test pattern

Before the animation, a static test pattern is the quickest wiring check:

- **1px white border** → confirms the full 64×64 extent and every edge.
- **R / G / B bars** → confirms color channels aren't swapped.
- **Centered text** → confirms orientation and that the two 32-row halves align.

A **vertical split / duplicated image** points to an address-line (A–E) wiring error —
most often the pin-12 "GND"/D issue described in [hardware-wiring.md](hardware-wiring.md).
