//! Downloading a feed (PRD §6.5): a conditional GET (`If-None-Match` / `If-Modified-Since`,
//! so an unchanged calendar costs a 304), refused past 20 MB or when what came back isn't
//! a calendar (a login page); and the last good copy, DPAPI-encrypted on disk, so the
//! schedule is right offline and from the first second after a restart. Errors never
//! carry the link: reqwest's are stripped of their URL before anything sees them.

use std::path::PathBuf;
use std::time::Duration;

use reqwest::header::{ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};

use super::dpapi;

const MAX_BYTES: usize = 20 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);
/// How often a feed is asked again when all is well.
pub const REFRESH_MS: u64 = 15 * 60_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Validators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

#[derive(Debug)]
pub enum Fetched {
    NotModified,
    Body { text: String, validators: Validators },
}

pub fn client() -> Option<Client> {
    Client::builder().timeout(TIMEOUT).user_agent(concat!("ReUI/", env!("CARGO_PKG_VERSION"))).build().map_err(|e| eprintln!("calendar: no HTTP client: {}", e.without_url())).ok()
}

/// One GET of `url`, conditional on `known`.
pub async fn get(client: &Client, url: &str, known: &Validators) -> Result<Fetched, String> {
    let mut request = client.get(url);
    if let Some(etag) = &known.etag {
        request = request.header(IF_NONE_MATCH, etag);
    }
    if let Some(lm) = &known.last_modified {
        request = request.header(IF_MODIFIED_SINCE, lm);
    }
    let mut response = request.send().await.map_err(describe)?;
    match response.status() {
        StatusCode::NOT_MODIFIED => return Ok(Fetched::NotModified),
        s if !s.is_success() => return Err(format!("answered {}", s.as_u16())),
        _ => {}
    }
    if response.content_length().is_some_and(|n| n as usize > MAX_BYTES) {
        return Err("over 20 MB".into());
    }
    let header = |name| response.headers().get(name).and_then(|v: &reqwest::header::HeaderValue| v.to_str().ok()).map(str::to_string);
    let validators = Validators { etag: header(ETAG), last_modified: header(LAST_MODIFIED) };
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(describe)? {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_BYTES {
            return Err("over 20 MB".into());
        }
    }
    let text = String::from_utf8_lossy(&body).into_owned();
    if !text.get(..4096.min(text.len())).unwrap_or(&text).to_ascii_uppercase().contains("BEGIN:VCALENDAR") {
        return Err("isn't a calendar".into());
    }
    Ok(Fetched::Body { text, validators })
}

/// What went wrong, without the URL.
fn describe(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "timed out".into()
    } else if e.is_connect() {
        "can't connect".into()
    } else {
        e.without_url().to_string()
    }
}

/// When to ask again after `failures` failures in a row: soon at first, then the usual
/// pace (1, 5, 15 min).
pub fn retry_after(failures: u32) -> u64 {
    match failures {
        0 => REFRESH_MS,
        1 => 60_000,
        2 => 5 * 60_000,
        _ => REFRESH_MS,
    }
}

/// The last good copy of a feed.
#[derive(Debug, Serialize, Deserialize)]
pub struct Cached {
    pub validators: Validators,
    pub text: String,
}

fn cache_path(id: &str) -> PathBuf {
    let safe: String = id.chars().filter(char::is_ascii_alphanumeric).collect();
    crate::paths::local().join("calendar").join(format!("{safe}.cache"))
}

/// Blocking (disk, DPAPI).
pub fn load_cache(id: &str) -> Option<Cached> {
    let bytes = std::fs::read(cache_path(id)).ok()?;
    serde_json::from_slice(&dpapi::unprotect(&bytes)?).ok()
}

/// Blocking (disk, DPAPI).
pub fn save_cache(id: &str, cached: &Cached) {
    let path = cache_path(id);
    let Some(sealed) = serde_json::to_vec(cached).ok().and_then(|j| dpapi::protect(&j)) else { return };
    if let Some(dir) = path.parent() {
        _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("tmp");
    if let Err(e) = std::fs::write(&tmp, sealed).and_then(|_| std::fs::rename(&tmp, &path)) {
        eprintln!("calendar: can't save a feed's copy: {e}");
    }
}

/// A removed source's copy goes with it.
pub fn forget_cache(id: &str) {
    _ = std::fs::remove_file(cache_path(id));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_back_off_then_settle() {
        assert_eq!([0, 1, 2, 3, 9].map(retry_after), [REFRESH_MS, 60_000, 300_000, REFRESH_MS, REFRESH_MS]);
        assert_eq!(cache_path("../x").file_name().unwrap(), "x.cache", "an id can't leave the folder");
    }
}
