//! Which media session Now Playing shows, and what its player is called. Pure: the module
//! feeds in what it has observed of each session (PRD 5.2), so Windows' own idea of the
//! "current" session is only a tie-breaker.

/// What's known of one session, times in Unix ms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    pub id: u64,
    pub playing: bool,
    /// When it last started playing (or was first seen).
    pub started_at: u64,
    /// When it was last seen playing (or was first seen).
    pub last_played: u64,
    /// Windows' current session.
    pub current: bool,
    /// Has a title, isn't closed, and isn't a hidden video.
    pub eligible: bool,
}

/// A paused session still counts until it's been paused `retract_ms`; 0 keeps it forever.
fn fresh(since: u64, now: u64, retract_ms: u64) -> bool {
    retract_ms == 0 || now.saturating_sub(since) < retract_ms
}

/// The session to show:
/// 1. the one the user picked (`(id, when)`) while it exists and has played (or was picked)
///    within the retract time;
/// 2. else the playing one that started most recently (ties: Windows' current);
/// 3. else the most recently playing paused one, until it's been paused `retract_ms`;
/// 4. else nothing.
pub fn rank(candidates: &[Candidate], picked: Option<(u64, u64)>, now: u64, retract_ms: u64) -> Option<u64> {
    let eligible = || candidates.iter().filter(|c| c.eligible);
    if let Some((id, at)) = picked {
        if let Some(c) = eligible().find(|c| c.id == id) {
            if c.playing || fresh(c.last_played.max(at), now, retract_ms) {
                return Some(id);
            }
        }
    }
    if let Some(c) = eligible().filter(|c| c.playing).max_by_key(|c| (c.started_at, c.current)) {
        return Some(c.id);
    }
    eligible().filter(|c| fresh(c.last_played, now, retract_ms)).max_by_key(|c| (c.last_played, c.current)).map(|c| c.id)
}

/// When a shown paused session stops counting (Unix ms), for a timer; `None` while it
/// plays or when paused sessions never step away.
pub fn expires_at(c: &Candidate, picked: Option<(u64, u64)>, retract_ms: u64) -> Option<u64> {
    if c.playing || retract_ms == 0 {
        return None;
    }
    let since = match picked {
        Some((id, at)) if id == c.id => c.last_played.max(at),
        _ => c.last_played,
    };
    Some(since + retract_ms)
}

/// The session `step` places from `shown` in `order` (wrapping), for the ‹ › switcher.
pub fn step(order: &[u64], shown: u64, step: isize) -> Option<u64> {
    let at = order.iter().position(|&id| id == shown)? as isize;
    let len = order.len() as isize;
    (len > 1).then(|| order[(at + step).rem_euclid(len) as usize])
}

/// Chrome's YouTube Music app (the PWA id is the same for every install).
const YOUTUBE_MUSIC_PWA: &str = "cinhimbnkkaeohfgghhklpknlkffjgod";

/// A readable player name from its AppUserModelID ("Spotify.exe", "MSEdge",
/// "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify", "Microsoft.ZuneVideo_8wekyb3d8bbwe!…").
/// `exe` is the player's executable when the AUMID says nothing (Firefox names its session
/// with an install hash).
pub fn app_name(aumid: &str, exe: Option<&str>) -> String {
    const KNOWN: [(&str, &str); 13] = [
        (YOUTUBE_MUSIC_PWA, "YouTube Music"),
        ("youtubemusic", "YouTube Music"),
        ("spotify", "Spotify"),
        ("zunevideo", "Movies & TV"),
        ("zunemusic", "Media Player"),
        ("applemusic", "Apple Music"),
        ("chrome", "Chrome"),
        ("msedge", "Edge"),
        ("firefox", "Firefox"),
        ("vlc", "VLC"),
        ("brave", "Brave"),
        ("opera", "Opera"),
        ("librewolf", "LibreWolf"),
    ];
    let lower = aumid.to_ascii_lowercase().replace(' ', "");
    if let Some((_, name)) = KNOWN.iter().find(|(needle, _)| lower.contains(needle)) {
        return name.to_string();
    }
    if is_hash(aumid) {
        return match exe {
            Some(exe) => app_name(exe, None),
            None => "Firefox".into(),
        };
    }
    // Packaged: "Publisher.App_hash!Entry" → "App"; executable: "C:\...\app.exe" → "app".
    let base = aumid.split('!').next().unwrap_or(aumid);
    let base = base.split('_').next().unwrap_or(base);
    let base = base.rsplit(['\\', '/']).next().unwrap_or(base);
    let base = base.strip_suffix(".exe").or_else(|| base.strip_suffix(".EXE")).unwrap_or(base);
    let name = base.rsplit('.').next().unwrap_or(base);
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Now Playing".into(),
    }
}

/// Firefox-style session ids: 16 hex digits, no app name in them.
pub fn is_hash(aumid: &str) -> bool {
    aumid.len() == 16 && aumid.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Browsers play whatever a page does, so their titles are YouTube-like.
pub fn is_browser(app: &str) -> bool {
    matches!(app, "Chrome" | "Edge" | "Firefox" | "Brave" | "Opera" | "LibreWolf" | "Waterfox" | "Zen" | "Floorp")
}

/// Spotify's ads on the free tier: "Advertisement" or "Spotify" with no artist.
pub fn is_ad(aumid: &str, title: &str, artist: &str) -> bool {
    aumid.to_ascii_lowercase().contains("spotify") && artist.trim().is_empty() && matches!(title.trim(), "Advertisement" | "Spotify" | "Spotify Free")
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;

    fn c(id: u64, playing: bool, started_at: u64, last_played: u64) -> Candidate {
        Candidate { id, playing, started_at, last_played, current: false, eligible: true }
    }

    #[test]
    fn the_most_recently_started_player_wins() {
        let now = 100 * MIN;
        // Spotify paused, YouTube starts in Chrome: Chrome, whatever Windows thinks.
        let spotify = Candidate { current: true, ..c(1, false, 10 * MIN, 99 * MIN) };
        let chrome = c(2, true, 99 * MIN, now);
        assert_eq!(rank(&[spotify, chrome], None, now, 10 * MIN), Some(2));
        // Two playing: the newer; a tie goes to Windows' current.
        let a = c(1, true, 50 * MIN, now);
        let b = c(2, true, 60 * MIN, now);
        assert_eq!(rank(&[a, b], None, now, 10 * MIN), Some(2));
        let a = Candidate { current: true, ..c(1, true, 60 * MIN, now) };
        assert_eq!(rank(&[a, b], None, now, 10 * MIN), Some(1));
    }

    #[test]
    fn paused_players_stay_until_the_retract_time() {
        let now = 100 * MIN;
        let older = c(1, false, 0, 85 * MIN);
        let recent = c(2, false, 0, 95 * MIN);
        assert_eq!(rank(&[older, recent], None, now, 10 * MIN), Some(2));
        assert_eq!(rank(&[older, recent], None, 106 * MIN, 10 * MIN), None);
        // 0: never steps away.
        assert_eq!(rank(&[older], None, 1_000 * MIN, 0), Some(1));
        assert_eq!(expires_at(&recent, None, 10 * MIN), Some(105 * MIN));
        assert_eq!(expires_at(&c(3, true, 0, now), None, 10 * MIN), None);
    }

    #[test]
    fn a_picked_source_holds_while_it_lives() {
        let now = 100 * MIN;
        let playing = c(1, true, 99 * MIN, now);
        let paused = c(2, false, 0, 20 * MIN);
        // Picked just now: shown even though it's long paused and another one plays.
        assert_eq!(rank(&[playing, paused], Some((2, 99 * MIN)), now, 10 * MIN), Some(2));
        assert_eq!(expires_at(&paused, Some((2, 99 * MIN)), 10 * MIN), Some(109 * MIN));
        // The pick goes stale, or the session goes away: back to the policy.
        assert_eq!(rank(&[playing, paused], Some((2, 80 * MIN)), now, 10 * MIN), Some(1));
        assert_eq!(rank(&[playing], Some((2, 99 * MIN)), now, 10 * MIN), Some(1));
    }

    #[test]
    fn untitled_or_closed_sessions_are_skipped() {
        let hidden = Candidate { eligible: false, ..c(1, true, 99, 100) };
        assert_eq!(rank(&[hidden], None, 100, MIN), None);
        assert_eq!(rank(&[hidden, c(2, false, 0, 90)], Some((1, 100)), 100, MIN), Some(2));
    }

    #[test]
    fn the_switcher_wraps() {
        assert_eq!(step(&[4, 7, 9], 9, 1), Some(4));
        assert_eq!(step(&[4, 7, 9], 4, -1), Some(9));
        assert_eq!(step(&[4], 4, 1), None);
        assert_eq!(step(&[4, 7], 5, 1), None);
    }

    #[test]
    fn player_names_are_readable() {
        assert_eq!(app_name("Spotify.exe", None), "Spotify");
        assert_eq!(app_name("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify", None), "Spotify");
        assert_eq!(app_name("MSEdge", None), "Edge");
        assert_eq!(app_name("Chrome", None), "Chrome");
        assert_eq!(app_name("Chrome._crx_cinhimbnkkaeohfgghhklpknlkffjgod", None), "YouTube Music");
        assert_eq!(app_name("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic", None), "Media Player");
        assert_eq!(app_name("Microsoft.ZuneVideo_8wekyb3d8bbwe!Microsoft.ZuneVideo", None), "Movies & TV");
        assert_eq!(app_name("308046B0AF4A39CB", None), "Firefox");
        assert_eq!(app_name("308046B0AF4A39CB", Some("zen.exe")), "Zen");
        assert_eq!(app_name("308046B0AF4A39CB", Some("librewolf.exe")), "LibreWolf");
        assert_eq!(app_name("C:\\Program Files\\Foobar2000\\foobar2000.exe", None), "Foobar2000");
        assert_eq!(app_name("Publisher.CoolPlayer_abc123!App", None), "CoolPlayer");
        assert_eq!(app_name("", None), "Now Playing");
    }

    #[test]
    fn spotify_ads_are_recognised() {
        assert!(is_ad("Spotify.exe", "Advertisement", ""));
        assert!(is_ad("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify", "Spotify", " "));
        assert!(!is_ad("Spotify.exe", "Spotify", "Some Band"));
        assert!(!is_ad("Chrome", "Advertisement", ""));
    }
}
