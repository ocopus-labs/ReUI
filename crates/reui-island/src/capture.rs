//! Screen shares and recordings (docs/PRD-share-privacy.md): what other people see of the
//! island when you share your screen, record it, or take a screenshot. Windows draws a
//! window marked `WDA_EXCLUDEFROMCAPTURE` on your screen as usual and leaves it out of
//! every capture it makes (Teams, Zoom, OBS, Snipping Tool, PrintScreen), so the island
//! marks itself only while a card that may hold private text is in front. Nothing to watch
//! or poll: it's decided when the front activity changes. Detecting a share isn't needed,
//! and couldn't be relied on (Desktop Duplication leaves no trace; PRD privacy §6).

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE};

/// The `captures` setting.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Captures {
    /// Left out while a private card (the clipboard, an agent's request, a notification,
    /// the shelf) is in front; shown otherwise.
    #[default]
    HidePrivate,
    /// Never in a capture: only you see the island.
    HideIsland,
    /// Captured like any window.
    ShowAll,
}

impl Captures {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "private" => Some(Captures::HidePrivate),
            "island" => Some(Captures::HideIsland),
            "everything" => Some(Captures::ShowAll),
            _ => None,
        }
    }

    /// Whether the island is left out of captures while the front activity is (or isn't)
    /// private.
    pub fn excluded(self, front_private: bool) -> bool {
        match self {
            Captures::HidePrivate => front_private,
            Captures::HideIsland => true,
            Captures::ShowAll => false,
        }
    }
}

/// Applies `excluded` to the window. On Windows before 10 2004 the call fails and the island
/// stays captured: `WDA_MONITOR` would show a black box in its place instead, which tells
/// the people watching more than the pill does.
pub fn set_excluded(hwnd: HWND, excluded: bool) {
    let affinity = if excluded { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
    if let Err(e) = unsafe { SetWindowDisplayAffinity(hwnd, affinity) } {
        eprintln!("island: can't {} screen captures: {e}", if excluded { "leave" } else { "rejoin" });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_reads_its_three_values() {
        assert_eq!(Captures::parse("private"), Some(Captures::HidePrivate));
        assert_eq!(Captures::parse("island"), Some(Captures::HideIsland));
        assert_eq!(Captures::parse("everything"), Some(Captures::ShowAll));
        assert_eq!(Captures::parse("sometimes"), None);
        assert_eq!(Captures::default(), Captures::HidePrivate);
    }

    #[test]
    fn only_private_cards_leave_captures_by_default() {
        assert!(Captures::HidePrivate.excluded(true));
        assert!(!Captures::HidePrivate.excluded(false));
        assert!(Captures::HideIsland.excluded(false) && Captures::HideIsland.excluded(true));
        assert!(!Captures::ShowAll.excluded(true));
    }
}
