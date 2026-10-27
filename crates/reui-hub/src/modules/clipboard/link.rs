//! Link previews for copied URLs: the page's title, site name and image, from its HTML
//! metadata (`og:*`, `twitter:*`, `<title>`).
//!
//! This is the one place ReUI talks to the network for content, so it's deliberately
//! narrow: public http(s) hosts only (never localhost, private ranges or single-label
//! names, so copying an intranet or router link never makes ReUI call it), the first
//! 512 KB of HTML, a 4 s timeout, and the image capped at 4 MB. Names are resolved by
//! us and only public addresses reach the connector, every redirect hop is re-checked,
//! and URLs that look like they carry a secret (reset/magic links, signed URLs, OAuth
//! callbacks) are never fetched as-is: only their origin is (PRD decision D6).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::redirect::{Action, Attempt, Policy};
use reqwest::{Client, Url};

const MAX_HTML: usize = 512 * 1024;
const MAX_IMAGE: usize = 4 * 1024 * 1024;
const MAX_REDIRECTS: usize = 4;
const TIMEOUT: Duration = Duration::from_secs(4);
const THUMB_PX: u32 = 360;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct LinkMeta {
    pub title: Option<String>,
    pub site: Option<String>,
    /// PNG thumbnail of the page image.
    pub image: Option<Vec<u8>>,
}

pub fn client() -> Option<Client> {
    Client::builder()
        .timeout(TIMEOUT)
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) ReUI link preview")
        .redirect(Policy::custom(redirect))
        .dns_resolver(Arc::new(PublicResolver))
        // A proxy resolves names itself, around the public-address check.
        .no_proxy()
        .build()
        .ok()
}

/// Each hop must pass the same checks as the URL we started from. This runs before the
/// hop is connected, so a redirect to `http://10.0.0.1/` never gets a socket.
fn redirect(attempt: Attempt) -> Action {
    // `previous` starts with the original URL, which isn't a redirect.
    if attempt.previous().len() > MAX_REDIRECTS {
        attempt.error("too many redirects")
    } else if !allowed(attempt.url()) {
        attempt.error("redirect to a disallowed URL")
    } else {
        attempt.follow()
    }
}

/// Resolves names ourselves and hands the connector only public addresses, so a name
/// that resolves (or rebinds) to a private address can't be reached: the connection
/// goes to exactly the addresses checked here. (IP-literal hosts skip the resolver;
/// `allowed` checks those.)
struct PublicResolver;

impl Resolve for PublicResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0)).await?.filter(|a| public_ip(a.ip())).collect();
            if addrs.is_empty() {
                return Err(format!("{host} has no public address").into());
            }
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

/// Whether an address is on the public internet: not loopback, private, link-local,
/// CGNAT, multicast, documentation or otherwise reserved. IPv6 forms that embed an IPv4
/// address are judged by that address.
fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => public_v4(v4),
        IpAddr::V6(v6) => public_v6(v6),
    }
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0 // 0.0.0.0/8, "this network"
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local() // incl. 169.254.169.254 cloud metadata
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || (a == 100 && (b & 0xc0) == 64) // CGNAT 100.64.0.0/10
        || (a == 192 && b == 0 && c == 0) // IETF protocol assignments
        || (a == 198 && (b & 0xfe) == 18) // benchmarking 198.18.0.0/15
        || a >= 240) // reserved 240.0.0.0/4
}

fn public_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return false;
    }
    // IPv4-mapped (::ffff:a.b.c.d), IPv4-compatible (::a.b.c.d), NAT64 (64:ff9b::/96).
    let embeds_v4 = (s[..5] == [0; 5] && (s[5] == 0xffff || s[5] == 0)) || (s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0; 4]);
    if embeds_v4 {
        let [.., a, b, c, d] = ip.octets();
        return public_v4(Ipv4Addr::new(a, b, c, d));
    }
    // 6to4 (2002::/16) carries the IPv4 address in segments 1-2.
    if s[0] == 0x2002 {
        let [_, _, a, b, c, d, ..] = ip.octets();
        return public_v4(Ipv4Addr::new(a, b, c, d));
    }
    !((s[0] & 0xfe00) == 0xfc00 // ULA fc00::/7
        || (s[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
        || (s[0] & 0xffc0) == 0xfec0 // site-local fec0::/10 (deprecated)
        || (s[0] == 0x2001 && s[1] == 0x0db8)) // documentation 2001:db8::/32
}

/// Whether ReUI may connect to this URL at all (also applied to every redirect hop and
/// to the page image).
pub fn allowed(url: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") || has_credentials(url) {
        return false;
    }
    let Some(host) = url.host_str() else { return false };
    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
        return public_ip(ip);
    }
    let host = host.to_ascii_lowercase();
    host.contains('.') && !["localhost", ".local", ".internal", ".lan", ".home", ".corp"].iter().any(|s| host == s.trim_start_matches('.') || host.ends_with(s))
}

fn has_credentials(url: &Url) -> bool {
    !url.username().is_empty() || url.password().is_some()
}

/// What to fetch for a copied URL.
#[derive(Debug, PartialEq)]
enum Plan {
    /// Nothing: not a public http(s) URL, or it carries `user:pass@` credentials.
    Never,
    /// Only `scheme://host[:port]/`. The URL looks like it carries a secret, and
    /// fetching it could use it up: reset and magic links are often single-use, and
    /// link prefetchers are a known way to break them.
    Origin(Url),
    /// The URL itself, minus its fragment.
    Full(Url),
}

fn plan(url: &Url) -> Plan {
    if !allowed(url) {
        return Plan::Never;
    }
    let mut url = url.clone();
    url.set_fragment(None);
    if secret_query(&url) || secret_path(&url) {
        url.set_path("/");
        url.set_query(None);
        return Plan::Origin(url);
    }
    Plan::Full(url)
}

const SECRET_PARAMS: &[&str] = &[
    "token", "access_token", "id_token", "code", "sig", "signature", "key", "apikey", "api_key", "secret", "password", "pass", "pwd", "reset", "magic", "invite", "otp",
    "auth", "state", "session", "sid", "ticket", "credential", "credentials",
];

fn secret_query(url: &Url) -> bool {
    let names: Vec<String> = url.query_pairs().map(|(k, _)| k.to_ascii_lowercase()).collect();
    let has = |n: &str| names.iter().any(|k| k == n);
    // Azure SAS: `sv` (service version) with `sig`. `sig` alone already matches; this
    // keeps the rule explicit if the list changes.
    if has("sv") && has("sig") {
        return true;
    }
    names.iter().any(|k| {
        SECRET_PARAMS.contains(&k.as_str())
            || k.starts_with("x-amz-")
            || k.starts_with("x-goog-")
            || k.ends_with("token") // refresh_token, csrf_token, ...
            || k.ends_with("secret")
            || k.ends_with("password")
    })
}

/// Path words that mark a one-shot link on their own (`/password/reset`, `/verify-email`).
const SECRET_PATH_WORDS: &[&str] = &["reset", "verify", "confirm", "magic", "invite", "unsubscribe"];
/// Path words that mark one only when an opaque token segment follows (`/login/<token>`).
const AUTH_PATH_WORDS: &[&str] = &["login", "signin", "auth", "activate", "callback"];

fn secret_path(url: &Url) -> bool {
    let segments: Vec<String> = url.path_segments().map(|s| s.map(str::to_ascii_lowercase).collect()).unwrap_or_default();
    let has_word = |seg: &str, words: &[&str]| seg.split(['-', '_', '.']).any(|w| words.iter().any(|k| w.starts_with(k)));
    segments
        .iter()
        .enumerate()
        .any(|(i, seg)| has_word(seg, SECRET_PATH_WORDS) || jwt_like(seg) || (has_word(seg, AUTH_PATH_WORDS) && segments[i + 1..].iter().any(|s| opaque(s))))
}

/// A long run of token alphabet: the shape of a one-shot token in a path. Not enough on
/// its own (Figma file keys look like this too), only after an auth word.
fn opaque(segment: &str) -> bool {
    segment.len() >= 20 && segment.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// A JWT (`eyJ...` header, three dot-separated parts), lower-cased.
fn jwt_like(segment: &str) -> bool {
    segment.starts_with("eyj") && segment.split('.').count() == 3
}

pub async fn fetch(client: &Client, url: &str) -> Option<LinkMeta> {
    let (whole, url) = match plan(&Url::parse(url).ok()?) {
        Plan::Full(url) => (true, url),
        Plan::Origin(url) => (false, url),
        Plan::Never => return None,
    };
    let response = client.get(url).send().await.ok()?.error_for_status().ok()?;
    // The redirect policy already checked every hop; this is a second line.
    if !allowed(response.url()) {
        return None;
    }
    let page_url = response.url().clone();
    let content_type = content_type(&response);
    if !content_type.as_deref().is_none_or(|t| t.contains("html")) {
        return None;
    }
    // A cut-off page still has its <head>, which is all we read.
    let html = read_capped(response, MAX_HTML, true).await?;
    let html = decode_html(&html, content_type.as_deref());
    let mut meta = parse(&html);
    if !whole {
        // Only the site's name: the home page's title and image would describe a
        // different page than the one copied.
        return meta.site.map(|site| LinkMeta { site: Some(site), ..LinkMeta::default() });
    }
    if let Some(image_url) = image_url(&html).and_then(|src| page_url.join(&src).ok()).filter(allowed) {
        if let Some(bytes) = fetch_image(client, image_url).await {
            meta.image = tokio::task::spawn_blocking(move || super::os::thumbnail(&bytes, THUMB_PX)).await.ok().flatten();
        }
    }
    (meta.title.is_some() || meta.image.is_some()).then_some(meta)
}

/// The image's bytes, if it's a raster image that fits under the cap whole: a cut-off
/// image decodes to half a picture or garbage. SVG is skipped (no renderer yet).
async fn fetch_image(client: &Client, url: Url) -> Option<Vec<u8>> {
    let response = client.get(url).send().await.ok()?.error_for_status().ok()?;
    let kind = content_type(&response)?;
    if !kind.starts_with("image/") || kind.starts_with("image/svg") {
        return None;
    }
    if response.content_length().is_some_and(|n| n > MAX_IMAGE as u64) {
        return None;
    }
    read_capped(response, MAX_IMAGE, false).await
}

/// Lower-cased `Content-Type` header.
fn content_type(response: &reqwest::Response) -> Option<String> {
    response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(|t| t.trim().to_ascii_lowercase())
}

/// Up to `cap` bytes of the body. Past the cap, keeps the first `cap` bytes if
/// `truncate`, else gives up.
async fn read_capped(mut response: reqwest::Response, cap: usize, truncate: bool) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > cap {
            if !truncate {
                return None;
            }
            bytes.truncate(cap);
            break;
        }
    }
    Some(bytes)
}

/// The page as text, in the charset from the `Content-Type` header, else a
/// `<meta charset>` near the top, else UTF-8. A BOM wins over both, as in browsers.
fn decode_html(bytes: &[u8], content_type: Option<&str>) -> String {
    let label = content_type.and_then(charset_param).or_else(|| meta_charset(&bytes[..bytes.len().min(2048)]));
    let encoding = label.and_then(|l| encoding_rs::Encoding::for_label(l.as_bytes())).unwrap_or(encoding_rs::UTF_8);
    encoding.decode(bytes).0.into_owned()
}

/// `charset` parameter of a `type/subtype; charset=x` value.
fn charset_param(value: &str) -> Option<String> {
    value.split(';').skip(1).find_map(|p| {
        let (k, v) = p.split_once('=')?;
        k.trim().eq_ignore_ascii_case("charset").then(|| v.trim().trim_matches(['"', '\'']).to_owned())
    })
}

/// `<meta charset="x">` or `<meta http-equiv="Content-Type" content="text/html; charset=x">`.
fn meta_charset(head: &[u8]) -> Option<String> {
    let head = String::from_utf8_lossy(head);
    let lower = head.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find("<meta") {
        let start = from + at;
        let end = start + lower[start..].find('>')?;
        let (tag, tag_lower) = (&head[start..end], &lower[start..end]);
        // `charset` may also match inside an http-equiv `content` value, so keep only
        // label characters.
        let found = attribute(tag, tag_lower, "charset").or_else(|| attribute(tag, tag_lower, "content").as_deref().and_then(charset_param));
        if let Some(charset) = found.map(|c| c.chars().take_while(|c| c.is_ascii_alphanumeric() || "-_.:".contains(*c)).collect::<String>()).filter(|c| !c.is_empty()) {
            return Some(charset);
        }
        from = end;
    }
    None
}

/// Title and site name from the page's metadata.
fn parse(html: &str) -> LinkMeta {
    let title = meta_content(html, "og:title").or_else(|| meta_content(html, "twitter:title")).or_else(|| title_tag(html));
    let site = meta_content(html, "og:site_name");
    LinkMeta { title: title.map(|t| clean(&t)).filter(|t| !t.is_empty()), site: site.map(|s| clean(&s)).filter(|s| !s.is_empty()), image: None }
}

fn image_url(html: &str) -> Option<String> {
    meta_content(html, "og:image").or_else(|| meta_content(html, "twitter:image")).map(|u| decode_entities(u.trim()))
}

/// `content` of the `<meta property|name="key">` tag, whatever the attribute order.
fn meta_content(html: &str, key: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find("<meta") {
        let start = from + at;
        let end = start + lower[start..].find('>')?;
        let tag = &html[start..end];
        let tag_lower = &lower[start..end];
        let named = ["property", "name"].iter().any(|attr| attribute(tag, tag_lower, attr).is_some_and(|v| v.eq_ignore_ascii_case(key)));
        if named {
            return attribute(tag, tag_lower, "content");
        }
        from = end;
    }
    None
}

/// Value of `attr="..."` (or single-quoted) in a tag.
fn attribute(tag: &str, tag_lower: &str, attr: &str) -> Option<String> {
    let mut from = 0;
    while let Some(at) = tag_lower[from..].find(attr) {
        let i = from + at;
        let before_ok = i == 0 || tag_lower.as_bytes()[i - 1].is_ascii_whitespace();
        let rest = tag_lower[i + attr.len()..].trim_start();
        if before_ok && rest.starts_with('=') {
            let value_start = tag.len() - rest.len() + 1;
            let value = tag[value_start..].trim_start();
            let quote = value.chars().next()?;
            if quote == '"' || quote == '\'' {
                let inner = &value[1..];
                return inner.find(quote).map(|e| inner[..e].to_string());
            }
            return value.split(|c: char| c.is_whitespace() || c == '/').next().map(str::to_owned);
        }
        from = i + attr.len();
    }
    None
}

fn title_tag(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let open_end = start + lower[start..].find('>')? + 1;
    let close = open_end + lower[open_end..].find("</title>")?;
    Some(html[open_end..close].to_string())
}

fn clean(text: &str) -> String {
    decode_entities(&text.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Named entities common in titles; anything else numeric is decoded generically.
const ENTITIES: &[(&str, char)] = &[
    ("amp", '&'), ("lt", '<'), ("gt", '>'), ("quot", '"'), ("apos", '\''), ("nbsp", ' '), ("mdash", '—'), ("ndash", '–'), ("hellip", '…'), ("lsquo", '‘'),
    ("rsquo", '’'), ("ldquo", '“'), ("rdquo", '”'), ("middot", '·'), ("bull", '•'), ("copy", '©'), ("reg", '®'), ("trade", '™'),
];

/// One pass over the text, so `&amp;lt;` becomes `&lt;`, not `<`. Unknown or malformed
/// entities are left as written.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let decoded = rest[1..].find(';').filter(|&end| end <= 10).and_then(|end| Some((entity(&rest[1..1 + end])?, end + 2)));
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    let code = if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
        u32::from_str_radix(hex, 16).ok()?
    } else if let Some(dec) = name.strip_prefix('#') {
        dec.parse().ok()?
    } else {
        return ENTITIES.iter().find(|(n, _)| *n == name).map(|&(_, c)| c);
    };
    char::from_u32(code).filter(|&c| c != '\0')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_open_graph_and_falls_back_to_the_title() {
        let html = r#"<html><head>
            <title>Fallback title</title>
            <meta content="Macfolio — portfolio templates" property="og:title">
            <meta property='og:site_name' content='macfolio.com'>
            <meta property="og:image" content="/img/cover.png?a=1&amp;b=2" />
        </head></html>"#;
        let meta = parse(html);
        assert_eq!(meta.title.as_deref(), Some("Macfolio — portfolio templates"));
        assert_eq!(meta.site.as_deref(), Some("macfolio.com"));
        assert_eq!(image_url(html).as_deref(), Some("/img/cover.png?a=1&b=2"));
        assert_eq!(parse("<title>\n  Just a   title </title>").title.as_deref(), Some("Just a title"));
    }

    #[test]
    fn never_fetches_private_or_local_hosts() {
        let ok = |u: &str| allowed(&Url::parse(u).unwrap());
        assert!(ok("https://www.figma.com/design/abc"));
        assert!(ok("http://8.8.8.8/x"));
        assert!(ok("http://[2606:4700::1111]/"));
        assert!(!ok("http://localhost:3000"));
        assert!(!ok("http://192.168.1.1/admin"));
        assert!(!ok("http://10.0.0.5"));
        assert!(!ok("http://169.254.169.254/latest/meta-data/"));
        assert!(!ok("http://100.64.1.1/"));
        assert!(!ok("http://[::1]/"));
        assert!(!ok("http://[::ffff:10.0.0.1]/"));
        assert!(!ok("http://[fe80::1]/"));
        assert!(!ok("http://0x7f000001/")); // URL parser normalises to 127.0.0.1
        assert!(!ok("http://nas.local/share"));
        assert!(!ok("http://intranet/wiki"));
        assert!(!ok("file:///C:/secret.txt"));
        assert!(!ok("https://user:pass@example.com/"));
    }

    #[test]
    fn public_ip_rejects_every_non_public_range() {
        let public = |s: &str| public_ip(s.parse().unwrap());
        for ip in ["8.8.8.8", "1.1.1.1", "140.82.112.3", "2606:4700::1111", "2a00:1450:4001::200e", "::ffff:8.8.8.8", "64:ff9b::808:808"] {
            assert!(public(ip), "{ip} should be public");
        }
        for ip in [
            "0.0.0.0", "0.1.2.3", "127.0.0.1", "10.0.0.1", "172.16.5.4", "192.168.0.1", "169.254.169.254", "100.64.1.1", "100.127.255.255", "255.255.255.255", "224.0.0.1",
            "192.0.2.1", "198.51.100.1", "203.0.113.1", "198.18.0.1", "192.0.0.1", "240.0.0.1", "::", "::1", "fe80::1", "fc00::1", "fd12:3456::1", "ff02::1", "2001:db8::1",
            "::ffff:10.0.0.1", "::ffff:127.0.0.1", "::10.0.0.1", "64:ff9b::a00:1", "2002:a00:1::1",
        ] {
            assert!(!public(ip), "{ip} should not be public");
        }
        assert!(public("100.128.0.1"), "just past CGNAT");
    }

    fn planned(u: &str) -> Plan {
        plan(&Url::parse(u).unwrap())
    }

    fn origin(u: &str) -> Plan {
        Plan::Origin(Url::parse(u).unwrap())
    }

    fn full(u: &str) -> Plan {
        Plan::Full(Url::parse(u).unwrap())
    }

    #[test]
    fn ordinary_links_are_fetched_whole() {
        for u in [
            "https://github.com/rust-lang/rust/pull/12345",
            "https://github.com/rust-lang/rust/issues?q=is%3Aopen",
            "https://www.figma.com/design/AbCdEfGhIjKlMnOpQrStUv/Landing-page?node-id=1-2",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42",
            "https://youtu.be/dQw4w9WgXcQ?si=abc",
            "https://en.wikipedia.org/wiki/Rust_(programming_language)",
            "https://example.com:8443/docs/login",
        ] {
            assert_eq!(planned(u), full(u), "{u}");
        }
    }

    #[test]
    fn fragment_is_stripped_from_the_request() {
        assert_eq!(planned("https://docs.rs/reqwest/latest/reqwest/#example"), full("https://docs.rs/reqwest/latest/reqwest/"));
        assert_eq!(planned("https://app.example.com/reset#token=abc"), origin("https://app.example.com/"));
    }

    #[test]
    fn secret_links_fetch_only_the_origin() {
        for (u, o) in [
            ("https://accounts.example.com/password/reset?token=8f3a9c", "https://accounts.example.com/"),
            ("https://example.com/users/password/edit?reset_password_token=abc", "https://example.com/"),
            ("https://app.example.com/reset-password/9f8e7d6c", "https://app.example.com/"),
            ("https://app.example.com/verify-email/abc", "https://app.example.com/"),
            ("https://app.example.com/auth/v1/verify?token=abc&type=magiclink", "https://app.example.com/"),
            ("https://app.example.com/magic-link/xyz", "https://app.example.com/"),
            ("https://app.slack.com/invite/abc", "https://app.slack.com/"),
            ("https://news.example.com/unsubscribe/42", "https://news.example.com/"),
            ("https://app.example.com/login/AbCdEfGhIjKlMnOpQrStUvWx", "https://app.example.com/"),
            ("https://app.example.com/s/eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.sig", "https://app.example.com/"),
            ("https://app.example.com/callback?code=4/0Adeu5BW&state=xyz", "https://app.example.com/"),
            ("https://example.com/cb?STATE=1", "https://example.com/"),
            ("https://example.com/api?apikey=1", "https://example.com/"),
            ("https://example.com/x?session=1", "https://example.com/"),
            ("https://example.com/x?OTP=123456", "https://example.com/"),
            (
                "https://bucket.s3.amazonaws.com/file.pdf?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=AKIA%2F&X-Amz-Signature=abc",
                "https://bucket.s3.amazonaws.com/",
            ),
            ("https://storage.googleapis.com/b/o?X-Goog-Signature=abc&X-Goog-Expires=900", "https://storage.googleapis.com/"),
            ("https://acct.blob.core.windows.net/c/f.png?sv=2022-11-02&se=2026-01-01&sp=r&sig=abc%3D", "https://acct.blob.core.windows.net/"),
            ("https://example.com:8443/reset?x=1", "https://example.com:8443/"),
        ] {
            assert_eq!(planned(u), origin(o), "{u}");
        }
    }

    #[test]
    fn credentials_or_unsafe_hosts_fetch_nothing() {
        assert_eq!(planned("https://user:pass@example.com/page"), Plan::Never);
        assert_eq!(planned("https://user@example.com/page"), Plan::Never);
        assert_eq!(planned("http://192.168.1.1/reset?token=1"), Plan::Never);
        assert_eq!(planned("ftp://example.com/"), Plan::Never);
    }

    #[test]
    fn decodes_entities_in_one_pass() {
        assert_eq!(decode_entities("A &#8212; B &#x2014; C &mdash; D"), "A — B — C — D");
        assert_eq!(decode_entities("Tom &amp; Jerry&#39;s &quot;show&quot;"), "Tom & Jerry's \"show\"");
        assert_eq!(decode_entities("&amp;lt;b&amp;gt;"), "&lt;b&gt;");
        assert_eq!(decode_entities("AT&T & co &bogus; &#xZZ; &#0; &"), "AT&T & co &bogus; &#xZZ; &#0; &");
        assert_eq!(decode_entities("caf&#233;"), "café");
    }

    #[test]
    fn decodes_in_the_page_charset() {
        let latin1 = b"<title>Caf\xe9 cr\xe8me</title>";
        assert_eq!(parse(&decode_html(latin1, Some("text/html; charset=ISO-8859-1"))).title.as_deref(), Some("Café crème"));
        let meta = b"<head><meta charset=\"windows-1252\"><title>\x93Quoted\x94</title>";
        assert_eq!(parse(&decode_html(meta, Some("text/html"))).title.as_deref(), Some("\u{201c}Quoted\u{201d}"));
        let equiv = b"<meta http-equiv=\"Content-Type\" content=\"text/html; charset=shift_jis\"><title>\x93\xfa\x96\x7b</title>";
        assert_eq!(parse(&decode_html(equiv, None)).title.as_deref(), Some("日本"));
        assert_eq!(parse(&decode_html("<title>naïve</title>".as_bytes(), None)).title.as_deref(), Some("naïve"));
    }
}
