//! The ReUI logo as a Windows icon resource, for the programs' build scripts:
//! `reui_design::icon::embed()` turns `design/logo/reui.ico` (drawn by `tools/logo.ps1`)
//! into a compiled `.res` and hands it to the linker, so Explorer, the taskbar, Start and
//! Apps & features show the logo, and the programs can load it (resource 1) for their
//! windows and the tray. Writing the `.res` here needs no resource compiler or extra crate.

use std::path::{Path, PathBuf};

/// The icon group's resource ID (`MAKEINTRESOURCE(1)`).
pub const ICON_ID: u16 = 1;

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const LANG_EN_US: u16 = 0x0409;

/// The logo's `.ico`.
pub fn ico_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../design/logo/reui.ico")
}

/// For a `build.rs`: links the logo into the crate's binaries (MSVC only).
pub fn embed() {
    let ico = ico_path();
    println!("cargo:rerun-if-changed={}", ico.display());
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }
    let bytes = std::fs::read(&ico).unwrap_or_else(|e| panic!("can't read {}: {e}", ico.display()));
    let res = resource(&bytes).unwrap_or_else(|e| panic!("{}: {e}", ico.display()));
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR")).join("reui-icon.res");
    std::fs::write(&out, res).expect("write reui-icon.res");
    println!("cargo:rustc-link-arg-bins={}", out.display());
}

/// One image of an `.ico`: its directory entry (without the offset) and its bytes.
struct Image<'a> {
    entry: [u8; 12],
    data: &'a [u8],
}

fn images(ico: &[u8]) -> Result<Vec<Image<'_>>, String> {
    let word = |at: usize| ico.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let dword = |at: usize| ico.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    if word(0) != Some(0) || word(2) != Some(1) {
        return Err("not an icon file".into());
    }
    let count = usize::from(word(4).ok_or("truncated")?);
    (0..count)
        .map(|i| {
            let at = 6 + 16 * i;
            let head = ico.get(at..at + 12).ok_or("truncated directory")?;
            let (len, offset) = (dword(at + 8).ok_or("truncated")?, dword(at + 12).ok_or("truncated")?);
            let data = ico.get(offset..offset + len).ok_or("an image runs past the end")?;
            Ok(Image { entry: head.try_into().unwrap(), data })
        })
        .collect()
}

/// A compiled resource file (`.res`) holding the icon's images and their group.
pub fn resource(ico: &[u8]) -> Result<Vec<u8>, String> {
    let images = images(ico)?;
    if images.is_empty() {
        return Err("the icon has no images".into());
    }
    let mut res = Vec::new();
    // A .res file starts with an empty entry.
    entry(&mut res, 0, 0, &[]);
    let mut group = vec![0, 0, 1, 0];
    group.extend_from_slice(&(images.len() as u16).to_le_bytes());
    for (i, image) in images.iter().enumerate() {
        let id = i as u16 + 1;
        entry(&mut res, RT_ICON, id, image.data);
        // GRPICONDIRENTRY: the ICONDIRENTRY's first 12 bytes, then the image's ID.
        group.extend_from_slice(&image.entry);
        group.extend_from_slice(&id.to_le_bytes());
    }
    entry(&mut res, RT_GROUP_ICON, ICON_ID, &group);
    Ok(res)
}

/// One resource: a 32-byte header (numeric type and name), the data, padded to 4 bytes.
fn entry(res: &mut Vec<u8>, kind: u16, name: u16, data: &[u8]) {
    let (flags, lang) = match kind {
        0 => (0u16, 0u16),
        RT_GROUP_ICON => (0x1030, LANG_EN_US),
        _ => (0x1010, LANG_EN_US),
    };
    res.extend_from_slice(&(data.len() as u32).to_le_bytes());
    res.extend_from_slice(&32u32.to_le_bytes());
    for v in [0xFFFF, kind, 0xFFFF, name] {
        res.extend_from_slice(&v.to_le_bytes());
    }
    res.extend_from_slice(&0u32.to_le_bytes()); // data version
    res.extend_from_slice(&flags.to_le_bytes());
    res.extend_from_slice(&lang.to_le_bytes());
    res.extend_from_slice(&0u32.to_le_bytes()); // version
    res.extend_from_slice(&0u32.to_le_bytes()); // characteristics
    res.extend_from_slice(data);
    res.resize(res.len().next_multiple_of(4), 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_logo_becomes_one_icon_per_size_and_a_group() {
        let ico = std::fs::read(ico_path()).expect("design/logo/reui.ico (tools/logo.ps1)");
        let images = images(&ico).unwrap();
        assert!(images.len() >= 4, "{} sizes", images.len());
        let res = resource(&ico).unwrap();
        assert_eq!(res.len() % 4, 0);
        // The empty entry, then RT_ICON 1.
        assert_eq!(&res[..8], &[0, 0, 0, 0, 32, 0, 0, 0]);
        assert_eq!(u16::from_le_bytes([res[32 + 10], res[32 + 11]]), RT_ICON);
        assert!(resource(b"nope").is_err());
    }
}
