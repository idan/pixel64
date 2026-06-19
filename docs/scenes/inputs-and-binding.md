# Inputs & binding

The mechanism that turns a static scene into a **templatized** or **live** one. A scene declares a
set of named **inputs**; layer properties can be **bound** to them; values are supplied at render
time — some fixed when the scene is configured, some pushed live from the cloud, some provided by the
device itself.

This is the single feature behind both "templatized images" (text bound to a value) and
"data-driven procedural scenes" (shader uniforms bound to live data).

## Declaring inputs

A scene's manifest carries an `inputs` table. Each entry:

| Field | Notes |
|---|---|
| `name` | Identifier, referenced by bindings. |
| `type` | `float` \| `int` \| `bool` \| `color` \| `string` (v1 set). |
| `default` | Used until/unless a value is supplied. Required. |
| `source` | `static` \| `live` \| `device` (below). |
| `min`/`max` | Optional clamp for numeric types. |
| `ease_ms` | Optional; numeric/color only — see § Update semantics. |

**DECISION (was OQ6, type set): v1 inputs are `float`, `int`, `bool`, `color`, `string`.** Vectors and
image-refs are out of scope for v1 — a shader can build vectors from float inputs, and swapping an
image is a bundle change, not an input change.

### Input sources

- **`static`** — set when a scene is instantiated/configured (e.g. "the countdown target is
  2026-12-31", "the accent color is teal"). Travels in the bundle.
- **`live`** — pushed from the `web/` backend to the running device over the network: weather,
  a stock tick, presence, a queue length, a "now playing" string. Updated without re-sending the
  bundle (§ Live update channel).
- **`device`** — supplied by the firmware itself. A small built-in set:

  | Device input | Type | Source |
  |---|---|---|
  | `now` | int (unix seconds) | SNTP-synced wall clock |
  | `uptime` | float (seconds) | since boot |
  | `local_hms` | float×3 / packed | hour/minute/second for clocks |

  **OPEN:** the exact device-input set (sensors? brightness/ambient light if hardware grows?) — start
  with time only.

## Binding layer properties

Any kind-specific layer property may be either a **literal** or a **binding**.

- Numeric/color/bool property → reference an input directly: `{ "$input": "temperature" }`.
- **Shader uniforms** → the shader layer carries a `uniforms` map: `vm_slot ← input_name`. The input
  manager writes the current input values into the VM's uniform table each frame
  (§ [shader-vm.md](shader-vm.md)). This is how live data drives procedural pixels.
- **String properties** (text layer) → **interpolation**, below.

### String interpolation

A text layer's `text` is a template string with `{name}` placeholders filled from inputs, with an
optional format spec after a colon:

```
"{temp:.1f}°C"      → "21.4°C"
"{count:02d} new"   → "03 new"
"{now:%H:%M}"       → "14:09"     (time formatting for int unix-seconds inputs)
```

**DECISION (format mini-language): a deliberately small spec.** Numeric: width, zero-pad, fixed
decimals (`{x:6.2f}`, `{n:03d}`). Time: a `strftime`-subset for unix-seconds inputs (`%H %M %S %d %m
%Y %a %b`). No locale, no arbitrary expressions — keep the device-side formatter tiny. Anything richer
is computed cloud-side and pushed as a pre-formatted `string` input.

## Live update channel

Distinct from bundle delivery (§ [bundle-format.md](bundle-format.md)): a lightweight message that
updates one or more input values on the *currently active* scene.

```
{ scene: <id>, values: { "temperature": 21.4, "status": "all good" } }
```

- The device validates each value against the declared `type` and `min`/`max`; mismatches are
  rejected and logged, leaving the prior value intact.
- Transport is the persistent **WebSocket** control channel ([transport.md](transport.md)); this doc
  fixes only the *shape* and *semantics* of the values it carries.
- If a `live` input has never received a value, its `default` is used.

## Update semantics

- **Frame-coherent sampling.** All input values are latched once at the start of each rendered frame,
  so every layer/uniform within a frame sees a consistent snapshot.
- **Optional easing.** A numeric/color input with `ease_ms > 0` is not applied instantly; the device
  eases the rendered value from its current toward the newly received target over `ease_ms`
  milliseconds. This makes a temperature or gauge change *glide* instead of jumping.
  **DECISION:** easing is **in v1** but **opt-in per input** (default `ease_ms = 0` = snap). It's cheap
  (one lerp per eased input per frame) and dramatically improves how live data feels.

## Open questions

1. **Device-input set** beyond time (see OPEN above).
2. ~~Transport for the live channel~~ — **RESOLVED:** WebSocket control channel, see
   [transport.md](transport.md). The concrete message schema remains to be specified there.
3. Whether `enum`/`select` inputs are worth adding for editor ergonomics, or whether `int` + author
   convention suffices.
