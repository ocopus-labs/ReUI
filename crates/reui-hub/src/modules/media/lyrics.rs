//! Synced lyrics from LRCLIB (lrclib.net: free, no key), only when the user turned them on.
//!
//! An exact lookup by track, artist, album and duration first (the server allows ±2 s), then
//! a search that takes the closest duration within ±5 s. Titles are cleaned first, since
//! players report "Song (Remastered 2011)" and YouTube "Artist - Song (Official Video)".
//! Answers, including "no lyrics", are cached on disk so a track is asked about once; a
//! 429/503 backs off for a while instead of retrying on every track change.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use reqwest::{Client, StatusCode, Url};
use reui_protocol::LyricLine;
use serde_json::{json, Value};

const API: &str = "https://lrclib.net/api";
const TIMEOUT: Duration = Duration::from_secs(8);
/// "No lyrics" is asked again after this; found lyrics don't change.
const NEGATIVE_TTL_SECS: u64 = 7 * 24 * 3600;
const EXACT_SLACK_S: f64 = 2.0;
const SEARCH_SLACK_S: f64 = 5.0;
const MAX_BODY: usize = 2 * 1024 * 1024;

/// What we ask LRCLIB about, already cleaned.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_s: u64,
}

impl Query {
    /// From what the player reports. `video_site`: the source is a browser, whose titles are
    /// YouTube-like ("Artist - Song") and whose "artist" is often a channel. `None` when
    /// there's not enough to ask about.
    pub fn new(title: &str, artist: &str, album: &str, length_ms: u64, video_site: bool) -> Option<Self> {
        let mut title = clean_title(title);
        let mut artist = clean_artist(artist);
        if let Some((left, right)) = split_artist_title(&title) {
            let channel = artist.is_empty() || is_channel(&artist) || left.eq_ignore_ascii_case(&artist);
            if video_site || channel {
                (artist, title) = (clean_artist(left), clean_title(right));
            }
        }
        let duration_s = (length_ms + 500) / 1000;
        if title.is_empty() || artist.is_empty() || duration_s == 0 {
            return None;
        }
        let album = if album.trim().eq_ignore_ascii_case(title.trim()) { String::new() } else { album.trim().to_string() };
        Some(Self { title, artist, album, duration_s })
    }

    fn cache_name(&self) -> String {
        let key = format!("{}\u{1}{}\u{1}{}\u{1}{}", self.title.to_lowercase(), self.artist.to_lowercase(), self.album.to_lowercase(), self.duration_s);
        format!("{}.json", &blake3::hash(key.as_bytes()).to_hex()[..32])
    }
}

/// Words that mark a bracketed or " - " part of a title as packaging, not the song's name.
const NOISE: [&str; 22] = [
    "remaster", "remastered", "remasterd", "official", "video", "audio", "lyric", "lyrics", "visualizer", "visualiser", "explicit", "hd", "hq", "4k", "mv", "feat", "ft", "featuring", "version", "edit", "mono", "stereo",
];

fn is_noise(part: &str) -> bool {
    part.to_lowercase().split(|c: char| !c.is_alphanumeric()).any(|w| NOISE.contains(&w) || w.starts_with("remaster"))
}

/// "Song (Remastered 2011) [Official Video]" → "Song"; "Song - 2011 Remaster" → "Song";
/// "Song feat. Someone" → "Song". Brackets that are part of the name stay ("Song (Interlude)").
pub fn clean_title(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut rest = title;
    while let Some(open) = rest.find(['(', '[']) {
        let close = if rest[open..].starts_with('(') { ')' } else { ']' };
        let Some(len) = rest[open + 1..].find(close) else { break };
        let inner = &rest[open + 1..open + 1 + len];
        out.push_str(&rest[..open]);
        if !is_noise(inner) {
            out.push_str(&rest[open..open + len + 2]);
        }
        rest = &rest[open + len + 2..];
    }
    out.push_str(rest);
    // A trailing " - 2011 Remaster" / " - Radio Edit".
    let mut out = match rsplit_dash(&out) {
        Some((left, right)) if is_noise(right) => left.to_string(),
        _ => out,
    };
    // An unbracketed "feat. X" to the end.
    let lower = out.to_lowercase();
    if let Some(at) = [" feat. ", " ft. ", " featuring ", " feat "].iter().filter_map(|m| lower.find(m)).min() {
        out.truncate(at);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ").trim_end_matches([' ', '-', '–', '—']).to_string()
}

/// "Adele - Topic" → "Adele"; "AdeleVEVO" → "Adele".
pub fn clean_artist(artist: &str) -> String {
    let a = artist.trim();
    let a = a.strip_suffix(" - Topic").unwrap_or(a);
    let a = a.strip_suffix("VEVO").unwrap_or(a);
    a.trim().to_string()
}

/// A channel name rather than an artist.
fn is_channel(artist: &str) -> bool {
    let lower = artist.to_lowercase();
    lower.ends_with("vevo") || lower.ends_with(" - topic") || lower.contains("records") || lower.contains("music") || lower.ends_with("official")
}

/// The first " - " (or en/em dash) split, for "Artist - Title".
fn split_artist_title(title: &str) -> Option<(&str, &str)> {
    [" - ", " – ", " — "].iter().filter_map(|sep| title.find(sep).map(|at| (at, sep.len()))).min().map(|(at, len)| (title[..at].trim(), title[at + len..].trim())).filter(|(l, r)| !l.is_empty() && !r.is_empty())
}

fn rsplit_dash(title: &str) -> Option<(&str, &str)> {
    [" - ", " – ", " — "].iter().filter_map(|sep| title.rfind(sep).map(|at| (at, sep.len()))).max().map(|(at, len)| (title[..at].trim_end(), title[at + len..].trim()))
}

/// LRC → timed lines, in order. `[mm:ss.xx]` stamps (several per line repeat it), metadata
/// tags (`[ar:…]`, `[offset:…]`) and untimed lines ignored, word-level `<mm:ss.xx>` stamps
/// removed. A stamped empty line is kept as an empty line: the gap between verses, where
/// the card shows nothing.
pub fn parse_lrc(text: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut stamps = Vec::new();
        while let Some(inner) = rest.strip_prefix('[') {
            let Some(end) = inner.find(']') else { break };
            let Some(ms) = parse_stamp(&inner[..end]) else { break };
            stamps.push(ms);
            rest = inner[end + 1..].trim_start();
        }
        if stamps.is_empty() {
            continue;
        }
        let text = strip_word_stamps(rest);
        lines.extend(stamps.into_iter().map(|at_ms| LyricLine { at_ms, text: text.clone() }));
    }
    lines.sort_by_key(|l| l.at_ms);
    // One empty line per gap is enough, and none that a line at the same time replaces.
    let mut out: Vec<LyricLine> = Vec::with_capacity(lines.len());
    for line in lines {
        match out.last_mut() {
            Some(last) if last.text.is_empty() && line.text.is_empty() => {}
            Some(last) if last.at_ms == line.at_ms && last.text.is_empty() => *last = line,
            _ => out.push(line),
        }
    }
    if out.iter().all(|l| l.text.is_empty()) {
        out.clear();
    }
    out
}

/// "01:02.34" / "01:02.345" / "01:02:34" / "01:02" → ms.
fn parse_stamp(stamp: &str) -> Option<u64> {
    let (min, rest) = stamp.split_once(':')?;
    let (sec, frac) = match rest.split_once(['.', ':']) {
        Some((s, f)) => (s, f),
        None => (rest, ""),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(min) || !digits(sec) || !(frac.is_empty() || digits(frac)) || frac.len() > 3 {
        return None;
    }
    let frac_ms = if frac.is_empty() { 0 } else { frac.parse::<u64>().ok()? * 10u64.pow(3 - frac.len() as u32) };
    Some(min.parse::<u64>().ok()? * 60_000 + sec.parse::<u64>().ok()? * 1000 + frac_ms)
}

fn strip_word_stamps(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        match rest[open..].find('>') {
            Some(len) if parse_stamp(&rest[open + 1..open + len]).is_some() => {
                out.push_str(&rest[..open]);
                rest = &rest[open + len + 1..];
            }
            _ => {
                out.push_str(&rest[..=open]);
                rest = &rest[open + 1..];
            }
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Of search results `(duration in s, has synced lyrics)`, the one closest to `target_s`
/// within ±5 s that has synced lyrics.
fn closest(candidates: &[(f64, bool)], target_s: f64) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .filter(|(_, (d, synced))| *synced && (d - target_s).abs() <= SEARCH_SLACK_S)
        .min_by(|a, b| (a.1 .0 - target_s).abs().total_cmp(&(b.1 .0 - target_s).abs()))
        .map(|(i, _)| i)
}

/// Lyrics for a track: from the disk cache, else LRCLIB. `Some(empty)` means it has none;
/// `None` that we couldn't ask (offline, backing off), so nothing is remembered.
pub async fn lookup(query: Query) -> Option<Vec<LyricLine>> {
    let path = cache_dir().map(|d| d.join(query.cache_name()));
    if let Some(path) = path.clone() {
        let cached = tokio::task::spawn_blocking(move || read_cache(&path, unix_s())).await.ok().flatten();
        if cached.is_some() {
            return cached;
        }
    }
    let lines = fetch(&query).await?;
    if let Some(path) = path {
        let text = cache_json(&lines, unix_s());
        _ = tokio::task::spawn_blocking(move || {
            if let Some(dir) = path.parent() {
                _ = std::fs::create_dir_all(dir);
            }
            _ = std::fs::write(&path, text);
        })
        .await;
    }
    Some(lines)
}

fn cache_dir() -> Option<PathBuf> {
    Some(crate::paths::local().join("lyrics"))
}

fn unix_s() -> u64 {
    reui_protocol::unix_ms() / 1000
}

fn cache_json(lines: &[LyricLine], now_s: u64) -> String {
    let lines: Vec<Value> = lines.iter().map(|l| json!([l.at_ms, l.text])).collect();
    json!({ "at": now_s, "lines": lines }).to_string()
}

/// A cached answer that's still good.
fn read_cache(path: &Path, now_s: u64) -> Option<Vec<LyricLine>> {
    parse_cache(&std::fs::read_to_string(path).ok()?, now_s)
}

fn parse_cache(text: &str, now_s: u64) -> Option<Vec<LyricLine>> {
    let value: Value = serde_json::from_str(text).ok()?;
    let at = value.get("at")?.as_u64()?;
    let lines: Vec<LyricLine> = value
        .get("lines")?
        .as_array()?
        .iter()
        .filter_map(|l| Some(LyricLine { at_ms: l.get(0)?.as_u64()?, text: l.get(1)?.as_str()?.to_string() }))
        .collect();
    if lines.is_empty() && now_s.saturating_sub(at) > NEGATIVE_TTL_SECS {
        return None;
    }
    Some(lines)
}

/// 429/503: wait before asking again (30 s, doubling to 10 min).
static BACKOFF: Mutex<Option<(Instant, Duration)>> = Mutex::new(None);

fn backing_off() -> bool {
    BACKOFF.lock().unwrap().is_some_and(|(until, _)| Instant::now() < until)
}

fn back_off() {
    let mut backoff = BACKOFF.lock().unwrap();
    let wait = backoff.map_or(Duration::from_secs(30), |(_, w)| (w * 2).min(Duration::from_secs(600)));
    eprintln!("media: lrclib.net asked us to slow down; lyrics paused for {}s", wait.as_secs());
    *backoff = Some((Instant::now() + wait, wait));
}

fn client() -> Option<&'static Client> {
    static CLIENT: OnceLock<Option<Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            Client::builder().timeout(TIMEOUT).user_agent(concat!("ReUI/", env!("CARGO_PKG_VERSION"), " (https://github.com/)")).build().inspect_err(|e| eprintln!("media: no HTTP client for lyrics: {e}")).ok()
        })
        .as_ref()
}

enum Answer {
    Found(Value),
    NotFound,
}

async fn get(url: Url) -> Option<Answer> {
    if backing_off() {
        return None;
    }
    let response = client()?.get(url).send().await.inspect_err(|e| eprintln!("media: lyrics lookup failed: {e}")).ok()?;
    match response.status() {
        StatusCode::NOT_FOUND => return Some(Answer::NotFound),
        StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE => {
            back_off();
            return None;
        }
        s if !s.is_success() => {
            eprintln!("media: lyrics lookup answered {s}");
            return None;
        }
        _ => {}
    }
    *BACKOFF.lock().unwrap() = None;
    if response.content_length().is_some_and(|n| n as usize > MAX_BODY) {
        return None;
    }
    let text = response.text().await.ok()?;
    Some(serde_json::from_str(&text).map_or(Answer::NotFound, Answer::Found))
}

fn synced(record: &Value) -> Option<&str> {
    record.get("syncedLyrics").and_then(Value::as_str).filter(|s| !s.trim().is_empty())
}

async fn fetch(query: &Query) -> Option<Vec<LyricLine>> {
    let duration = query.duration_s.to_string();
    let mut params = vec![("track_name", query.title.as_str()), ("artist_name", query.artist.as_str()), ("duration", duration.as_str())];
    if !query.album.is_empty() {
        params.push(("album_name", query.album.as_str()));
    }
    let exact = get(Url::parse_with_params(&format!("{API}/get"), &params).ok()?).await?;
    if let Answer::Found(record) = &exact {
        if record.get("instrumental").and_then(Value::as_bool) == Some(true) {
            return Some(Vec::new());
        }
        let close = record.get("duration").and_then(Value::as_f64).is_none_or(|d| (d - query.duration_s as f64).abs() <= EXACT_SLACK_S);
        if let (true, Some(lrc)) = (close, synced(record)) {
            return Some(parse_lrc(lrc));
        }
    }
    let url = Url::parse_with_params(&format!("{API}/search"), [("track_name", query.title.as_str()), ("artist_name", query.artist.as_str())]).ok()?;
    let Answer::Found(Value::Array(results)) = get(url).await? else { return Some(Vec::new()) };
    let candidates: Vec<(f64, bool)> = results.iter().map(|r| (r.get("duration").and_then(Value::as_f64).unwrap_or(f64::MAX), synced(r).is_some())).collect();
    Some(closest(&candidates, query.duration_s as f64).and_then(|i| synced(&results[i])).map(parse_lrc).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(at_ms: u64, text: &str) -> LyricLine {
        LyricLine { at_ms, text: text.into() }
    }

    #[test]
    fn lrc_parses_stamps_repeats_and_gaps() {
        let lrc = "[ar:Someone]\n[offset:+100]\n[00:01.50]First\n\n[00:05.00]\n[00:06.00]\n[00:07.2][01:00.123]Chorus\nno stamp\n[00:09:50]Colon <00:09.80>word stamps\n[xx:yy]bad";
        assert_eq!(parse_lrc(lrc), vec![line(1_500, "First"), line(5_000, ""), line(7_200, "Chorus"), line(9_500, "Colon word stamps"), line(60_123, "Chorus")]);
        assert_eq!(parse_lrc("[ti:Only tags]\nplain text"), vec![]);
        assert_eq!(parse_lrc("[00:01.00]\n[00:02.00]"), vec![]);
        assert_eq!(parse_lrc("[00:01]a\n[00:01]\n"), vec![line(1_000, "a"), line(1_000, "")]);
    }

    #[test]
    fn stamps() {
        assert_eq!(parse_stamp("01:02.34"), Some(62_340));
        assert_eq!(parse_stamp("01:02.345"), Some(62_345));
        assert_eq!(parse_stamp("1:02"), Some(62_000));
        assert_eq!(parse_stamp("ar:Name"), None);
        assert_eq!(parse_stamp("01:02.3456"), None);
    }

    #[test]
    fn titles_are_cleaned() {
        assert_eq!(clean_title("Here Comes the Sun (Remastered 2009)"), "Here Comes the Sun");
        assert_eq!(clean_title("Song - 2011 Remaster"), "Song");
        assert_eq!(clean_title("Song [Official Video] (feat. Someone)"), "Song");
        assert_eq!(clean_title("Song feat. Someone Else"), "Song");
        assert_eq!(clean_title("Song (Interlude)"), "Song (Interlude)");
        assert_eq!(clean_title("Song - Radio Edit"), "Song");
        assert_eq!(clean_artist("Adele - Topic"), "Adele");
        assert_eq!(clean_artist("TaylorSwiftVEVO"), "TaylorSwift");
    }

    #[test]
    fn youtube_titles_split_into_artist_and_title() {
        let q = Query::new("Adele - Hello (Official Music Video)", "Adele", "", 366_000, true).unwrap();
        assert_eq!((q.artist.as_str(), q.title.as_str(), q.duration_s), ("Adele", "Hello", 366));
        let q = Query::new("Rick Astley - Never Gonna Give You Up", "", "", 213_400, false).unwrap();
        assert_eq!((q.artist.as_str(), q.title.as_str()), ("Rick Astley", "Never Gonna Give You Up"));
        let q = Query::new("Artist - Song", "Atlantic Records", "", 200_000, false).unwrap();
        assert_eq!((q.artist.as_str(), q.title.as_str()), ("Artist", "Song"));
        // A music app's own artist stays; the remaster suffix doesn't make a split.
        let q = Query::new("Song - Remastered", "Band", "Song", 200_000, false).unwrap();
        assert_eq!((q.artist.as_str(), q.title.as_str(), q.album.as_str()), ("Band", "Song", ""));
        assert_eq!(Query::new("Song", "", "", 200_000, false), None);
        assert_eq!(Query::new("Song", "Band", "", 0, false), None);
    }

    #[test]
    fn search_takes_the_closest_synced_duration_within_five_seconds() {
        assert_eq!(closest(&[(200.0, true), (203.0, true), (201.5, false)], 202.0), Some(1));
        assert_eq!(closest(&[(190.0, true), (215.0, true)], 202.0), None);
        assert_eq!(closest(&[(202.0, false)], 202.0), None);
        assert_eq!(closest(&[], 202.0), None);
    }

    #[test]
    fn cache_keeps_lyrics_and_forgets_misses_after_a_week() {
        let lines = vec![line(1_000, "a"), line(2_000, "")];
        assert_eq!(parse_cache(&cache_json(&lines, 100), 100 + 365 * 86_400), Some(lines));
        assert_eq!(parse_cache(&cache_json(&[], 100), 100 + 86_400), Some(vec![]));
        assert_eq!(parse_cache(&cache_json(&[], 100), 100 + 8 * 86_400), None);
        assert_eq!(parse_cache("nope", 0), None);
    }
}
