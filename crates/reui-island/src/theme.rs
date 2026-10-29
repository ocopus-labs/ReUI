//! What Windows says about colour: whether apps are in light mode (the Auto theme) and the
//! accent colour (the Windows accent). Read once when the look is built, and again only
//! when Windows broadcasts a change (`WM_SETTINGCHANGE` "ImmersiveColorSet",
//! `WM_DWMCOLORIZATIONCOLORCHANGED`; see `main.rs`); never polled.

use reui_design::Color;
use windows::core::HSTRING;
use windows::Win32::Foundation::LPARAM;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
const DWM: &str = r"Software\Microsoft\Windows\DWM";

fn dword(key: &str, value: &str) -> Option<u32> {
    let (mut data, mut len) = (0u32, 4u32);
    let error = unsafe { RegGetValueW(HKEY_CURRENT_USER, &HSTRING::from(key), &HSTRING::from(value), RRF_RT_REG_DWORD, None, Some((&mut data as *mut u32).cast()), Some(&mut len)) };
    error.is_ok().then_some(data)
}

/// A test sandbox (`REUI_CONFIG_DIR`) may stand in for Windows' app mode with an
/// `apps-theme` file there ("light" / "dark"), so Auto can be checked live (change the file,
/// send the island "ImmersiveColorSet") without touching the user's Windows settings.
fn forced() -> Option<bool> {
    let dir = std::env::var_os("REUI_CONFIG_DIR").filter(|d| !d.is_empty())?;
    match std::fs::read_to_string(std::path::Path::new(&dir).join("apps-theme")).ok()?.trim() {
        "light" => Some(true),
        "dark" => Some(false),
        _ => None,
    }
}

/// Windows' app mode is light (Settings > Personalization > Colors). Light when the value
/// is missing, as Windows itself treats it.
pub fn apps_light() -> bool {
    forced().unwrap_or_else(|| apps_light_from(dword(PERSONALIZE, "AppsUseLightTheme")))
}

fn apps_light_from(value: Option<u32>) -> bool {
    value.is_none_or(|v| v != 0)
}

/// The Windows accent colour, if it's set.
pub fn accent() -> Option<Color> {
    dword(DWM, "AccentColor").map(accent_from)
}

/// `AccentColor` is 0xAABBGGRR; the accent is always opaque.
fn accent_from(abgr: u32) -> Color {
    Color::from_rgb8((abgr & 0xFF) as u8, (abgr >> 8 & 0xFF) as u8, (abgr >> 16 & 0xFF) as u8)
}

/// Whether a `WM_SETTINGCHANGE` is about colours (app mode, accent): its `lParam` names
/// the area that changed.
pub fn is_color_change(lparam: LPARAM) -> bool {
    if lparam.0 == 0 {
        return false;
    }
    let text = unsafe { windows::core::PCWSTR(lparam.0 as *const u16).to_string() }.unwrap_or_default();
    text == "ImmersiveColorSet"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_values_read_as_windows_means_them() {
        assert!(apps_light_from(None), "missing: light, as Windows treats it");
        assert!(apps_light_from(Some(1)));
        assert!(!apps_light_from(Some(0)));
        assert_eq!(accent_from(0xFF_D7_78_00).to_rgba8(), [0x00, 0x78, 0xD7, 0xFF]);
    }

    #[test]
    fn only_colour_broadcasts_count() {
        let name: Vec<u16> = "ImmersiveColorSet\0".encode_utf16().collect();
        assert!(is_color_change(LPARAM(name.as_ptr() as isize)));
        let other: Vec<u16> = "Policy\0".encode_utf16().collect();
        assert!(!is_color_change(LPARAM(other.as_ptr() as isize)));
        assert!(!is_color_change(LPARAM(0)));
    }
}
