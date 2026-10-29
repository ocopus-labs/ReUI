//! An MCP server's link (`Elicitation` in URL mode, PRD agent moments §5.6) opened in the
//! default browser when you press Open link. Claude doesn't open it itself once the hook
//! accepts, so the island must.

/// Whether a link may go to the browser: only http(s), so a server can't have the island
/// run a file or another protocol's handler. Pure.
pub(super) fn is_web(url: &str) -> bool {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://")) && !url.contains(char::is_whitespace)
}

/// The link's host, for the card's detail.
pub(super) fn host(url: &str) -> Option<String> {
    url.split("://").nth(1).and_then(|r| r.split(['/', '?', '#']).next()).filter(|h| !h.is_empty()).map(str::to_owned)
}

/// Opens it. Blocking (ShellExecute); best effort.
pub(super) fn open(url: &str) {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    if !is_web(url) {
        println!("agent: not opening a link that isn't http(s)");
        return;
    }
    let result = unsafe { ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(url.trim()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    if result.0 as isize <= 32 {
        eprintln!("agent: can't open the link ({})", result.0 as isize);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_links_open() {
        assert!(is_web("https://auth.example.com/login?x=1"));
        assert!(is_web(" HTTP://localhost:3000/cb "));
        assert!(!is_web("file:///C:/Windows/System32/calc.exe"));
        assert!(!is_web("C:\\evil.exe"));
        assert!(!is_web("ms-settings:display"));
        assert!(!is_web("https://a.com/x y"));
    }

    #[test]
    fn the_host_is_what_the_card_names() {
        assert_eq!(host("https://auth.example.com/login?x=1").as_deref(), Some("auth.example.com"));
        assert_eq!(host("https://auth.example.com?x").as_deref(), Some("auth.example.com"));
        assert_eq!(host("nonsense"), None);
    }
}
