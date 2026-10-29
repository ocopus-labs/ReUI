//! Secrets at rest (PRD §11): a secret ICS link is a password to the calendar, and a
//! cached feed is the calendar itself, so both are encrypted with DPAPI for the current
//! user (`CryptProtectData`): readable by this user on this PC only, by no other account
//! and not from a copied file. Settings encrypts links the same way before it writes them.

use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};

/// What a protected value starts with in a settings file: `dpapi:` and the blob in hex.
pub const PREFIX: &str = "dpapi:";

pub fn protect(data: &[u8]) -> Option<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe { CryptProtectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out).ok()? };
    Some(take(out))
}

pub fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe { CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out).ok()? };
    Some(take(out))
}

fn take(blob: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    let bytes = unsafe { std::slice::from_raw_parts(blob.pbData, blob.cbData as usize) }.to_vec();
    unsafe { _ = LocalFree(Some(HLOCAL(blob.pbData.cast()))) };
    bytes
}

/// A secret as a settings value: `dpapi:` + hex.
pub fn seal(text: &str) -> Option<String> {
    protect(text.as_bytes()).map(|b| format!("{PREFIX}{}", hex(&b)))
}

/// The secret in a settings value; `None` if it isn't one, or isn't this user's.
pub fn open(value: &str) -> Option<String> {
    let bytes = unhex(value.strip_prefix(PREFIX)?)?;
    String::from_utf8(unprotect(&bytes)?).ok()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sealed_secret_opens_only_as_itself() {
        let sealed = seal("https://calendar.example.com/private-abc/basic.ics").unwrap();
        assert!(sealed.starts_with(PREFIX) && !sealed.contains("calendar.example.com"));
        assert_eq!(open(&sealed).as_deref(), Some("https://calendar.example.com/private-abc/basic.ics"));
        assert_eq!(open("https://plain.example.com/x.ics"), None);
        assert_eq!(open("dpapi:00ff"), None, "not a blob");
        assert_eq!(unhex("0a0B"), Some(vec![10, 11]));
    }
}
