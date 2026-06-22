# Performance: refresh & draw rates

On the Pico the panel refresh is **driven entirely by PIO + DMA** — the CPU does zero work clocking
pixels (unlike the ESP build, where the CPU managed the gaps between DMA transfers and the radio stole
those cycles → flicker). So refresh is **hardware-timed and rock-steady**, decoupled from everything
the CPU is doing.

> The numbers below are the design math / reference figures. They haven't been bench-measured on this
> build yet (no probe + the refresh is autonomous), but the panel is visibly **flicker-free** and the
> reference RP2350 driver this is ported from measures ~**450 FPS** at full color on a 64×64
> ([dgrantpete/Pi-Pico-Hub75-Driver](https://github.com/dgrantpete/Pi-Pico-Hub75-Driver)).

## How the refresh time breaks down

One full frame = scan all **32 row-addresses** (1/32 scan); each row displays all **`B` bit-planes**
(BCM). Per plane, per row:

- **Shift** 64 pixels at the data SM's pixel clock = `sys_clk / DATA_DIV / 2`. At 150 MHz with
  `DATA_DIV = 4` that's ~18.75 MHz → 64 px ≈ **3.4 µs**.
- **Display window** = the BCM dwell for that plane, `(2^i − 1)` OE ticks at `sys_clk / OE_DIV`.

Each plane takes `max(shift_time, OE_window)`. So frame time ≈
`32 rows × Σ_planes max(shift, OE_window)` + latch/address overhead. With small dwell windows the
shift time dominates; raising `OE_DIV` lengthens the high-plane windows (better color, slower frame).

For context: **>~200 Hz is flicker-free to the eye**, and this clears that with large margin. The
only place you'd see structure is filming the panel with a fast shutter.

## Color depth (BCM) and the LSB floor

Color comes from BCM: plane `i` is shown `2^i − 1` ticks (1 : 2 : 4 : … weighting). The catch: a
plane can't be displayed for *less* than the shift time, so the **low planes are floored** at the
shift time and don't get their proportionally-tiny windows. With the current dividers the bright end
saturates — gradients ramp only near the dark end (observed during bring-up). Fixes, when smooth
gradients matter:

- **Raise `OE_DIV`** so the high-plane windows exceed the shift time → the weighting becomes
  effective across more of the range.
- **Add a gamma LUT** in `set_pixel` (LEDs are perceptually non-linear) so a linear input looks
  linear to the eye.

Solid-color content (status text, fixed-color UI) is unaffected — it doesn't use intermediate levels.

## Draw rate

`display::draw_task` re-renders the current `Screen` and `commit()`s on a `Timer::after(33 ms)` →
~**30 fps** content updates. That's plenty for status screens; the *refresh* (what keeps the panel
lit) is independent and runs at the hardware rate above.

## Tuning knobs

All in `src/hub75.rs`.

| Change | Effect | Trade-off |
|--------|--------|-----------|
| `B` (bit-planes) `8` → fewer | faster refresh, less RAM | coarser color |
| `B` → more | richer color | slower refresh; 2 × `64·64/2·B` bytes of RAM |
| `DATA_DIV` `4` → lower | faster pixel clock → faster refresh | eventually the panel/level-shifter glitch — back off if corruption appears |
| `OE_DIV` `1` → higher | effective BCM weighting (better gradients/brightness range) | longer per-plane windows → slower refresh |
| draw-task `33 ms` → lower | faster content updates | more CPU on rendering |

Rough rule of thumb: **refresh ∝ 1 / (rows × planes × per-plane time)**; per-plane time is the larger
of the pixel-shift time (set by `DATA_DIV`) and the BCM window (set by `OE_DIV`).
