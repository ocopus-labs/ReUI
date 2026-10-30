//! Downloading a feed (PRD §6.5): a conditional GET (`If-None-Match` / `If-Modified-Since`,
//! so an unchanged calendar costs a 304), refused past 20 MB or when what came back isn't
//! a calendar (a login or error page); and the last good copy, DPAPI-encrypted on disk, so
//! the schedule is right offline and from the first second after a restart. Errors never
//! carry the link: reqwest's are stripped of their URL before anything sees them.

use std::path::PathBuf;
use std::time::Duration;

use reqwest::header::{ACCEPT, CACHE_CONTROL, CONTENT_TYPE, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED, PRAGMA, USER_AGENT};
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};

use super::dpapi;

const MAX_BYTES: usize = 20 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);
/// How often a feed is asked again when all is well.
pub const REFRESH_MS: u64 = 15 * 60_000;

/// Who we say we are everywhere but Outlook: honest, and what a server's logs should show.
const OWN_AGENT: &str = concat!("ReUI/", env!("CARGO_PKG_VERSION"));
/// Since August 2025 Outlook answers a published calendar asked for by a client it doesn't
/// recognise with a redirect to an error page (`OwaBasicUnsupportedException`); a current
/// browser's name gets the feed (nextcloud/server#54799). Only Outlook gets it: elsewhere a
/// browser's name can get a web page for the calendar instead of the calendar itself.
const BROWSER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";

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
    Client::builder().timeout(TIMEOUT).user_agent(OWN_AGENT).build().map_err(|e| eprintln!("calendar: no HTTP client: {}", e.without_url())).ok()
}

/// The name to ask `host` with.
fn agent_for(host: &str) -> &'static str {
    let host = host.to_ascii_lowercase();
    let outlook = host.starts_with("outlook.") || host == "outlook.com" || host.ends_with(".outlook.com");
    if outlook { BROWSER_AGENT } else { OWN_AGENT }
}

/// One GET of `url`, conditional on `known`.
pub async fn get(client: &Client, url: &str, known: &Validators) -> Result<Fetched, String> {
    let asked = Url::parse(url).map_err(|_| "isn't a link".to_string())?;
    let mut request = client
        .get(asked.clone())
        .header(USER_AGENT, agent_for(asked.host_str().unwrap_or("")))
        .header(ACCEPT, "text/calendar, */*;q=0.5")
        // A corporate proxy may otherwise answer with its own old copy.
        .header(CACHE_CONTROL, "no-cache")
        .header(PRAGMA, "no-cache");
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
    let html_type = header(CONTENT_TYPE).is_some_and(|t| t.to_ascii_lowercase().contains("html"));
    let redirected = response.url() != &asked;
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(describe)? {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_BYTES {
            return Err("over 20 MB".into());
        }
    }
    let text = String::from_utf8_lossy(&body).into_owned();
    match refusal(&text, html_type, redirected) {
        Some(why) => Err(why.into()),
        None => Ok(Fetched::Body { text, validators }),
    }
}

/// Why a successful answer isn't a calendar, if it isn't: sent on to a web page (an
/// expired link, a sign-in, Outlook refusing the client) reads differently from a link to
/// something else altogether.
fn refusal(text: &str, html_type: bool, redirected: bool) -> Option<&'static str> {
    let head = text.get(..4096.min(text.len())).unwrap_or(text).to_ascii_uppercase();
    if head.contains("BEGIN:VCALENDAR") {
        return None;
    }
    let html = html_type || head.trim_start().starts_with("<!DOCTYPE HTML") || head.contains("<HTML");
    Some(match (html, redirected) {
        (true, true) => "was sent to a web page",
        (true, false) => "answered with a web page",
        (false, _) => "isn't a calendar",
    })
}

/// An HTTP date (`Last-Modified`) as Unix ms.
pub fn http_date_ms(date: &str) -> Option<u64> {
    let at = chrono::DateTime::parse_from_rfc2822(date.trim()).ok()?;
    u64::try_from(at.timestamp_millis()).ok()
}

/// What went wrong, without the URL.
fn describe(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "timed out".into()
    } else if e.is_connect() {
        "can't connect".into()
    } else if e.is_redirect() {
        "redirects too often".into()
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
    /// When this body first came (Unix ms), so "changed 3 h ago" survives a restart.
    #[serde(default)]
    pub changed: Option<u64>,
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

    #[test]
    fn only_outlook_is_asked_as_a_browser() {
        for host in ["outlook.office365.com", "outlook.live.com", "OUTLOOK.OFFICE.COM", "outlook.office365.us", "outlook.com"] {
            assert_eq!(agent_for(host), BROWSER_AGENT, "{host}");
        }
        for host in ["calendar.google.com", "p01-caldav.icloud.com", "notoutlook.com", "127.0.0.1", ""] {
            assert_eq!(agent_for(host), OWN_AGENT, "{host}");
        }
    }

    #[test]
    fn a_web_page_is_told_apart_from_a_calendar() {
        assert_eq!(refusal("BEGIN:VCALENDAR\r\nEND:VCALENDAR", false, true), None);
        assert_eq!(refusal("\r\n<!DOCTYPE html><html><body>OwaBasicUnsupportedException", false, true), Some("was sent to a web page"));
        assert_eq!(refusal("<p>Sign in</p>", true, false), Some("answered with a web page"));
        assert_eq!(refusal("{\"error\":1}", false, false), Some("isn't a calendar"));
    }

    #[test]
    fn last_modified_reads_as_unix_ms() {
        assert_eq!(http_date_ms("Wed, 21 Oct 2015 07:28:00 GMT"), Some(1_445_412_480_000));
        assert_eq!(http_date_ms("yesterday"), None);
    }
}
