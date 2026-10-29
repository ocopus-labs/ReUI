//! Finding the meeting link in an event (PRD §5.4): calendars put it in a property of
//! their own, the location, or deep in an HTML invitation, often wrapped in a redirect
//! (Outlook's Safe Links, Google's `/url?q=`). Only https links on known meeting hosts
//! count, so an invitation can't make Join run a file or another app's protocol. Pure.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Teams,
    Zoom,
    Meet,
    Webex,
    GoTo,
    Whereby,
    Jitsi,
    Chime,
}

impl Service {
    /// What the card calls it ("10:00 – 10:30 · Teams").
    pub fn name(self) -> &'static str {
        match self {
            Service::Teams => "Teams",
            Service::Zoom => "Zoom",
            Service::Meet => "Google Meet",
            Service::Webex => "Webex",
            Service::GoTo => "GoTo",
            Service::Whereby => "Whereby",
            Service::Jitsi => "Jitsi",
            Service::Chime => "Chime",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub url: String,
    pub service: Service,
}

/// The first meeting link in `fields`, looked at in the order given (the caller passes
/// the dedicated properties first, then location, description and HTML).
pub fn find<'a>(fields: impl IntoIterator<Item = &'a str>) -> Option<Link> {
    fields.into_iter().find_map(|text| urls(text).find_map(|u| meeting(&u)))
}

/// The http(s) URLs in a text, cut where URLs end in prose and markup.
fn urls(text: &str) -> impl Iterator<Item = String> + '_ {
    let lower = text.to_ascii_lowercase();
    let mut starts = Vec::new();
    let mut from = 0;
    while let Some(i) = lower[from..].find("http") {
        let at = from + i;
        if lower[at..].starts_with("https://") || lower[at..].starts_with("http://") {
            starts.push(at);
        }
        from = at + 4;
    }
    starts.into_iter().map(move |at| {
        let rest = &text[at..];
        let end = rest.find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '[' | ']' | '{' | '}' | '|' | '\\' | '^' | '`')).unwrap_or(rest.len());
        rest[..end].trim_end_matches(['.', ',', ';', ')', '(', '!', '*']).to_string()
    })
}

/// `url` as a meeting link, unwrapping a redirect first.
fn meeting(url: &str) -> Option<Link> {
    let url = unwrap(url);
    let lower = url.to_ascii_lowercase();
    let rest = lower.strip_prefix("https://")?;
    let (host, path) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
    let host = host.rsplit('@').next()?.split(':').next()?;
    let on = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    let service = if on("teams.microsoft.com") && (path.starts_with("/l/meetup-join/") || path.starts_with("/meet/")) || on("teams.live.com") && path.starts_with("/meet/") {
        Service::Teams
    } else if (on("zoom.us") || on("zoomgov.com")) && ["/j/", "/my/", "/w/", "/s/"].iter().any(|p| path.starts_with(p)) {
        Service::Zoom
    } else if host == "meet.google.com" && is_meet_code(path) {
        Service::Meet
    } else if on("webex.com") && ["/meet/", "/join/", "/wbxmjs/", "/j.php"].iter().any(|p| path.contains(p)) {
        Service::Webex
    } else if host == "meet.goto.com" && path.len() > 1 || on("gotomeeting.com") && path.starts_with("/join/") {
        Service::GoTo
    } else if on("whereby.com") && path.len() > 1 {
        Service::Whereby
    } else if host == "meet.jit.si" && path.len() > 1 {
        Service::Jitsi
    } else if on("chime.aws") && path.len() > 1 {
        Service::Chime
    } else {
        return None;
    };
    Some(Link { url, service })
}

/// `/abc-defg-hij`: Meet's own rooms, not its landing pages.
fn is_meet_code(path: &str) -> bool {
    let code = path.trim_start_matches('/').split(['?', '/', '#']).next().unwrap_or("");
    let parts: Vec<&str> = code.split('-').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_lowercase()))
}

/// The link a redirect points to: Outlook's Safe Links (`*.safelinks.protection.outlook.com
/// /?url=…`) and Google's (`www.google.com/url?q=…`). Others are left as they are.
fn unwrap(url: &str) -> String {
    let lower = url.to_ascii_lowercase();
    let key = if lower.contains(".safelinks.protection.outlook.com/") {
        "url"
    } else if lower.starts_with("https://www.google.com/url?") || lower.starts_with("https://google.com/url?") {
        "q"
    } else {
        return url.to_string();
    };
    let query = url.split_once('?').map_or("", |(_, q)| q);
    query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| percent_decode(v))
        .filter(|v| v.to_ascii_lowercase().starts_with("https://"))
        .unwrap_or_else(|| url.to_string())
}

fn percent_decode(v: &str) -> String {
    let bytes = v.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%').then(|| bytes.get(i + 1..i + 3)).flatten().and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match (hex, bytes[i]) {
            (Some(b), _) => {
                out.push(b);
                i += 3;
                continue;
            }
            (None, b'+') => out.push(b' '),
            (None, b) => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The location as a place worth showing: `None` when it's only a link or the meeting
/// service's own words ("Microsoft Teams Meeting"), which the service's name says better.
pub fn place(location: &str) -> Option<String> {
    let l = location.trim();
    let lower = l.to_ascii_lowercase();
    if l.is_empty() || lower.starts_with("http") || ["microsoft teams", "teams meeting", "zoom meeting", "google meet", "webex"].iter().any(|w| lower.contains(w)) {
        return None;
    }
    // "Room 4; https://zoom.us/j/1": the part before the link.
    let cut = lower.find("http").map_or(l, |i| &l[..i]);
    let cut = cut.trim().trim_end_matches([';', ',', '-', '·']).trim();
    (!cut.is_empty()).then(|| cut.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(text: &str) -> Option<Service> {
        find([text]).map(|l| l.service)
    }

    #[test]
    fn real_world_meeting_links_are_found() {
        let teams = "Join: https://teams.microsoft.com/l/meetup-join/19%3ameeting_NjE2YzY3%40thread.v2/0?context=%7b%22Tid%22%3a%22abc%22%7d";
        assert_eq!(find([teams]).unwrap().url, &teams[6..]);
        assert_eq!(service("https://teams.microsoft.com/meet/26812345678?p=AbCdEf"), Some(Service::Teams));
        assert_eq!(service("https://teams.live.com/meet/9345678901234?p=x"), Some(Service::Teams));
        assert_eq!(service("<https://us02web.zoom.us/j/81234567890?pwd=AbC.1>"), Some(Service::Zoom));
        assert_eq!(find(["(https://zoom.us/my/sam.k)."]).unwrap().url, "https://zoom.us/my/sam.k");
        assert_eq!(service("https://acme.zoomgov.com/j/1612345678"), Some(Service::Zoom));
        assert_eq!(service("Meet: https://meet.google.com/abc-defg-hij?authuser=0"), Some(Service::Meet));
        assert_eq!(service("https://acme.webex.com/acme/j.php?MTID=m123"), Some(Service::Webex));
        assert_eq!(service("https://acme.webex.com/meet/sam"), Some(Service::Webex));
        assert_eq!(service("https://meet.goto.com/123456789"), Some(Service::GoTo));
        assert_eq!(service("https://whereby.com/team-room"), Some(Service::Whereby));
        assert_eq!(service("https://meet.jit.si/OurRoom"), Some(Service::Jitsi));
        assert_eq!(service("https://chime.aws/1234567890"), Some(Service::Chime));
    }

    #[test]
    fn landing_pages_other_schemes_and_lookalikes_are_not_meetings() {
        assert_eq!(service("https://zoom.us/download"), None);
        assert_eq!(service("https://meet.google.com/"), None);
        assert_eq!(service("https://support.google.com/meet"), None);
        assert_eq!(service("http://zoom.us/j/123"), None, "https only");
        assert_eq!(service("zoommtg://zoom.us/join?confno=123"), None);
        assert_eq!(service("https://zoom.us.evil.example/j/123"), None);
        assert_eq!(service("https://evilzoom.us/j/123"), None);
        assert_eq!(service("file:///C:/Windows/notepad.exe"), None);
        assert_eq!(service("https://teams.microsoft.com/downloads"), None);
    }

    #[test]
    fn redirects_are_unwrapped_first() {
        let safe = "https://eur02.safelinks.protection.outlook.com/?url=https%3A%2F%2Fteams.microsoft.com%2Fl%2Fmeetup-join%2F19%253ameeting_X%2540thread.v2%2F0&data=05%7C01&reserved=0";
        let link = find([safe]).unwrap();
        assert_eq!(link.service, Service::Teams);
        assert_eq!(link.url, "https://teams.microsoft.com/l/meetup-join/19%3ameeting_X%40thread.v2/0");
        let google = "https://www.google.com/url?q=https://us06web.zoom.us/j/8123?pwd%3Dabc&sa=D&source=calendar";
        assert_eq!(find([google]).unwrap().url, "https://us06web.zoom.us/j/8123?pwd=abc");
        // A redirect to something that isn't a meeting stays nothing.
        assert_eq!(service("https://www.google.com/url?q=https://example.com/"), None);
    }

    #[test]
    fn fields_are_tried_in_order_and_html_works() {
        let html = r#"<p>Hi</p><a href="https://meet.google.com/xyz-abcd-efg">Join with Google Meet</a>"#;
        assert_eq!(find(["Room 4", "no link here", html]).unwrap().service, Service::Meet);
        let both = find(["https://zoom.us/j/1", "https://meet.google.com/abc-defg-hij"]).unwrap();
        assert_eq!(both.service, Service::Zoom, "the first field wins");
        assert!(find(["", "Lunch"]).is_none());
    }

    #[test]
    fn places_drop_links_and_service_words() {
        assert_eq!(place("Room 4"), Some("Room 4".into()));
        assert_eq!(place("Microsoft Teams Meeting"), None);
        assert_eq!(place("https://zoom.us/j/1"), None);
        assert_eq!(place("Room 4; https://zoom.us/j/1"), Some("Room 4".into()));
        assert_eq!(place("  "), None);
    }
}
