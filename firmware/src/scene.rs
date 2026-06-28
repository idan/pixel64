//! On-device scene rendering — runs the shared shader VM (`pixel64-renderer`) onto the HUB75 panel.
//!
//! MVP: one **embedded** example scene (an animated RGB plasma) rendered straight to the driver, to
//! prove the shared VM executes on the RP2350 and animates the panel. Each pixel's linear `0..1`
//! color is quantized with the renderer's `to_u8` and written via [`Display::set_pixel`], whose gamma
//! LUT is the **output-gamma stage** the scene pipeline specifies (docs/scenes/layers-and-compositing.md)
//! — so the device's color path lines up with the intended preview output.
//!
//! Deliberately *not* here yet (see docs/scenes/device-runtime.md): the flash scene store, network
//! delivery, the multi-layer compositor, live-input uniforms, and moving the per-pixel work to core 1.
//! This is the single-layer, single-scene floor those build on.

use embedded_graphics::pixelcolor::Rgb888;
use pixel64_renderer::{op, render_grid, to_u8, Stack};

use crate::hub75::{self, Display};

/// Upper bound on a scene's scratch slots on-device (fixed so there's no heap). Scenes the device
/// accepts must fit; the web compiler enforces the same kind of limit (docs/scenes/shader-vm.md).
const MAX_SLOTS: usize = 32;

/// An embedded shader program: two flat (opcode, arg) u32 streams + a constants pool, matching the
/// renderer's bytecode contract (renderer/README.md).
pub struct Scene {
    pub frame: &'static [u32],
    pub pixel: &'static [u32],
    pub constants: &'static [f32],
    pub num_slots: usize,
}

// --- Embedded demo scene: animated RGB plasma ---
//
//   r = 0.5 + 0.5·sin(uv.x·τ + t)
//   g = 0.5 + 0.5·sin(uv.y·τ + t·1.3)
//   b = 0.5 + 0.5·sin((uv.x + uv.y)·τ + t·0.7)
//   a = 1
//
// Hand-authored bytecode (no per-frame block, no slots) — exercises PUSH_CONST/LOAD_UNIFORM/MUL/ADD/
// SIN/STORE_OUT. It isn't tied to a web scene; the point is to prove the VM runs on-device, and the
// shared VM would render it identically in the preview.

// Constants pool indices.
const HALF: u32 = 0;
const TAU: u32 = 1;
const G_RATE: u32 = 2;
const B_RATE: u32 = 3;
const ONE: u32 = 4;
const CONSTANTS: &[f32] = &[0.5, core::f32::consts::TAU, 1.3, 0.7, 1.0];

// Uniform indices (see the renderer uniform layout).
const T: u32 = 0;
const UV_X: u32 = 6;
const UV_Y: u32 = 7;

const FRAME: &[u32] = &[];

#[rustfmt::skip]
const PIXEL: &[u32] = &[
    // r = 0.5 + 0.5 * sin(uv.x * TAU + t)
    op::PUSH_CONST,   HALF,
    op::PUSH_CONST,   HALF,
    op::LOAD_UNIFORM, UV_X,
    op::PUSH_CONST,   TAU,
    op::MUL,          0,
    op::LOAD_UNIFORM, T,
    op::ADD,          0,
    op::SIN,          0,
    op::MUL,          0,      // 0.5 * sin(...)
    op::ADD,          0,      // 0.5 + ...
    op::STORE_OUT,    0,      // -> r

    // g = 0.5 + 0.5 * sin(uv.y * TAU + t * 1.3)
    op::PUSH_CONST,   HALF,
    op::PUSH_CONST,   HALF,
    op::LOAD_UNIFORM, UV_Y,
    op::PUSH_CONST,   TAU,
    op::MUL,          0,
    op::LOAD_UNIFORM, T,
    op::PUSH_CONST,   G_RATE,
    op::MUL,          0,      // t * 1.3
    op::ADD,          0,
    op::SIN,          0,
    op::MUL,          0,
    op::ADD,          0,
    op::STORE_OUT,    1,      // -> g

    // b = 0.5 + 0.5 * sin((uv.x + uv.y) * TAU + t * 0.7)
    op::PUSH_CONST,   HALF,
    op::PUSH_CONST,   HALF,
    op::LOAD_UNIFORM, UV_X,
    op::LOAD_UNIFORM, UV_Y,
    op::ADD,          0,      // uv.x + uv.y
    op::PUSH_CONST,   TAU,
    op::MUL,          0,
    op::LOAD_UNIFORM, T,
    op::PUSH_CONST,   B_RATE,
    op::MUL,          0,      // t * 0.7
    op::ADD,          0,
    op::SIN,          0,
    op::MUL,          0,
    op::ADD,          0,
    op::STORE_OUT,    2,      // -> b

    // a = 1
    op::PUSH_CONST,   ONE,
    op::STORE_OUT,    3,
    op::END,          0,
];

/// The built-in demo scene.
pub const DEMO: Scene = Scene {
    frame: FRAME,
    pixel: PIXEL,
    constants: CONSTANTS,
    num_slots: 0,
};

/// Render one frame of `scene` at time `t` (seconds) / `frame` index into the display's inactive
/// buffer. The caller `commit()`s.
///
/// `eval_res` is the shader eval resolution (docs/scenes/shader-vm.md): the VM runs over an
/// `eval_res × eval_res` grid and the result is nearest-neighbour upscaled to the 64×64 panel.
/// `eval_res = 64` is full resolution; `32` quarters the per-pixel VM work for a proportional fps win
/// at coarser spatial detail. Must divide 64 evenly.
pub fn render(d: &mut Display, scene: &Scene, t: f32, frame: u32, eval_res: usize) {
    let eval_res = eval_res.clamp(1, hub75::W);
    let scale = hub75::W / eval_res;

    // Reserved built-in uniforms (0..9); bound scene inputs (10..) come with the input manager later.
    let mut uniforms = [0.0f32; 10];
    uniforms[0] = t;
    uniforms[1] = frame as f32;
    uniforms[2] = eval_res as f32; // res.x
    uniforms[3] = eval_res as f32; // res.y

    let mut slots = [0.0f32; MAX_SLOTS];
    let mut stack = Stack::new();

    render_grid(
        scene.frame,
        scene.pixel,
        scene.constants,
        &mut slots[..scene.num_slots],
        &mut stack,
        &mut uniforms,
        eval_res,
        |ex, ey, out| {
            let color = Rgb888::new(to_u8(out[0]), to_u8(out[1]), to_u8(out[2]));
            // Nearest-neighbour upscale: fill the scale×scale panel block for this eval cell.
            let (px, py) = (ex * scale, ey * scale);
            for dy in 0..scale {
                for dx in 0..scale {
                    d.set_pixel(px + dx, py + dy, color);
                }
            }
        },
    );
}
