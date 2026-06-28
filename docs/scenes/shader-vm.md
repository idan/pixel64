# Shader VM

The execution engine behind a **shader layer**. It runs a compiled bytecode program to produce
premultiplied RGBA per pixel: conceptually `f(coords, t, uniforms) → rgba`. The same VM runs on the
device (native ARM) and in the browser preview (compiled to WASM) — see
[preview-and-parity.md](preview-and-parity.md).

## Value & machine model

- **DECISION (was OQ1): scalar `f32` core.** The VM only knows `f32` scalars. Vectors (`vec2/3/4`) are
  a *language* convenience that the compiler **lowers to scalar ops** ([shader-language.md](shader-language.md)).
  The interpreter stays tiny; no per-opcode type dispatch.
- **DECISION (was OQ2): stack machine.** An `f32` operand stack, plus three indexed arrays:
  - **constants** — `f32` pool, read-only.
  - **uniforms** — `f32` slots written by the runtime each frame (built-ins + bound inputs).
  - **slots** — scratch registers for `let` bindings and frame-globals (compiler-allocated indices).

  Stack-based is trivial to compile to and to interpret; revisit register-based only if profiling
  on real hardware demands it.

### Uniform layout

The runtime populates the uniform array before execution. Reserved low slots are built-ins; the rest
are bound scene inputs ([inputs-and-binding.md](inputs-and-binding.md)):

| Slot | Name | When written |
|---|---|---|
| 0 | `t` (seconds) | per frame |
| 1 | `frame` | per frame |
| 2,3 | `res.x`, `res.y` | per layer setup |
| 4,5 | `x`, `y` (pixel coords) | **per pixel** |
| 6,7 | `uv.x`, `uv.y` (0..1) | per pixel |
| 8,9 | `st.x`, `st.y` (centered, −1..1, aspect-corrected) | per pixel |
| 10… | bound inputs | per frame (or eased) |

`uv = (pixel + 0.5) / res`. `st = (uv − 0.5) · 2`, with the longer axis scaled by aspect (1.0 for the
square panel, but defined for generality).

## Execution structure: per-frame / per-pixel

The defining performance lever (LOCKED in [README.md](README.md)). A program has two bytecode blocks:

- **`per_frame`** — runs **once per rendered frame**. Reads `t`, `frame`, bound inputs. Writes
  **frame-global slots** (oscillators, palette phase, precomputed constants). The slots persist into
  the per-pixel block.
- **`per_pixel`** — runs **once per evaluated pixel**. Reads the per-pixel uniforms (`x/y/uv/st`),
  the frame-globals, and any uniforms. Writes the four **output slots** `r,g,b,a` (premultiplied,
  clamped 0..1). 

Anything not dependent on pixel position belongs in `per_frame` — the compiler hoists where it safely
can, but the author's block placement is the primary signal.

## Eval resolution & frame rate

A shader layer declares `eval_res` (≤ 64×64) and the scene declares a target fps. The runtime runs
`per_frame` once, then `per_pixel` across the `eval_res` grid, into the layer's f32 RGBA buffer, which
the compositor upscales ([layers-and-compositing.md](layers-and-compositing.md)). Between frames (panel
refreshes at hundreds of Hz via BCM, scene fps is much lower) the last buffer is reused.

## Opcode set

Deliberately small. Higher-level functions (`length`, `hsv`, `palette`, `fbm`, vector ops) are **not
opcodes** — they're **prelude functions written in the shader language** and lowered to these
primitives ([shader-language.md](shader-language.md)). The dedicated opcodes:

| Group | Opcodes |
|---|---|
| Stack/memory | `PUSH_CONST i`, `LOAD_UNIFORM i`, `LOAD_SLOT i`, `STORE_SLOT i`, `DUP`, `POP`, `SWAP` |
| Arithmetic | `ADD`, `SUB`, `MUL`, `DIV`, `NEG`, `MOD` (fmod) |
| Math | `ABS`, `FLOOR`, `CEIL`, `FRACT`, `SIGN`, `SQRT`, `MIN`, `MAX`, `CLAMP`, `MIX`, `STEP`, `SMOOTHSTEP` |
| Transcendental | `SIN`, `COS`, `TAN`, `ATAN2`, `EXP`, `LOG`, `POW` |
| Noise/hash | `HASH`, `NOISE2`, `NOISE3` |
| Compare/logic | `LT`, `GT`, `LE`, `GE`, `EQ`, `NE`, `AND`, `OR`, `NOT`, `SELECT` (all yield/consume 1.0/0.0) |
| Control flow | `JMP off`, `JMP_IF_ZERO off` |
| Output | `STORE_OUT c` (c ∈ r,g,b,a), `END` |

### Transcendentals & LUTs

- **`SIN`/`COS` use a pure-`f32` polynomial** (REVISED from the original LUT decision). `libm::sinf`
  range-reduces in `f64`, and the RP2350's Cortex-M33 FPU is **single-precision only** → that `f64` is
  software-emulated (~3,000 cycles/call) and dominated on-device rendering (measured: 280 ms/frame, of
  which `sin` was the bulk). A degree-9 odd-Taylor approximation over `[-π/2, π/2]`
  (`renderer/src/vm.rs::fast_sin`) is ~20 cycles, ~1e-6 error (far under 1/255), and uses only IEEE
  `f32` +,−,× (no FMA) so device and WASM produce **identical bits** — same goals as the LUT (f64-free,
  fast, deterministic) with no table to ship and better accuracy. The web TS parity-reference
  (`web/src/lib/scene/builtins.ts`) uses the same polynomial, so the editor's WASM-vs-TS harness stays
  green on `sin`/`cos` scenes.
- **`TAN`/`ATAN2` still use `libm`** (rare per-pixel); promote to `fast_*` if they ever show up hot.
- **DECISION: `EXP`/`LOG`/`POW` use `libm` (`f32`) initially**, not LUTs. They're rarer per-pixel and
  the FPU makes them tolerable; promote to LUTs only if profiling says so. (Parity note: `libm`
  results may differ by a few ULP between native and WASM — within our visual tolerance,
  [preview-and-parity.md](preview-and-parity.md).)

### Noise

- **DECISION (was OQ): value noise, not Perlin/Simplex, for v1.** `NOISE2`/`NOISE3` are gradient-free
  **value noise** over a fixed integer hash with smoothstep interpolation. Reasons: trivial to specify
  bit-for-bit (so preview matches), no Simplex patent concerns, cheap. `fbm` (fractal sum) is a prelude
  function that calls `NOISE*` a fixed number of octaves.
- `HASH` is a deterministic `f32`→`f32` pseudo-random (integer bit-mix), the basis of the noise and of
  any author-level randomness. The exact mix function is **specified** so device and preview agree.
- `NOISE3`'s third axis is typically `t` → free animated noise without storing frames.

## Control flow & the gas budget

- **DECISION (was OQ4): real `if`/`else` and compile-time-bounded `for` are allowed.** Branches are
  free on a scalar in-order CPU (no SIMD divergence penalty), so there's no reason to force branchless
  `select`. `select` remains available as an opcode for cheap conditional values.
- Loops must have a **compile-time-constant trip count**; the compiler may unroll small ones. No
  `while` / no data-dependent loop bounds.
- **Gas budget.** Independent of static bounds, the per-pixel execution carries an **instruction
  budget** (gas). If a program exceeds it, evaluation of that pixel halts and emits the current output
  slots (or transparent). This *guarantees* bounded frame time even if a program is pathological, and
  — because compute is on its own core ([device-runtime.md](device-runtime.md)) — a costly shader
  degrades fps rather than stalling the panel.

## Resource limits (→ bundle validation & capability versioning)

These are validated at compile time in `web/` and enforced/advertised by the device:

| Limit | Purpose |
|---|---|
| max bytecode size (per block) | flash/RAM bound |
| max slots | scratch RAM bound |
| max constants | pool bound |
| max stack depth | interpreter stack bound |
| gas per pixel | frame-time bound |

The bytecode carries a **`vm_version`** (opcode-set revision). The device advertises the versions it
supports so the cloud never pushes bytecode using opcodes a given firmware lacks
([bundle-format.md](bundle-format.md)).

## Cost model (the per-pixel envelope)

Rough budget so the compiler and editor can warn before pushing (numbers to be validated on hardware):

- Compute core: ~150 MHz, dedicated (core 1).
- 64×64 @ 30 fps = ~123k px/s → **~1,200 cycles/pixel** for everything.
- FPU `f32` add/mul ≈ 1–3 cycles; LUT `sin` ≈ 10–20; interpreter dispatch ≈ 5–15 cycles/opcode.
- ⇒ on the order of **50–100 opcodes/pixel at full res, 30 fps**. Lower `eval_res` or fps buys
  proportional headroom.

The editor estimates a program's **worst-case opcodes/pixel** (static analysis + loop bounds) and
flags scenes that won't hold their target fps on-device — see
[preview-and-parity.md](preview-and-parity.md).

## Open questions

1. **Promote `exp/log/pow` to LUTs?** Deferred until profiled.
2. **Gas default value** — pick once we can measure real opcode costs on RP2350.
3. **Exact limit numbers** (sizes, slots, stack, gas) — set after a first implementation pass.
4. **Output color space** — confirm authors target perceptual 0..1 with gamma applied at output
   (current assumption, see [layers-and-compositing.md](layers-and-compositing.md)).
