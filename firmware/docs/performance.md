# Performance: refresh & draw rates

## Measured (64×64, 4 planes, 20 MHz PARL_IO clock)

| Metric | Value | What it is |
|--------|-------|------------|
| `REFRESH_HZ` | **~520 Hz** | Full-panel DMA refreshes per second (how fast the image is *held* lit) |
| `DRAW_HZ` | **~52–53 Hz** | Animation frames drawn per second (how fast the *content* changes) |

Both numbers are about what the configuration predicts — nothing here indicates a
problem.

## Why refresh ≈ 520 Hz

It's essentially the theoretical ceiling for these settings:

- One frame = 1 / 520 ≈ **1.92 ms**, spread across 32 row-addresses (1/32 scan)
  → **~60 µs per row**.
- Each row displays 4 BCM planes weighted **1 : 2 : 4 : 8 = 15 time-units**
  → ~4 µs per unit.
- Shifting 64 pixels at 20 MHz = 64 / 20 MHz = **3.2 µs**. The base unit is bound by
  that shift time plus latch/address/OE overhead ≈ 4 µs. Matches the measurement.

For context: **>~200 Hz is flicker-free to the eye**, so 520 Hz has comfortable
margin. The only place you'd notice it is **filming the panel** — below ~1 kHz a phone
camera may catch faint rolling bands. To the eye it's rock-solid.

## Why draw ≈ 52–53 Hz

`draw_task` is throttled by `Timer::after(Duration::from_millis(16))` → a 62.5 Hz
ceiling. The gap down to ~52 Hz is two unavoidable costs:

- Draw work itself: ~1–2 ms for clear + circle + text.
- Buffer-swap latency: `refresh_task` only notices a freshly-published buffer once per
  refresh cycle (~1.9 ms), so each frame waits up to that long for the hand-off.

16 + ~2 + ~2 ≈ 20 ms → ~52 Hz. To let the animation run as fast as it can, reduce or
remove the throttle (`Duration::from_millis(0)`); `DRAW_HZ` then climbs toward the
refresh rate.

## Tuning knobs

All in `src/bin/main.rs`.

| Change | Effect | Trade-off |
|--------|--------|-----------|
| `Rate::from_mhz(20)` → `30` / `40` | ~linear refresh gain (≈780 Hz at 30 MHz) | Eventually the panel's shift registers / wiring glitch — especially **without level shifters**. Push until corruption appears, then back off. |
| `PLANES: 4` → `3` | ~1.6× faster refresh (8 weight-units vs 15) | Coarser color (3 bits/channel) |
| `PLANES: 4` → `5` | Richer color | ~2× slower refresh (toward ~270 Hz) |
| Draw throttle `16 ms` → lower | Faster animation | More CPU spent redrawing; ball moves faster |

Rough rule of thumb: **refresh ∝ clock_rate / (2^PLANES − 1)** for the BCM weighting,
times the fixed per-row shift/overhead.
