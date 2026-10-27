//! The programs to install: embedded at build time (the single-file `ReUI-Setup-<v>.exe`,
//! see `payload_gen.rs`) or, when nothing is embedded, the ones next to the setup exe (the
//! zip release, or a dev `target\release`).

use std::borrow::Cow;
use std::path::Path;

use crate::payload_gen::{PROGRAMS, REQUIRED};

mod generated {
    include!(concat!(env!("OUT_DIR"), "/payload.rs"));
}

#[derive(Debug, Clone, PartialEq)]
pub struct File {
    pub name: &'static str,
    pub bytes: Cow<'static, [u8]>,
}

/// What `build.rs` embedded (empty unless built with `REUI_PAYLOAD_DIR`).
pub fn embedded() -> Vec<File> {
    from_entries(generated::EMBEDDED)
}

fn from_entries(entries: &'static [(&'static str, &'static [u8])]) -> Vec<File> {
    entries.iter().map(|&(name, bytes)| File { name, bytes: Cow::Borrowed(bytes) }).collect()
}

/// The programs found in `dir`.
pub fn from_dir(dir: &Path) -> std::io::Result<Vec<File>> {
    let mut files = Vec::new();
    for &name in PROGRAMS {
        let path = dir.join(name);
        if path.is_file() {
            files.push(File { name, bytes: Cow::Owned(std::fs::read(&path)?) });
        }
    }
    Ok(files)
}

/// The embedded payload if there is one, else the programs next to the setup.
pub fn resolve(setup_dir: &Path) -> Result<Vec<File>, String> {
    let files = match embedded() {
        files if !files.is_empty() => files,
        _ => from_dir(setup_dir).map_err(|e| format!("Couldn't read ReUI's files from {}: {e}", setup_dir.display()))?,
    };
    check(&files)?;
    Ok(files)
}

pub fn check(files: &[File]) -> Result<(), String> {
    let missing: Vec<_> = REQUIRED.iter().filter(|r| !files.iter().any(|f| f.name == **r)).copied().collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("This setup is incomplete: {} missing. Download it again from ReUI's releases page.", missing.join(", ")))
    }
}

/// Writes every file into `dir` (`--extract`, used to verify a packaged setup).
pub fn extract(files: &[File], dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for file in files {
        std::fs::write(dir.join(file.name), &file.bytes)?;
    }
    Ok(())
}

pub fn total_size(files: &[File]) -> u64 {
    files.iter().map(|f| f.bytes.len() as u64).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload_gen::generate;

    fn temp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("reui-setup-payload-{tag}-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn dir_payload_round_trips_through_extract() {
        let src = temp("src");
        for (i, name) in PROGRAMS.iter().enumerate() {
            std::fs::write(src.join(name), vec![i as u8; 1000 + i]).unwrap();
        }
        std::fs::write(src.join("unrelated.txt"), b"not shipped").unwrap();
        let files = from_dir(&src).unwrap();
        assert_eq!(files.len(), PROGRAMS.len());
        check(&files).unwrap();

        let out = temp("out");
        extract(&files, &out).unwrap();
        for name in PROGRAMS {
            assert_eq!(std::fs::read(out.join(name)).unwrap(), std::fs::read(src.join(name)).unwrap());
        }
        assert!(!out.join("unrelated.txt").exists());
        assert_eq!(total_size(&files), (0..PROGRAMS.len()).map(|i| 1000 + i as u64).sum::<u64>());
        _ = std::fs::remove_dir_all(src);
        _ = std::fs::remove_dir_all(out);
    }

    #[test]
    fn missing_required_programs_are_named() {
        let files = vec![File { name: "reui-hub.exe", bytes: Cow::Borrowed(b"x") }];
        let err = check(&files).unwrap_err();
        assert!(err.contains("reui-island.exe") && err.contains("reui-bridge.exe"), "{err}");
        // Settings is optional.
        let files: Vec<File> = REQUIRED.iter().map(|&name| File { name, bytes: Cow::Borrowed(b"x") }).collect();
        check(&files).unwrap();
    }

    #[test]
    fn generated_code_embeds_what_the_folder_has() {
        assert_eq!(generate(None).unwrap().0, "pub static EMBEDDED: &[(&str, &[u8])] = &[\n];\n");

        let dir = temp("gen");
        for name in REQUIRED {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let (code, inputs) = generate(Some(&dir)).unwrap();
        assert_eq!(inputs.len(), REQUIRED.len());
        for name in REQUIRED {
            assert!(code.contains(&format!("(\"{name}\", include_bytes!(")), "{code}");
        }
        assert!(!code.contains("reui-settings.exe"));
        // Backslashes in the absolute paths are escaped, so the literal compiles.
        assert!(code.contains("\\\\"), "{code}");

        std::fs::remove_file(dir.join("reui-island.exe")).unwrap();
        assert!(generate(Some(&dir)).unwrap_err().contains("reui-island.exe"));
        _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn embedded_entries_become_files() {
        static ENTRIES: &[(&str, &[u8])] = &[("reui-hub.exe", b"hub"), ("reui-island.exe", b"island")];
        let files = from_entries(ENTRIES);
        assert_eq!(files[1], File { name: "reui-island.exe", bytes: Cow::Borrowed(b"island") });
    }
}
