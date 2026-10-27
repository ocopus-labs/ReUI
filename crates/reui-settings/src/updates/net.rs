//! HTTPS GETs with WinHTTP (the system's TLS, proxy settings and redirects; nothing to
//! ship), and SHA-256 with CNG. Blocking: callers run these on their own thread.

use std::ffi::c_void;
use std::io::Write;
use std::path::Path;

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Networking::WinHttp::*;
use windows::Win32::Security::Cryptography::{BCryptCreateHash, BCryptDestroyHash, BCryptFinishHash, BCryptHashData, BCRYPT_HASH_HANDLE, BCRYPT_SHA256_ALG_HANDLE};

/// Whole-response cap for API answers and checksum files.
const SMALL_MAX: usize = 1 << 20;
/// An installer bigger than this isn't ours.
const DOWNLOAD_MAX: u64 = 512 << 20;
const TIMEOUT_MS: i32 = 30_000;

/// Closes a WinHTTP handle when dropped.
struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { _ = WinHttpCloseHandle(self.0) };
        }
    }
}

fn handle(raw: *mut c_void, what: &str) -> Result<Handle, String> {
    if raw.is_null() {
        Err(format!("{what} failed ({})", windows::core::Error::from_thread().message()))
    } else {
        Ok(Handle(raw))
    }
}

/// `https://host/path?query` → (host, path); only HTTPS is accepted.
pub fn split_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("https://")?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    (!host.is_empty() && !host.contains(['@', ':'])).then_some((host, path))
}

/// An open request with its response headers read. `session` and `connect` live as long
/// as the request.
struct Response {
    request: Handle,
    _connect: Handle,
    _session: Handle,
}

fn request(url: &str, accept: Option<&str>) -> Result<Response, String> {
    let (host, path) = split_url(url).ok_or_else(|| format!("not an HTTPS link: {url}"))?;
    let agent = HSTRING::from(format!("ReUI/{}", crate::pages::VERSION));
    unsafe {
        let session = handle(WinHttpOpen(&agent, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0), "Starting the connection")?;
        _ = WinHttpSetTimeouts(session.0, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS);
        let connect = handle(WinHttpConnect(session.0, &HSTRING::from(host), INTERNET_DEFAULT_HTTPS_PORT, 0), "Connecting")?;
        let request = handle(WinHttpOpenRequest(connect.0, &HSTRING::from("GET"), &HSTRING::from(path), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), WINHTTP_FLAG_SECURE), "Opening the request")?;
        let headers: Vec<u16> = accept.map(|a| format!("Accept: {a}\r\n")).unwrap_or_default().encode_utf16().collect();
        WinHttpSendRequest(request.0, (!headers.is_empty()).then_some(&headers[..]), None, 0, 0, 0).map_err(|e| format!("Couldn't reach {host} ({})", e.message()))?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(|e| format!("No answer from {host} ({})", e.message()))?;
        let mut status = 0u32;
        let mut len = size_of::<u32>() as u32;
        let mut index = 0u32;
        WinHttpQueryHeaders(request.0, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, PCWSTR::null(), Some((&mut status as *mut u32).cast()), &mut len, &mut index).map_err(|e| e.message())?;
        match status {
            200 => Ok(Response { request, _connect: connect, _session: session }),
            403 | 429 => Err(format!("{host} asks to try again later")),
            404 => Err(format!("Not found on {host}")),
            other => Err(format!("{host} answered {other}")),
        }
    }
}

/// Reads the body in chunks; `sink` gets each one. Stops past `max` bytes.
fn read_body(response: &Response, max: u64, mut sink: impl FnMut(&[u8]) -> Result<(), String>) -> Result<u64, String> {
    let mut total = 0u64;
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let mut read = 0u32;
        unsafe { WinHttpReadData(response.request.0, buffer.as_mut_ptr().cast(), buffer.len() as u32, &mut read) }.map_err(|e| format!("The download stopped ({})", e.message()))?;
        if read == 0 {
            return Ok(total);
        }
        total += u64::from(read);
        if total > max {
            return Err("The download is larger than expected".into());
        }
        sink(&buffer[..read as usize])?;
    }
}

/// A small response (an API answer, a checksum file) as text.
pub fn get_text(url: &str, accept: Option<&str>) -> Result<String, String> {
    let response = request(url, accept)?;
    let mut body = Vec::new();
    read_body(&response, SMALL_MAX as u64, |chunk| {
        body.extend_from_slice(chunk);
        Ok(())
    })?;
    String::from_utf8(body).map_err(|_| "The answer isn't text".to_string())
}

/// Downloads `url` to `path`, calling `progress(bytes so far)` as it goes; returns the size
/// and the SHA-256 (lower-case hex) of what was written.
pub fn download(url: &str, path: &Path, mut progress: impl FnMut(u64)) -> Result<(u64, String), String> {
    let response = request(url, Some("application/octet-stream"))?;
    let mut file = std::fs::File::create(path).map_err(|e| format!("Can't write {}: {e}", path.display()))?;
    let mut hash = Sha256::new()?;
    let mut done = 0u64;
    let size = read_body(&response, DOWNLOAD_MAX, |chunk| {
        file.write_all(chunk).map_err(|e| format!("Can't write {}: {e}", path.display()))?;
        hash.update(chunk)?;
        done += chunk.len() as u64;
        progress(done);
        Ok(())
    })?;
    file.sync_all().map_err(|e| e.to_string())?;
    Ok((size, hash.finish()?))
}

/// SHA-256 with CNG's built-in algorithm handle.
pub struct Sha256(BCRYPT_HASH_HANDLE);

impl Sha256 {
    pub fn new() -> Result<Self, String> {
        let mut h = BCRYPT_HASH_HANDLE::default();
        let status = unsafe { BCryptCreateHash(BCRYPT_SHA256_ALG_HANDLE, &mut h, None, None, 0) };
        if status.is_err() {
            return Err(format!("SHA-256 isn't available ({:#x})", status.0));
        }
        Ok(Self(h))
    }

    pub fn update(&mut self, data: &[u8]) -> Result<(), String> {
        let status = unsafe { BCryptHashData(self.0, data, 0) };
        if status.is_err() {
            return Err(format!("Hashing failed ({:#x})", status.0));
        }
        Ok(())
    }

    pub fn finish(self) -> Result<String, String> {
        let mut out = [0u8; 32];
        let status = unsafe { BCryptFinishHash(self.0, &mut out, 0) };
        if status.is_err() {
            return Err(format!("Hashing failed ({:#x})", status.0));
        }
        Ok(out.iter().map(|b| format!("{b:02x}")).collect())
    }
}

impl Drop for Sha256 {
    fn drop(&mut self) {
        unsafe { _ = BCryptDestroyHash(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_links_are_split() {
        assert_eq!(split_url("https://api.github.com/repos/o/r/releases/latest"), Some(("api.github.com", "/repos/o/r/releases/latest")));
        assert_eq!(split_url("https://example.com"), Some(("example.com", "/")));
        assert_eq!(split_url("http://example.com/x"), None);
        assert_eq!(split_url("https://user@evil.com/x"), None);
        assert_eq!(split_url("https:///x"), None);
    }

    #[test]
    fn sha256_matches_known_vectors() {
        let hex = |data: &[u8]| {
            let mut h = Sha256::new().unwrap();
            h.update(data).unwrap();
            h.finish().unwrap()
        };
        assert_eq!(hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
