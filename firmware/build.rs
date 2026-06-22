//! Put `memory.x` on the linker search path and pass the RP2350 / cortex-m link args.
//!
//! (Adapted from the embassy `examples/rp235x` build script. We drop `-Tdefmt.x` because this
//! firmware logs via `log` over USB-serial, not defmt — see docs/pico-port.md.)

use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    // Put `memory.x` in our output directory and ensure it's on the linker search path.
    let out = &PathBuf::from(env::var_os("OUT_DIR").unwrap());
    File::create(out.join("memory.x"))
        .unwrap()
        .write_all(include_bytes!("memory.x"))
        .unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");

    println!("cargo:rustc-link-arg-bins=--nmagic");
    println!("cargo:rustc-link-arg-bins=-Tlink.x");
}
