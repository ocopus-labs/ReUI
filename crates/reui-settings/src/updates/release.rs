//! Releases as GitHub describes them, without any networking: the latest release's JSON
//! (versions compare the semver way, in `reui_protocol::version`), the checksum file,
//! release notes and CHANGELOG entries as plain text.

use serde_json::Value;

/// Release notes are shown in full up to this many characters.
const NOTES_MAX: usize = 2000;

#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// Without the tag's `v`.
    pub version: String,
    /// Plain text.
    pub notes: String,
    /// `ReUI-Setup-<version>.exe`.
    pub setup: Option<Asset>,
    /// `sha256sums.txt`, when the release has one.
    pub sums: Option<Asset>,
}

/// The release the GitHub API returns (`/repos/<owner>/<repo>/releases/latest`).
pub fn parse(json: &str) -> Result<Release, String> {
    let root: Value = serde_json::from_str(json).map_err(|e| format!("GitHub sent something unexpected ({e})"))?;
    let tag = root.get("tag_name").and_then(Value::as_str).ok_or("GitHub's answer has no version")?;
    let version = tag.trim().trim_start_matches(['v', 'V']).to_string();
    if !reui_protocol::version::valid(&version) {
        return Err(format!("{tag} isn't a version"));
    }
    let assets: Vec<Asset> = root
        .get("assets")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|a| Some(Asset { name: a.get("name")?.as_str()?.to_string(), url: a.get("browser_download_url")?.as_str()?.to_string(), size: a.get("size").and_then(Value::as_u64).unwrap_or(0) }))
                .collect()
        })
        .unwrap_or_default();
    let wanted = format!("reui-setup-{version}.exe").to_ascii_lowercase();
    let setup = assets.iter().find(|a| a.name.to_ascii_lowercase() == wanted).cloned();
    let sums = assets.iter().find(|a| a.name.eq_ignore_ascii_case("sha256sums.txt")).cloned();
    let notes = plain(root.get("body").and_then(Value::as_str).unwrap_or_default());
    Ok(Release { version, notes, setup, sums })
}

pub use reui_protocol::version::newer;

/// The hash `sha256sum` lists for `file` ("<hex>  <name>" or "<hex> *<name>"), lower case.
pub fn sha256_for(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, name) = line.trim().split_once(char::is_whitespace)?;
        let name = name.trim().trim_start_matches('*');
        let valid = hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit());
        (valid && name.eq_ignore_ascii_case(file)).then(|| hash.to_ascii_lowercase())
    })
}

/// Markdown as plain text: no heading marks, emphasis or link targets; bullets as "•";
/// at most a few paragraphs' worth.
pub fn plain(markdown: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for line in markdown.lines() {
        let line = line.trim_end();
        let trimmed = line.trim_start();
        if trimmed.starts_with("<!--") || trimmed.starts_with("```") {
            continue;
        }
        let text = if let Some(rest) = trimmed.strip_prefix(['-', '*', '+']).filter(|r| r.starts_with(' ')) {
            format!("• {}", inline(rest.trim()))
        } else {
            inline(trimmed.trim_start_matches('#').trim())
        };
        // One blank line at most between paragraphs.
        if text.is_empty() && out.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        out.push(text);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    let text = out.join("\n");
    match text.char_indices().nth(NOTES_MAX) {
        Some((cut, _)) => format!("{}…", text[..cut].trim_end()),
        None => text,
    }
}

/// `**bold**`, `` `code` `` and `[text](url)` → their text.
fn inline(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        match after.find("](").and_then(|close| after[close + 2..].find(')').map(|end| (close, close + 2 + end))) {
            Some((close, end)) => {
                out.push_str(&rest[..open]);
                out.push_str(&after[..close]);
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[..=open]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.replace("**", "").replace("__", "").replace('`', "")
}

/// The CHANGELOG's entry for `version` ("## 0.2.0 (date)" up to the next "## "), as plain
/// text.
pub fn changelog_entry(changelog: &str, version: &str) -> Option<String> {
    let mut lines = changelog.lines();
    lines.by_ref().find(|l| {
        let Some(title) = l.strip_prefix("## ") else { return false };
        let first = title.split_whitespace().next().unwrap_or_default().trim_start_matches(['v', 'V']);
        first == version
    })?;
    let body: Vec<&str> = lines.take_while(|l| !l.starts_with("## ")).collect();
    let text = plain(&body.join("\n"));
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_latest_release_is_read_from_githubs_json() {
        let json = r###"{
            "tag_name": "v0.2.0",
            "html_url": "https://github.com/o/r/releases/tag/v0.2.0",
            "body": "## What's changed\r\n\r\n* **Updates** from Settings by @someone in [#12](https://github.com/o/r/pull/12)\r\n- `--welcome` tour\r\n\r\n\r\n<!-- hidden -->\r\nThanks!",
            "assets": [
                { "name": "ReUI-0.2.0-x64.zip", "browser_download_url": "https://github.com/o/r/releases/download/v0.2.0/ReUI-0.2.0-x64.zip", "size": 9000000 },
                { "name": "ReUI-Setup-0.2.0.exe", "browser_download_url": "https://github.com/o/r/releases/download/v0.2.0/ReUI-Setup-0.2.0.exe", "size": 8123456 },
                { "name": "sha256sums.txt", "browser_download_url": "https://github.com/o/r/releases/download/v0.2.0/sha256sums.txt", "size": 200 }
            ]
        }"###;
        let release = parse(json).unwrap();
        assert_eq!(release.version, "0.2.0");
        let setup = release.setup.unwrap();
        assert_eq!((setup.name.as_str(), setup.size), ("ReUI-Setup-0.2.0.exe", 8_123_456));
        assert!(setup.url.ends_with("/ReUI-Setup-0.2.0.exe"));
        assert_eq!(release.sums.unwrap().name, "sha256sums.txt");
        assert_eq!(release.notes, "What's changed\n\n• Updates from Settings by @someone in #12\n• --welcome tour\n\nThanks!");

        let bare = parse(r#"{ "tag_name": "0.3.0", "assets": [] }"#).unwrap();
        assert_eq!((bare.setup, bare.sums, bare.notes.as_str()), (None, None, ""));
        assert!(parse(r#"{ "message": "Not Found" }"#).is_err());
        assert!(parse(r#"{ "tag_name": "nightly" }"#).is_err());
        assert!(parse("<html>").is_err());
    }

    #[test]
    fn checksums_are_found_by_file_name() {
        let hash = "a".repeat(64);
        let sums = format!("{}  ReUI-0.2.0-x64.zip\n{} *ReUI-Setup-0.2.0.exe\n", "b".repeat(64), hash.to_uppercase());
        assert_eq!(sha256_for(&sums, "ReUI-Setup-0.2.0.exe"), Some(hash));
        assert_eq!(sha256_for(&sums, "ReUI-Setup-0.3.0.exe"), None);
        assert_eq!(sha256_for("xyz  ReUI-Setup-0.2.0.exe", "ReUI-Setup-0.2.0.exe"), None);
    }

    #[test]
    fn changelog_entries_are_found_by_version() {
        let changelog = "# Changelog\n\n## 0.2.0 (Nov 2026)\n\n- Updates\n- The tour\n\n## 0.1.0 (unreleased)\n\nFirst public build.\n";
        assert_eq!(changelog_entry(changelog, "0.2.0").as_deref(), Some("• Updates\n• The tour"));
        assert_eq!(changelog_entry(changelog, "0.1.0").as_deref(), Some("First public build."));
        assert_eq!(changelog_entry(changelog, "0.1"), None);
        assert_eq!(changelog_entry(changelog, "0.3.0"), None);
    }

    #[test]
    fn long_notes_are_cut() {
        let notes = plain(&"word ".repeat(1000));
        assert!(notes.chars().count() <= NOTES_MAX + 1 && notes.ends_with('…'));
    }
}
