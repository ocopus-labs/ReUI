//! Which files are downloads in progress. No browser tells Windows it's downloading; each
//! writes a partial file next to where the download will end up and renames it when it's
//! done, and the suffix says whose it is. Pure, tested.

use std::path::{Path, PathBuf};

/// Partial-file suffixes (lower case, with the dot) and who writes them.
const SUFFIXES: &[(&str, Writer)] = &[
    // Chrome, Edge, Brave, Vivaldi, Arc: "<name>.crdownload", first "Unconfirmed <n>.crdownload".
    (".crdownload", Writer::Chromium),
    // Opera keeps Chromium's machinery under its own suffix.
    (".opdownload", Writer::Chromium),
    // Firefox (and its forks): "<name>.part", beside a 0-byte "<name>" it fills in at the end.
    (".part", Writer::Firefox),
    // Older download managers and Edge Legacy.
    (".partial", Writer::Other),
    // qBittorrent, µTorrent and BitTorrent (incomplete-file suffix on).
    (".!qb", Writer::Other),
    (".!ut", Writer::Other),
];

/// Whose partial file it is; Chromium's unconfirmed names are told apart by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Writer {
    Chromium,
    Firefox,
    Other,
}

/// A download in progress, from its partial file's name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Partial {
    pub writer: Writer,
    /// The name it will have when it's done; `None` while Chromium hasn't settled it
    /// ("Unconfirmed 123456.crdownload").
    pub name: Option<String>,
}

/// `Some` when `file_name` is a partial download.
pub fn partial(file_name: &str) -> Option<Partial> {
    let ends_with = |suffix: &str| {
        let at = file_name.len().saturating_sub(suffix.len());
        at > 0 && file_name.is_char_boundary(at) && file_name[at..].eq_ignore_ascii_case(suffix)
    };
    let (suffix, writer) = SUFFIXES.iter().find(|(s, _)| ends_with(s))?;
    let stem = &file_name[..file_name.len() - suffix.len()];
    let unconfirmed = *writer == Writer::Chromium && stem.strip_prefix("Unconfirmed ").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    let name = (!unconfirmed && !stem.trim().is_empty()).then(|| stem.to_string());
    Some(Partial { writer: *writer, name })
}

/// The partial file at `path`, if it is one.
pub fn of_path(path: &Path) -> Option<Partial> {
    partial(&path.file_name()?.to_string_lossy())
}

/// Where the download at `path` will end up, when its name is known.
pub fn final_path(path: &Path) -> Option<PathBuf> {
    Some(path.with_file_name(of_path(path)?.name?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(file: &str) -> Option<String> {
        partial(file).and_then(|p| p.name)
    }

    #[test]
    fn each_browser_is_recognised_with_its_final_name() {
        assert_eq!(partial("setup.exe.crdownload"), Some(Partial { writer: Writer::Chromium, name: Some("setup.exe".into()) }));
        assert_eq!(partial("Report (1).PDF.CRDOWNLOAD").unwrap().name.as_deref(), Some("Report (1).PDF"));
        assert_eq!(partial("video.mp4.opdownload").unwrap().writer, Writer::Chromium);
        assert_eq!(partial("ubuntu.iso.part"), Some(Partial { writer: Writer::Firefox, name: Some("ubuntu.iso".into()) }));
        assert_eq!(named("old.zip.partial").as_deref(), Some("old.zip"));
        assert_eq!(named("linux.iso.!qB").as_deref(), Some("linux.iso"));
        assert_eq!(named("linux.iso.!ut").as_deref(), Some("linux.iso"));
        assert_eq!(named("Résumé – 日本.pdf.part").as_deref(), Some("Résumé – 日本.pdf"));
    }

    #[test]
    fn chromium_unconfirmed_names_are_not_yet_known() {
        assert_eq!(partial("Unconfirmed 812734.crdownload"), Some(Partial { writer: Writer::Chromium, name: None }));
        assert_eq!(named("Unconfirmed notes.txt.crdownload").as_deref(), Some("Unconfirmed notes.txt"), "a real file called Unconfirmed");
        assert_eq!(named("Unconfirmed 12.part").as_deref(), Some("Unconfirmed 12"), "only Chromium writes unconfirmed names");
    }

    #[test]
    fn finished_and_unrelated_files_are_not_downloads() {
        for name in ["setup.exe", "archive.part1.rar", "notes.part.txt", ".part", ".crdownload", "desktop.ini", "photo.png", "x.tmp"] {
            assert_eq!(partial(name), None, "{name}");
        }
    }

    #[test]
    fn paths_give_their_final_paths() {
        let dir = Path::new(r"C:\Users\me\Downloads");
        assert_eq!(final_path(&dir.join("a.zip.crdownload")), Some(dir.join("a.zip")));
        assert_eq!(final_path(&dir.join("Unconfirmed 1.crdownload")), None);
        assert_eq!(final_path(&dir.join("a.zip")), None);
    }
}
