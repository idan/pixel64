# Shader language

The authoring surface over the [shader VM](shader-vm.md). A small, GLSL-flavored language so the
mental model is "a pixel shader," compiled in `web/` to VM bytecode. Authors write two blocks —
cheap per-frame setup and per-pixel color — and get a live preview.

## A complete example

```glsl
// Uniforms are bound to scene inputs in the bundle (inputs-and-binding.md).
uniform float speed;        // e.g. a live or static input
uniform vec3  tint;

// Runs once per frame. Top-level decls here are "frame-globals".
frame {
    float phase = t * speed;
}

// Runs once per pixel. Must assign `color` (premultiplied vec4) by the end.
pixel {
    float d   = length(st);                       // st: centered, -1..1
    vec3  c   = palette(phase + d) * tint;        // IQ cosine palette (prelude)
    float a   = smoothstep(1.0, 0.0, d);          // vignette to transparent edge
    color = vec4(c * a, a);                       // premultiplied
}
```

- **DECISION (block structure): explicit `frame { }` and `pixel { }` blocks.** `frame` is optional;
  `pixel` is required and must assign `color`. We rejected auto-classifying statements into
  per-frame/per-pixel — explicit placement is predictable and maps 1:1 to the VM's two blocks. The
  compiler may still *hoist* a provably pixel-invariant sub-expression from `pixel` into `frame`.

## Types

- **DECISION (was OQ1): `float`, `vec2`, `vec3`, `vec4`, `bool`.** No first-class `int` — loop
  counters are the only integer-ish need and are handled by the `for` form below. Vectors are
  **compile-time sugar**: the type-checker tracks them, but every vector is lowered to N scalar `f32`
  values and every vector op to N scalar ops before bytecode emission.
- **Constructors:** `vec3(1.0)` (splat), `vec3(x, y, z)`, `vec4(rgb, a)`.
- **Swizzles:** `.xyzw` and `.rgba` (read; v1 read-only, no swizzle-assignment). `v.xy`, `c.rgb`,
  `v.yx` etc.
- **Operators:** `+ - * /` are component-wise on vectors; scalar·vector broadcasts. `* / %` , unary
  `-`, comparisons (`< <= > >= == !=` on scalars), `&& || !`, ternary `?:`.

## Built-in variables

| In scope | Name | Meaning |
|---|---|---|
| both | `t` | seconds since scene start (`f32`) |
| both | `frame` | frame counter |
| both | `res` | `vec2` eval resolution |
| `pixel` | `uv` | `vec2`, 0..1, pixel center |
| `pixel` | `st` | `vec2`, centered −1..1, aspect-corrected |
| `pixel` | `xy` | `vec2`, integer pixel coords as float |
| `pixel` | `color` | `vec4` output (premultiplied), must be assigned |

`uniform` declarations introduce externally-bound values
([inputs-and-binding.md](inputs-and-binding.md)).

## Built-in functions

Mapped either directly to opcodes or provided by the **prelude** (in-language, lowered to primitives):

- **Common math (opcodes):** `abs floor ceil fract mod sign sqrt min max clamp mix step smoothstep
  pow exp log`.
- **Trig (LUT opcodes):** `sin cos tan atan(y,x)`.
- **Geometry (prelude → scalar):** `length distance dot normalize` (and `cross` for `vec3` if needed).
- **Color (prelude):** `hsv(h,s,v)→vec3`, `palette(t)→vec3` (configurable IQ cosine palette; a
  default plus an overload taking `a,b,c,d` coefficients), `gamma`/`tonemap` helpers if useful.
- **Noise (opcodes + prelude):** `hash(x)`, `noise(vec2)`, `noise(vec3)`, `fbm(p, octaves)` where
  `octaves` is a compile-time constant (the prelude unrolls it).
- **DECISION:** the **IQ cosine `palette` is a first-class prelude builtin** — it's the single most
  useful primitive for good-looking procedural color and costs almost nothing.

## Control flow

```glsl
if (d < 0.5) { color = vec4(1.0); } else { color = vec4(0.0); }

for (i in 0..8) {            // compile-time-constant bounds only
    acc += noise(p * float(i));
}
```

- `if`/`else` and ternary compile to `JMP`/`JMP_IF_ZERO` (or to `SELECT` for simple value choices).
- `for` requires **constant bounds**; small loops may be unrolled. No `while`, no data-dependent
  bounds ([shader-vm.md](shader-vm.md)). The loop variable is a float in `0..n`.

## Compilation pipeline (`web/`, TypeScript/Bun)

1. **Parse** → AST.
2. **Type-check** — vector types, builtin signatures, `color` assigned on every path, uniforms
   declared.
3. **Lower** — vectors → scalars, vector ops → scalar ops, prelude inlined, `for` bounds resolved.
4. **Emit** — stack bytecode for `frame` and `pixel` blocks; allocate slots/constants.
5. **Optimize** — constant-fold, peephole, hoist pixel-invariant expressions into `frame`.
6. **Validate** — against the device limits (size, slots, stack depth, worst-case gas) and the target
   `vm_version`. Errors and the estimated **opcodes/pixel** surface in the editor
   ([preview-and-parity.md](preview-and-parity.md)).

The output is the bytecode + uniform-binding metadata that the bundle carries
([bundle-format.md](bundle-format.md)).

## Prelude

A standard library written in the shader language itself, prepended at compile time: `length`,
`distance`, `dot`, `normalize`, `hsv`, `palette`, `fbm`, `rotate2d`, easing helpers, etc. Keeping these
in-language (rather than as opcodes) keeps the VM minimal and lets the library grow without firmware
changes — as long as it only uses existing opcodes.

## Open questions

1. **`cross`/full `vec3` geometry** — include now or defer? (Most 64×64 work is 2D.)
2. **Swizzle-assignment** (`v.xy = ...`) — nice-to-have, adds compiler work; defer?
3. **Author-defined functions** beyond the prelude — allow `fn`-style helpers in a scene's source, or
   keep scenes to the two blocks + prelude for v1?
4. **Exact prelude contents** — finalize alongside the first real scenes.
