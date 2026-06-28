//! pixel64 shared renderer.
//!
//! A shader-bytecode stack VM ([`vm`]) that renders a per-frame + per-pixel program across an
//! eval-resolution grid. The **same crate runs in two places**, so the web preview matches the
//! device by construction (docs/scenes/preview-and-parity.md):
//!
//! - **`wasm32`** (default `wasm` feature) → the `web/` editor preview, via the wasm-bindgen
//!   [`Program`](program::Program) wrapper which owns the bytecode and returns an RGBA framebuffer.
//! - **the device** (`default-features = false` → `no_std`, no heap) → firmware renders straight to
//!   the HUB75 driver through [`render_grid`], with a fixed-capacity [`Stack`] and no allocation.
//!
//! Uniform layout (f32 slots), per docs/scenes/shader-vm.md:
//!   0:t 1:frame 2:res.x 3:res.y 4:x 5:y 6:uv.x 7:uv.y 8:st.x 9:st.y 10..:inputs
//! The host fills 0..3 and 10.. each frame; the VM fills 4..9 per pixel.

#![cfg_attr(not(feature = "wasm"), no_std)]

mod vm;

pub use vm::{op, render_grid, run, to_u8, Stack, STACK_CAP};

#[cfg(feature = "wasm")]
mod program;
