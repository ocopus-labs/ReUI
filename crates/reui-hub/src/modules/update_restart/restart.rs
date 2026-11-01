//! The two answers: restart now as Start → Update and restart does (pending updates are
//! installed on the way), or Windows' own Windows Update page, where the restart can be
//! scheduled or updates paused. Both are blocking calls: run them off the hub thread.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, LUID};
use windows::Win32::Security::{AdjustTokenPrivileges, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED, SE_SHUTDOWN_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY};
use windows::Win32::System::Shutdown::{InitiateShutdownW, SHTDN_REASON_FLAG_PLANNED, SHTDN_REASON_MAJOR_OPERATINGSYSTEM, SHTDN_REASON_MINOR_UPGRADE, SHUTDOWN_INSTALL_UPDATES, SHUTDOWN_RESTART};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// Restarts the PC, installing the updates that wait for it. Every interactive user holds
/// the shutdown privilege; it only has to be switched on for this process.
pub fn restart() {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token).is_err() {
            eprintln!("update-restart: can't open the process token");
            return;
        }
        let mut luid = LUID::default();
        if LookupPrivilegeValueW(PCWSTR::null(), SE_SHUTDOWN_NAME, &mut luid).is_ok() {
            let mut privileges = TOKEN_PRIVILEGES { PrivilegeCount: 1, ..Default::default() };
            privileges.Privileges[0].Luid = luid;
            privileges.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;
            _ = AdjustTokenPrivileges(token, false, Some(&privileges), 0, None, None);
        }
        _ = CloseHandle(token);
        let reason = SHTDN_REASON_MAJOR_OPERATINGSYSTEM | SHTDN_REASON_MINOR_UPGRADE | SHTDN_REASON_FLAG_PLANNED;
        let result = InitiateShutdownW(PCWSTR::null(), PCWSTR::null(), 0, SHUTDOWN_RESTART | SHUTDOWN_INSTALL_UPDATES, reason);
        if result != 0 {
            eprintln!("update-restart: Windows didn't restart (error {result})");
        }
    }
}

/// Settings → Windows Update.
pub fn open_settings() {
    let result = unsafe { ShellExecuteW(None, w!("open"), w!("ms-settings:windowsupdate"), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    if result.0 as isize <= 32 {
        eprintln!("update-restart: can't open Windows Update settings ({})", result.0 as isize);
    }
}
