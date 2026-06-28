//! wasm-bindgen entry point for the `web/` editor preview.
//!
//! Owns the decoded bytecode (per-frame + per-pixel blocks, constants, slot count) and reusable
//! scratch, and renders one frame to a premultiplied-over-black RGBA8 buffer via the shared
//! [`render_grid`](crate::render_grid) — the identical path the device uses.

use wasm_bindgen::prelude::*;

use crate::vm::{render_grid, to_u8, Stack};

const RESERVED_UNIFORMS: usize = 10;

#[wasm_bindgen]
pub struct Program {
    frame: Vec<u32>,
    pixel: Vec<u32>,
    constants: Vec<f32>,
    num_slots: usize,
    // scratch reused across frames/pixels
    slots: Vec<f32>,
    stack: Stack,
    uniforms: Vec<f32>,
    framebuffer: Vec<u8>,
}

#[wasm_bindgen]
impl Program {
    /// `instr_frame` / `instr_pixel` are flat (opcode, arg) u32 streams.
    #[wasm_bindgen(constructor)]
    pub fn new(
        instr_frame: &[u32],
        instr_pixel: &[u32],
        constants: &[f32],
        num_slots: u32,
    ) -> Program {
        Program {
            frame: instr_frame.to_vec(),
            pixel: instr_pixel.to_vec(),
            constants: constants.to_vec(),
            num_slots: num_slots as usize,
            slots: vec![0.0; num_slots as usize],
            stack: Stack::new(),
            uniforms: Vec::new(),
            framebuffer: Vec::new(),
        }
    }

    /// Render one frame. `uniforms` holds the reserved built-ins (0..3) and bound inputs (10..) for
    /// this frame; positions 4..9 are filled per pixel. Returns RGBA8 (`res * res * 4` bytes),
    /// premultiplied over opaque black.
    pub fn render(&mut self, uniforms: &[f32], res: u32) -> Vec<u8> {
        let res = res as usize;

        // Working uniform array: at least the reserved block.
        self.uniforms.clear();
        self.uniforms.extend_from_slice(uniforms);
        if self.uniforms.len() < RESERVED_UNIFORMS {
            self.uniforms.resize(RESERVED_UNIFORMS, 0.0);
        }
        // Reset slots so frame-globals start clean (render_grid also zeroes them).
        self.slots.clear();
        self.slots.resize(self.num_slots, 0.0);
        self.framebuffer.clear();
        self.framebuffer.resize(res * res * 4, 0);

        // Disjoint field borrows: the emit closure writes the framebuffer while render_grid holds the
        // other scratch.
        let Self {
            frame,
            pixel,
            constants,
            slots,
            stack,
            uniforms,
            framebuffer,
            num_slots: _,
        } = self;
        render_grid(
            frame.as_slice(),
            pixel.as_slice(),
            constants.as_slice(),
            slots.as_mut_slice(),
            stack,
            uniforms.as_mut_slice(),
            res,
            |x, y, out| {
                // premultiplied over opaque black → rgb already premultiplied
                let o = (y * res + x) * 4;
                framebuffer[o] = to_u8(out[0]);
                framebuffer[o + 1] = to_u8(out[1]);
                framebuffer[o + 2] = to_u8(out[2]);
                framebuffer[o + 3] = 255;
            },
        );

        self.framebuffer.clone()
    }
}
