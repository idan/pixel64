# Layers & compositing

The spine of the scene model. A **scene** is an ordered list of **layers**; the **compositor**
walks them bottom-to-top each frame and blends them into a single 64×64 frame, which is then
quantized and handed to the panel driver.

See [README.md](README.md) for the locked platform decisions this builds on.

## Coordinate space

- The frame is **64×64**, origin **top-left**, `x` →right, `y` →down (matches `embedded-graphics`
  and the framebuffer layout). Integer pixel coordinates address pixel *centers*.
- Shader layers additionally see normalized coordinates (`uv`, `st`) — defined in
  [shader-vm.md](shader-vm.md), not here.
- One scene **clock** drives everything: `t` (f32 seconds since the scene became active) and
  `frame` (integer counter). Every time-driven layer derives from this single clock so layers
  stay in sync.

## The layer

Every layer, regardless of kind, carries a common envelope:

| Property | Type | Notes |
|---|---|---|
| `kind` | enum | `image` \| `animation` \| `text` \| `shader` |
| `visible` | bool | Skipped entirely when false. |
| `opacity` | f32 0..1 | Multiplies the layer's alpha before blending. |
| `blend` | enum | How the layer combines with what's beneath (below). |
| `offset` | (i16, i16) | Integer pixel translation `(dx, dy)`. |
| `eval_res` | (u8, u8) | Internal render resolution; upscaled to 64×64 before blending. Defaults to scene `eval_res`, default 64×64. |
| `clip` | rect? | Optional integer clip rectangle; pixels outside contribute nothing. |
| kind-specific | — | See each kind below. |

**DECISION (was OQ8): `eval_res` lives on the layer**, defaulting to a scene-level value. This lets a
cheap 32×32 plasma sit under a crisp 64×64 text layer. Upscaling is **nearest-neighbor** in v1
(bilinear is an opcode-free post-step we can add later).

**DECISION (transforms): v1 supports integer translation only.** Scale and rotation require
resampling every frame and are rarely worth it at 64×64 — deferred. `offset` is enough for scrolling
text and moving sprites. Out-of-bounds behavior per layer: `clip` (default), `wrap`, or `none`.

**DECISION (masking): v1 supports a rectangular `clip` only.** Arbitrary alpha masks are deferred;
an alpha-carrying layer above another already covers most masking needs via blending.

## Compositing model

The accumulator is an **RGBA buffer with premultiplied alpha**, in linear-ish authored space.

- **DECISION (precision): the working accumulator is `f32` RGBA** (64×64×4×4 = 64 KB). We have the
  SRAM (§ [device-runtime.md](device-runtime.md)) and it keeps multi-layer blending clean. The
  per-layer `eval_res` buffer is also f32 RGBA at its own resolution.
- **DECISION (premultiplied alpha):** layers emit premultiplied RGBA. This makes both "over" and
  "additive" correct and cheap, and makes upscaling/`opacity` a single multiply.

Compositing initializes the accumulator to **transparent black** `(0,0,0,0)`, then for each visible
layer bottom-to-top:

1. Produce the layer's premultiplied RGBA at `eval_res` (decode image / pick animation frame /
   rasterize text / run the shader VM).
2. Upscale to 64×64 (nearest), apply `offset`, apply `clip`, multiply by `opacity`.
3. Blend onto the accumulator per `blend`.

After the last layer, the accumulator is composited over **opaque black** (the panel can't show
"transparent"), gamma-corrected, and quantized to the panel framebuffer (§ Output).

### Blend modes

With premultiplied source `S` and destination `D`:

| Mode | Formula | Use |
|---|---|---|
| `normal` (source-over) | `out = S + D·(1 − S.a)` | Default; stacking sprites/text over a background. |
| `add` | `out.rgb = S.rgb + D.rgb`, `out.a = S.a + D.a·(1−S.a)` | Glows, light-on-light — natural for an LED panel. |

**DECISION (was OQ5): v1 ships `normal` + `add` only.** `multiply` and `screen` are easy to add to
the same premultiplied pipeline later but aren't needed to prove the model.

## Layer kinds

Details of each kind live in their own docs; here is what the compositor needs from each.

### image
A static bitmap from an asset reference. Carries optional alpha (or a color key → alpha). Decoded to
premultiplied RGBA at its native size, then placed per the common envelope.
→ encoding in [bundle-format.md](bundle-format.md).

### animation
A frame sequence asset + timing: a frame rate (or per-frame durations) and a play mode
(`loop` \| `pingpong` \| `once`). The current frame is derived from the scene clock `t`, so animations
stay in sync with shaders and each other. Otherwise composited like an image.
→ encoding in [bundle-format.md](bundle-format.md).

### text
A string rendered with a bitmap font at a position, with color and alignment. The string may be a
literal or **bound to an input** (this is how "templatized" scenes work — a clock, a temperature, a
countdown). Supports horizontal scroll (via animated `offset`) for strings wider than the panel.
→ string binding/formatting in [inputs-and-binding.md](inputs-and-binding.md); fonts in
[bundle-format.md](bundle-format.md).

### shader
The procedural layer: a compiled bytecode program plus a mapping of uniform slots ← scene inputs.
Evaluated per pixel at the layer's `eval_res` by the VM, producing premultiplied RGBA.
→ [shader-vm.md](shader-vm.md), [shader-language.md](shader-language.md).

## Output: gamma & quantization

The panel driver uses **binary code modulation (BCM)** over bitplanes for per-channel brightness
depth. Two questions live at this boundary:

- **Gamma.** LEDs are roughly linear in emitted light; perception is not. We apply a **gamma/brightness
  LUT at the final quantization step** (accumulator → panel framebuffer), so authors work in a
  perceptual 0..1 space and the curve is corrected once, centrally.
- **DECISION (pragmatic): we do *not* convert to linear light for blending in v1.** Correct blending
  would composite in linear and convert at the end; that's extra per-pixel cost for a subtle quality
  gain on a 64×64 panel. We blend in authored space and gamma-correct at output. Revisit if banding
  or additive blending looks wrong.
- **OPEN:** the panel's usable per-channel bit depth (and therefore bitplane count / refresh
  headroom) is a **driver** property to be confirmed when the PIO+DMA HUB75 driver is built. The
  compositor only needs to know "quantize f32 RGBA → N-bit-per-channel via this gamma LUT." Tracked
  in [device-runtime.md](device-runtime.md).

## What's deferred

Scale/rotation transforms, alpha masks, `multiply`/`screen` blend, linear-light compositing,
bilinear upscaling, scene-to-scene transitions (these live in [device-runtime.md](device-runtime.md),
not the per-frame compositor).
