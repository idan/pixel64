# Performance: refresh & draw rates

On the Pico the panel refresh is **driven entirely by PIO + DMA** — the CPU does zero work clocking
pixels (unlike the ESP build, where the CPU managed the gaps between DMA transfers and the radio stole
those cycles → flicker). So refresh is **hardware-timed and rock-steady**, decoupled from everything
the CPU is doing.

> The numbers below are the design math / reference figures. Measure the real rate for the compiled
> config with **`cargo run --bin refbench`** (watches the framebuffer DMA's read address wrap, no
> probe needed). At the dialed-in `B = 10`, `OE_DIV = 8` the model predicts **~440 Hz** — well above
> the flicker floor and consistent with the visibly flicker-free panel. (Not yet hardware-confirmed:
> the dev Mac's USB-serial — behind a dock/hub — enumerates two CDC nodes and won't stream; rerun
> `refbench` over a clean/direct USB path to capture the real number.) For reference, the RP2350
> driver this is ported from measures ~**450 FPS** at full color on a 64×64
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

Color comes from BCM: plane `i` is shown `2^i − 1` ticks (1 : 2 : 4 : … weighting). LEDs emit light
~linearly in that code, but the eye is non-linear: a linear-code ramp reads as crushed at the bright
end with all the visible change bunched into the darks (the "gradients ramp only near the dark end"
symptom observed at bring-up). The dominant fix is perceptual gamma; the BCM dividers are secondary.

- **Perceptual gamma LUT (implemented).** `set_pixel` maps each 8-bit channel through
  `hub75::gamma_lut` (`out = (in/255)^GAMMA · (2^B−1)`) before bit-plane extraction. This
  redistributes the steps so equal input steps read as ~equal *perceived* brightness. Endpoints are
  identity (`0→0`, `255→max`), so solid-color content (status text, fixed-color UI) is unaffected.
- **Dark-end headroom.** The gamma curve crushes low inputs toward `0`, so deep gradients need code
  headroom below the perceptual range. More planes buys that headroom — cheap on RAM
  (`2·64·64/2·B` bytes) and refresh stays well above flicker.
- **`OE_DIV`.** Tiny low-plane windows (a few `sys_clk` ticks) can fall below the LED driver's linear
  pulse-response, so the smallest steps emit less than their share. Raising `OE_DIV` stretches every
  window proportionally, lifting the small ones into the linear region (at the cost of refresh rate).

**Dialed-in values (perceptually even on the panel):** `GAMMA = 2.2`, `B = 10`, `OE_DIV = 8`,
`DATA_DIV = 4`. Worth understanding *why* gamma sits at the textbook 2.2: at `OE_DIV = 1` the
low-plane windows were a few nanoseconds — below the LED driver's linear pulse-response — so the darks
read too dark and needed `GAMMA = 1.8` to compensate. Raising `OE_DIV` to 8 stretched those windows
into the driver's linear region, the panel started behaving linearly, and gamma converged back to the
sRGB-standard 2.2. `OE_DIV = 8` also lifts full-white duty from ~20% to ~75% (much brighter); the
practical ceiling here is **panel current draw at high duty**, not refresh rate.

### Calibrating

`cargo run --bin calibrate` draws gray/R/G/B ramps, stepped gray bands, and a **checkerboard-vs-solid
gamma-match** patch. The checkerboard is 50% *linear* light; the solid patch is `255·0.5^(1/GAMMA)`,
so the seam between them vanishes exactly when `GAMMA` matches the panel. Edit `GAMMA` in
`src/hub75.rs`, reflash, and converge: seam's right half too dark → raise `GAMMA`, too bright → lower.
Then check the stepped bands read evenly; if the dark end is crushed, bump `B`.

## Draw rate

`display::draw_task` re-renders the current `Screen` and `commit()`s on a `Timer::after(33 ms)` →
~**30 fps** content updates. That's plenty for status screens; the *refresh* (what keeps the panel
lit) is independent and runs at the hardware rate above.

## Scene VM (on-device shader rendering)

`cargo run --release --bin scene` runs the shared `renderer` VM on the panel and logs per-frame
render time. Measured on the demo scene (3 `sin`/pixel, ~42 opcodes), built `--release` with the
renderer at `opt=3`:

| eval_res | render/frame | fps | VM | blit (`set_pixel`) |
|---|---|---|---|---|
| 64×64 | ~60 ms | ~16 | ~53 ms (88%) | ~7.3 ms |
| 32×32 | ~21 ms | ~48 | ~13 ms | ~7.3 ms |

Findings:

- **Build it optimized.** `cargo run` (dev: `opt="s"` + debug/overflow checks) gave ~347 ms/frame.
  A profile override builds `pixel64-renderer` at `opt=3` always (firmware/Cargo.toml); run `--release`
  so the firmware crate (the `set_pixel` blit) is fast too.
- **`sin` was the killer.** `libm::sinf` reduces in `f64`; the M33 FPU is single-precision, so it was
  software-emulated (~3,000 cyc/call) — the bulk of the original 280 ms. Replaced with a pure-`f32`
  polynomial (`renderer/src/vm.rs::fast_sin`): 280 → 57 ms (5×). See
  [docs/scenes/shader-vm.md](../../docs/scenes/shader-vm.md).
- **The interpreter is now the budget** (~88% at 64×64, ~40 cyc/opcode — bounds checks + stack ops).
  **`eval_res`** (render at a lower grid, nearest-upscale; the spec's lever) is the fps knob: 32×32
  quarters the VM work → ~48 fps. The `set_pixel` BCM blit is fixed (~7 ms; it always writes all 4,096
  panel pixels) and minor.
- **Full 64×64 at 30 fps** would need shaving the per-opcode cost (`get_unchecked` in the dispatch
  loop) — safe only once on-device **bundle validation** guarantees indices are in range, so it's
  deferred to that work, not done speculatively.

## Tuning knobs

All in `src/hub75.rs`.

| Change | Effect | Trade-off |
|--------|--------|-----------|
| `B` (bit-planes) `10` → fewer | faster refresh, less RAM | coarser color; darks band under gamma |
| `B` → more | richer color / more dark-end headroom | slower refresh; 2 × `64·64/2·B` bytes of RAM |
| `DATA_DIV` `4` → lower | faster pixel clock → faster refresh | eventually the panel/level-shifter glitch — back off if corruption appears |
| `OE_DIV` `1` → higher | effective BCM weighting (better gradients/brightness range) | longer per-plane windows → slower refresh |
| draw-task `33 ms` → lower | faster content updates | more CPU on rendering |

Rough rule of thumb: **refresh ∝ 1 / (rows × planes × per-plane time)**; per-plane time is the larger
of the pixel-shift time (set by `DATA_DIV`) and the BCM window (set by `OE_DIV`).
