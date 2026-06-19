# Preview & parity

The promise: what you build in the `web/` editor looks like what the panel shows. We locked **"visually
identical," not bit-identical** ([README.md](README.md)) — float math isn't guaranteed to match to the
last bit across native ARM and the browser. This doc says how we make preview faithful anyway, and how
we *operationalize* "visually identical" so it can't silently rot.

## Preview architecture: one renderer, two build targets

- **DECISION (was OQ3): compile the device renderer to WASM and run *that* in the browser.** The
  compositor + shader VM are `no_std`-friendly Rust above the framebuffer seam
  ([device-runtime.md](device-runtime.md)); we compile that same crate to `wasm32` with a thin JS
  binding. The editor hands it a scene + uniform values + frame number and gets back an RGBA buffer to
  draw on a `<canvas>`.
- **Why, despite only needing "visually identical":** maintaining one renderer instead of two is the
  real win. A JS reimplementation would drift on every change to a blend mode, an opcode, the noise
  hash, the gamma LUT — and the bugs would be subtle pixel differences. One shared implementation means
  the **entire pipeline** (image decode, text raster, blend, gamma, *and* the shader VM) is identical
  in preview and on device by construction — not just the shader.

```
   shared Rust renderer crate (no_std-friendly, above the framebuffer seam)
            │                                   │
   wasm32 (browser preview)            thumbv8m.main-none-eabihf (device)
            │                                   │
      <canvas> in editor                   HUB75 panel
```

## Why bit-exactness isn't promised (and why it doesn't matter here)

- `f32` results can differ by a few ULP between native ARM and WASM: `libm` transcendentals
  (`exp/log/pow`) differ by implementation; FMA contraction and optimizer choices differ.
- **What keeps divergence sub-perceptual:** `sin/cos/tan` and `noise` are **LUT-backed with a shared
  table and a specified integer hash** ([shader-vm.md](shader-vm.md)) — table-indexed lookups + `f32`
  interpolation barely diverge. Output is quantized to a few bits per channel through a shared gamma
  LUT, which swallows sub-LSB differences. Net: differences land below one output code in practice.

## Conformance vectors: making "visually identical" testable

A suite that *proves* the two builds agree, run in CI:

- A set of **fixtures** = (scene or raw bytecode + uniform values + frame numbers).
- A **reference** output is produced by the shared Rust renderer run natively in CI.
- Both the **WASM** build and a **device-representative** build are asserted against the reference
  within a tolerance: e.g. **max per-channel diff ≤ 1–2 / 255** and **mean diff ≤ a small fraction**.
- Any change to the VM, compositor, LUTs, noise, or gamma re-runs this suite. Drift fails CI.

**OPEN:** the exact tolerance thresholds — set them tight after measuring real native-vs-WASM
divergence on representative scenes, then hold the line.

## Editor responsibilities

Beyond drawing frames, the editor closes the authoring loop:

- **Live recompile** on edit; surface compiler errors inline
  ([shader-language.md](shader-language.md)).
- **Transport controls** — play/pause/scrub `t`, step `frame`.
- **Uniform/input simulation** — sliders and value fields for `static`/`live`/`device` inputs, so a
  data-driven scene can be exercised without the real data source
  ([inputs-and-binding.md](inputs-and-binding.md)).
- **Budget warning** — display the compiler's estimated **worst-case opcodes/pixel** against the
  device cost model ([shader-vm.md](shader-vm.md)) and warn when a scene won't hold its target fps at
  its `eval_res`. Better to catch "too expensive" in the editor than after pushing.

## Optional: ground-truth-on-device

A stretch mode for confidence: stream actual frames *from a real device* back to the editor
(over the control channel) for side-by-side comparison with the WASM preview — the ultimate parity
check on real silicon. **OPEN / deferred:** nice for validating the tolerance numbers and for demos;
not required for the core loop.

## Open questions

1. **Tolerance thresholds** for the conformance suite (above).
2. **WASM packaging** — `wasm-bindgen` vs. a hand-rolled minimal binding; size budget for the editor.
3. **Ground-truth-on-device** mode — worth building, and when.
