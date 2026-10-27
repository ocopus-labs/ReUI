//! Embeds the payload (see `src/payload_gen.rs`) and the app manifest (asInvoker, modern
//! common controls, per-monitor DPI).

#[path = "src/payload_gen.rs"]
mod payload_gen;

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=REUI_PAYLOAD_DIR");
    println!("cargo:rerun-if-changed=src/payload_gen.rs");
    println!("cargo:rerun-if-changed=setup.manifest");

    let dir = std::env::var_os("REUI_PAYLOAD_DIR").filter(|d| !d.is_empty()).map(PathBuf::from);
    let (code, inputs) = payload_gen::generate(dir.as_deref()).unwrap_or_else(|e| panic!("{e}"));
    for input in &inputs {
        println!("cargo:rerun-if-changed={}", input.display());
    }
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(out.join("payload.rs"), code).expect("write payload.rs");

    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("setup.manifest");
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", manifest.display());
    }
}
