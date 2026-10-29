//! One hub per user. A second launch (the Start menu entry while ReUI runs, or a second
//! install) opens Settings instead and exits.

use std::path::Path;

use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, LocalFree, ERROR_ALREADY_EXISTS, HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, OpenProcessToken};

/// `Local\` scopes it to this session; the SID (or name) keeps users apart on shared
/// sessions such as RunAs. Backslashes aren't allowed after the namespace.
pub fn mutex_name(user: &str) -> String {
    let user: String = user.chars().map(|c| if c == '\\' || c.is_control() { '_' } else { c }).collect();
    format!(r"Local\ReUI-hub-{user}")
}

fn user_sid() -> Option<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
        let mut len = 0;
        _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        let got = GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr().cast()), len, &mut len);
        _ = CloseHandle(token);
        got.ok()?;
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut text = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut text).ok()?;
        let sid = text.to_string().ok();
        _ = LocalFree(Some(HLOCAL(text.0.cast())));
        sid
    }
}

/// Held for the hub's lifetime (the handle closes when the process exits).
pub struct Instance(#[cfg_attr(not(test), allow(dead_code))] HANDLE);

/// `Some` if this is the only hub for the user; `None` if one is already running.
pub fn acquire() -> Option<Instance> {
    let user = user_sid().or_else(|| std::env::var("USERNAME").ok()).unwrap_or_else(|| "default".into());
    // A test hub (`REUI_CONFIG_DIR`) runs beside the real one, and beside other test hubs.
    acquire_named(&format!("{}{}", mutex_name(&user), reui_protocol::sandbox_suffix()))
}

fn acquire_named(name: &str) -> Option<Instance> {
    let name = HSTRING::from(name);
    unsafe {
        // If the mutex can't be created at all, run anyway rather than never start.
        let Ok(handle) = CreateMutexW(None, false, &name) else { return Some(Instance(HANDLE::default())) };
        if GetLastError() == ERROR_ALREADY_EXISTS {
            _ = CloseHandle(handle);
            return None;
        }
        Some(Instance(handle))
    }
}

/// Opens Settings (`reui-settings.exe` next to the hub), if it's there.
pub fn open_settings(hub_exe: &Path) {
    let settings = hub_exe.with_file_name("reui-settings.exe");
    if settings.is_file() {
        if let Err(e) = std::process::Command::new(&settings).spawn() {
            eprintln!("hub: couldn't open {}: {e}", settings.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutex_names() {
        assert_eq!(mutex_name("S-1-5-21-1-2-3-1001"), r"Local\ReUI-hub-S-1-5-21-1-2-3-1001");
        assert_eq!(mutex_name(r"DOMAIN\rohit"), r"Local\ReUI-hub-DOMAIN_rohit");
        assert_ne!(mutex_name("S-1-5-21-1"), mutex_name("S-1-5-21-2"));
    }

    #[test]
    fn second_acquire_fails_until_the_first_is_gone() {
        let name = format!(r"Local\ReUI-hub-test-{}", std::process::id());
        let first = acquire_named(&name).expect("first");
        assert!(acquire_named(&name).is_none());
        unsafe { _ = CloseHandle(first.0) };
        assert!(acquire_named(&name).is_some());
    }

    #[test]
    fn current_user_has_a_sid() {
        let sid = user_sid().expect("a SID");
        assert!(sid.starts_with("S-1-"), "{sid}");
        assert!(!mutex_name(&sid)[r"Local\".len()..].contains('\\'));
    }
}
