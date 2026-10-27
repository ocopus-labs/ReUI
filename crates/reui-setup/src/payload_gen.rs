//! Generates the code that embeds the release binaries into `reui-setup.exe`.
//!
//! Shared by `build.rs` (via `#[path]`) and the crate's tests, so it uses only `std`.
//! `REUI_PAYLOAD_DIR` names a folder holding the built programs; each one found becomes an
//! `include_bytes!` entry, so the setup is a single self-contained exe with no archive
//! format or decompressor to get wrong. Without the variable the list is empty and the
//! setup installs the programs sitting next to it (the zip release, or a dev build).

use std::path::{Path, PathBuf};

/// Everything ReUI ships.
pub const PROGRAMS: &[&str] = &["reui-hub.exe", "reui-island.exe", "reui-bridge.exe", "reui-settings.exe"];

/// ReUI doesn't run without these; `reui-settings.exe` is optional (the hub skips it).
pub const REQUIRED: &[&str] = &["reui-hub.exe", "reui-island.exe", "reui-bridge.exe"];

/// Rust source defining `EMBEDDED: &[(&str, &[u8])]`, and the files it reads (for
/// `rerun-if-changed`). Errors if a payload folder is given but a required program is missing.
pub fn generate(dir: Option<&Path>) -> Result<(String, Vec<PathBuf>), String> {
    let mut code = String::from("pub static EMBEDDED: &[(&str, &[u8])] = &[\n");
    let mut inputs = Vec::new();
    if let Some(dir) = dir {
        for name in PROGRAMS {
            let path = dir.join(name);
            if !path.is_file() {
                if REQUIRED.contains(name) {
                    return Err(format!("REUI_PAYLOAD_DIR {} has no {name}", dir.display()));
                }
                continue;
            }
            let absolute = std::fs::canonicalize(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            // `{:?}` of a str is a valid Rust string literal (escapes backslashes and quotes).
            code += &format!("    ({name:?}, include_bytes!({:?})),\n", absolute.to_string_lossy());
            inputs.push(absolute);
        }
    }
    code += "];\n";
    Ok((code, inputs))
}
