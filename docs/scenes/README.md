# Scenes — architecture

How content reaches the 64×64 panel: the data model for what's displayed, how it's
authored and previewed in the web app, how it's packaged and delivered, and how it
executes on the device.

This is a **shared** concern — it spans `web/` (authoring, preview, delivery) and
`firmware/` (storage, scheduling, rendering). It lives at the repo root for that reason.

> **Status: design.** These documents describe a system that is *not yet implemented*.
> They exist to settle the architecture before we write code. Decisions marked
> **LOCKED** are settled; **OPEN** items are still under discussion.

## The one-sentence model

> A **scene** is an ordered stack of **layers**, composited bottom-to-top into a 64×64
> frame at a target frame rate; some layer properties may be **bound** to named **inputs**
> whose values are supplied at render time.

Everything we want to display is expressible in that one model:

| Want | How it maps |
|---|---|
| Static image | A scene with one **image** layer |
| Animated image | One **animation** layer (frame sequence + timing) |
| Templatized image | An image/animation layer **+** a **text** layer whose string is *bound to an input* |
| Procedural scene | A **shader** layer: `f(uv, t, inputs) → rgba`, evaluated per pixel by a small VM |

"Templatized" is therefore **not a fourth layer kind** — it's the data-binding mechanism
(§ Inputs) applied to any layer's properties. This keeps the taxonomy to four layer kinds,
not a combinatorial explosion of "image-with-text", "animation-with-text", etc.

## Locked decisions

These follow from the conversation that produced these docs; they shape everything downstream.

- **LOCKED — Target platform: Raspberry Pi Pico 2 W (RP2350), Rust + Embassy.** Migrating off
  the ESP32-C6 to get PIO+DMA HUB75 driving (flicker fix) and a second core. Wireless via the
  `cyw43` crate + `embassy-net`.
- **LOCKED — ARM Cortex-M33 cores, hardware FPU.** RP2350 boots either dual M33 (ARM, has FPU)
  or dual Hazard3 (RISC-V, no FPU). The supported Rust/Embassy path is the M33
  (`thumbv8m.main-none-eabihf`), which is also the path *with* the single-precision FPU. We take it.
- **LOCKED — Scene math is native `f32` on the device.** The hardware FPU is single-precision, so
  scene/VM math is `f32` throughout (never `f64` — that's software-emulated). Shader semantics are
  `f32`-native, matching GLSL.
- **LOCKED — Preview is "visually identical," not bit-identical.** We accept that the web preview
  and the device may differ by sub-perceptual rounding (float determinism across ARM-native and the
  browser is not guaranteed). Parity is enforced by tolerance, not bit-equality (§ Preview & parity).
- **LOCKED — `sin`/`cos`/`noise` use lookup tables.** Even with the FPU, transcendentals are
  software; the per-pixel budget wants LUTs regardless of float vs. fixed-point.
- **LOCKED — Per-frame / per-pixel split.** The shader VM evaluates a cheap *per-frame* block once,
  then a *per-pixel* block per pixel. This is the primary compute lever and survives having an FPU.
- **LOCKED — HAL-agnostic seam at the framebuffer.** The compositor and VM operate strictly *above*
  a "produce a 64×64 framebuffer" boundary; the panel driver (PIO+DMA HUB75) is a swappable backend
  below it. This is what makes the ESP32-C6 → RP2350 panel-driver rewrite *not* touch scene code.

## Architectural seams

```
   web/ (authoring + preview)              firmware/ (storage + render)
  ┌───────────────────────────┐          ┌──────────────────────────────┐
  │  scene editor / preview    │  bundle  │  scene store (flash)          │
  │  shader compiler → bytecode│ ───────► │  scheduler / playlist         │
  │  WASM preview VM           │  (wire)  │  input manager (live values)  │
  └───────────────────────────┘  inputs  │  compositor ── shader VM      │
                                  ──────► │      │                        │
                                          │      ▼  framebuffer (seam)    │
                                          │  panel driver (PIO+DMA HUB75) │
                                          └──────────────────────────────┘
                                                    core 0 │ core 1 (compute)
```

The **bundle** (scene definition + assets + shader bytecode) is the contract between web and
firmware. **Inputs** are a second, lighter-weight channel: live values pushed to a running scene
without re-sending the whole bundle.

## Document map

Each of these is a sibling document. Marked ✍️ = drafted, 📋 = planned.

| Doc | Covers |
|---|---|
| ✍️ [layers-and-compositing.md](layers-and-compositing.md) | The four layer kinds, their properties, transforms, blend modes, opacity/masking, and the compositor pass. |
| ✍️ [inputs-and-binding.md](inputs-and-binding.md) | Declaring scene inputs (name/type/default/source), binding layer properties to them, string interpolation, live updates. |
| ✍️ [shader-vm.md](shader-vm.md) | The procedural-layer VM: value model, per-frame/per-pixel execution, opcode set, built-ins, LUTs, eval-resolution/fps, resource bounds. |
| ✍️ [shader-language.md](shader-language.md) | The GLSL-ish authoring language: grammar, types (scalar/vec), built-in functions, and how it lowers to bytecode. |
| ✍️ [bundle-format.md](bundle-format.md) | On-the-wire / on-flash scene package: manifest, asset encoding, bytecode, content-addressing, schema + capability versioning, delivery. |
| ✍️ [transport.md](transport.md) | How device ↔ cloud talk: WebSocket-to-Durable-Object (push) + HTTPS-by-hash (bulk), the Embassy TLS/WS stack, Hibernation, auth, phasing. |
| ✍️ [device-runtime.md](device-runtime.md) | Firmware side: scene store, scheduler/playlist, input manager, render loop, the two-core split, and the RAM/flash budget. |
| ✍️ [preview-and-parity.md](preview-and-parity.md) | Web preview architecture (shared Rust→WASM VM vs. JS reimplementation), and the conformance-vector suite that keeps preview ≈ device. |

## Decisions taken while drafting

The eight original open questions were resolved during the first drafting pass (rationale in the
linked docs):

1. **VM shape** → scalar `f32` core; vec types are language sugar lowered to scalar. (shader-vm.md)
2. **VM machine model** → stack-based first. (shader-vm.md)
3. **Preview VM** → shared Rust renderer compiled to WASM (one implementation, not two). (preview-and-parity.md)
4. **Control flow** → real `if`/`else` + compile-time-bounded `for`, with a per-pixel gas budget. (shader-vm.md)
5. **Blend modes** → `normal` + `add` for v1. (layers-and-compositing.md)
6. **Input sources** → all three (`static`/`live`/`device`) from the start; live data is the point. (inputs-and-binding.md)
7. **Asset encoding** → palette-indexed + RLE primary, RGBA8888 secondary; assets in flash, decoded on demand. (bundle-format.md)
8. **Resolution/fps** → `eval_res` per layer, defaulting to a scene-level value. (layers-and-compositing.md)

## Remaining open questions

These survived the drafting pass and want your input — consolidated in the handoff at the end of the
drafting session. They cluster as: the binary **container format**, the **scheduler/transitions**
scope, several "v1 or v1.1" **feature-scope** calls (animation delta encoding, fonts, author-defined
functions, swizzle-assignment), and numeric **limits/tolerances** that need real hardware to set. See
the "Open questions" section at the foot of each deeper doc.

**Resolved since:** live-channel **transport** → WebSocket-to-Durable-Object (push) + HTTPS-by-hash
(bulk), see [transport.md](transport.md). Two sub-decisions remain open there: TLS cert
pinning-vs-validation, and the device token scheme.
