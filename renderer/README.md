# renderer/ — shared scene renderer (Rust)

The **shared** scene renderer: a small stack VM that executes shader **bytecode**
into a 64×64 premultiplied-RGBA framebuffer. This is the "one renderer, two build
targets" the architecture calls for ([docs/scenes/preview-and-parity.md](../docs/scenes/preview-and-parity.md)):

- **`wasm32-unknown-unknown`** (default `wasm` feature) → the `web/` editor preview (live now).
- **the device target** (`default-features = false` → `no_std`, no heap) → firmware, above the
  framebuffer seam. **Live now** — the firmware depends on this crate and renders an embedded scene
  on the panel via [`firmware/src/scene.rs`](../firmware/src/scene.rs) (`cargo run --bin scene`).

Because the same Rust runs both places — both call the shared [`render_grid`](src/vm.rs) over the
identical VM — the preview matches the device *by construction*, not by a parallel reimplementation.

## no_std / two-target layout

The core VM ([`src/vm.rs`](src/vm.rs)) is `no_std` and allocation-free: the operand stack is a
fixed-capacity [`Stack`] (array + top index), not a `Vec`, so it runs on the device with no heap and
identically in the browser. The wasm-bindgen [`Program`](src/program.rs) wrapper (which owns bytecode
and returns an RGBA `Vec`) lives behind the default `wasm` feature; with `default-features = false`
only the `no_std` core compiles.

## Layout

```
src/vm.rs    the f32 stack VM: opcode dispatch + value noise (integer hash)
src/lib.rs   wasm-bindgen `Program`: owns bytecode, runs per-frame/per-pixel, returns RGBA
```

## The bytecode contract

The web compiler ([web/src/lib/scene/emit.ts](../web/src/lib/scene/emit.ts)) lowers
shader source → bytecode; this VM executes it. The contract is two flat
`(opcode, arg)` u32 streams (a per-frame block + a per-pixel block), an `f32`
constants pool, and a slot count. **Vectors are lowered to scalar ops by the
compiler** — the VM only knows `f32`.

- **Opcodes** are numbered in [`src/vm.rs` `mod op`](src/vm.rs), mirrored in
  [web/src/lib/scene/opcodes.ts](../web/src/lib/scene/opcodes.ts). The two **must
  stay in sync**.
- **Uniform layout** (`f32` slots): `0:t 1:frame 2:res.x 3:res.y 4:x 5:y 6:uv.x
  7:uv.y 8:st.x 9:st.y`, then bound scene inputs from index 10. The host fills
  `0..3` and `10..` each frame; the VM fills `4..9` per pixel.
- **`color`** output lives in slots `0..3`; the compiler appends a `STORE_OUT`
  epilogue to the pixel block.

### Noise is bit-portable

`hash`/`noise`/`fbm` use an **integer bit-mix hash** (lowbias32) over integer
lattice coords, not a sine-based hash. Rust `u32` wrapping ops and JS `Math.imul`
produce identical bits, so noise agrees across Rust and the JS reference. (A
sine-based hash diverged badly under `f32` — the exact failure the docs predicted.)

## Build

From `web/`: `bun run build:wasm` (wraps `wasm-pack build … --target web`). Output
lands in `web/src/lib/renderer-wasm/` (gitignored build artifact). Requires the
`wasm32-unknown-unknown` target and `wasm-pack`.

## Parity

The editor renders each frame with both this WASM VM and the TS interpreter and
reports the max per-channel diff. All current example scenes are **bit-identical**
(Δ0/255). A proper conformance-vector CI suite (WASM vs. a native-Rust reference)
is the next step ([docs/scenes/preview-and-parity.md](../docs/scenes/preview-and-parity.md)).

## Status / shortcuts

- **`no_std`-ready and running on-device** (fixed-capacity stack, no heap); the wasm `Program` is
  feature-gated. Most transcendentals use `libm` (no system libm on `wasm32`).
- **`sin`/`cos` use a pure-`f32` polynomial** (`vm::fast_sin`), not `libm` and not the LUT the spec
  originally locked: `libm::sinf` range-reduces in `f64`, which the device's single-precision FPU
  software-emulates (~3000 cyc) and dominated on-device frame time. The polynomial is ~20 cyc and uses
  only IEEE f32 +,−,× → device and WASM agree bit-for-bit. The web TS parity-reference
  (`web/src/lib/scene/builtins.ts`) uses the same polynomial, so the editor's WASM-vs-TS harness stays
  exact on `sin`/`cos` scenes.
- `tan`/`atan2`/`exp`/`log`/`pow` still use `libm` (rare per-pixel).
- Single shader layer only — no multi-layer compositor / blend modes yet, and no gas budget on the
  interpreter (the device's embedded scene is trusted; gas comes with untrusted/network bundles).
