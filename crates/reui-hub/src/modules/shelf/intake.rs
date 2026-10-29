//! What a drop becomes (PRD §5.2). Files are held by reference; files under a temporary
//! folder are rescued (copied into ReUI's folder) because whoever put them there (an
//! archive's preview, a browser's drag copy) deletes them soon; content with no file of its
//! own is saved under its own name; links and text are kept as they are. The plan is pure;
//! `carry_out` does the writing, off the hub thread.

use std::path::{Path, PathBuf};

use reui_protocol::{Dropped, Preview};

use super::store::{files_dir, What};
use crate::modules::clipboard::classify;

/// One card's worth of a drop.
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    /// The user's own files: hold the paths.
    Refer(Vec<String>),
    /// Files about to vanish from a temporary folder: copy them in first.
    Rescue(Vec<String>),
    /// Content with no file: save it as `name`.
    Save { name: String, bytes: Vec<u8> },
    Link(String),
    Text(String),
}

/// The temporary folders whose files don't last: `%TEMP%` (also what browsers and archive
/// tools use) and `%LOCALAPPDATA%\Temp`, lower-cased, without a trailing separator.
pub fn temp_roots() -> Vec<String> {
    let mut roots = vec![std::env::temp_dir().to_string_lossy().into_owned()];
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        roots.push(Path::new(&local).join("Temp").to_string_lossy().into_owned());
    }
    roots.into_iter().map(|r| r.trim_end_matches(['\\', '/']).to_lowercase()).filter(|r| !r.is_empty()).collect()
}

/// Whether `path` lies under one of `roots` (as [`temp_roots`] gives them). Paths compare
/// case-insensitively, as Windows does, and only on whole folder names.
pub fn is_transient(path: &str, roots: &[String]) -> bool {
    let lower = path.to_lowercase();
    roots.iter().any(|root| lower.strip_prefix(root.as_str()).is_some_and(|rest| rest.starts_with(['\\', '/'])))
}

/// The cards a drop makes: one for the user's files, one for rescued ones, then one per
/// other thing. Blank text and empty content make nothing.
pub fn plan(items: Vec<Dropped>, roots: &[String]) -> Vec<Plan> {
    let mut plans = Vec::new();
    for item in items {
        match item {
            Dropped::Files(paths) => {
                let (rescue, refer): (Vec<String>, Vec<String>) = paths.into_iter().filter(|p| !p.trim().is_empty()).partition(|p| is_transient(p, roots));
                if !refer.is_empty() {
                    plans.push(Plan::Refer(refer));
                }
                if !rescue.is_empty() {
                    plans.push(Plan::Rescue(rescue));
                }
            }
            Dropped::Data { name, bytes } if !bytes.is_empty() => plans.push(Plan::Save { name: safe_name(&name), bytes }),
            Dropped::Data { .. } => {}
            Dropped::Link(url) if !url.trim().is_empty() => plans.push(Plan::Link(url.trim().to_string())),
            Dropped::Link(_) => {}
            Dropped::Text(text) => match classify(&text) {
                // A dragged URL often comes as plain text.
                Preview::Link { .. } => plans.push(Plan::Link(text.trim().to_string())),
                _ if text.trim().is_empty() => {}
                _ => plans.push(Plan::Text(text)),
            },
        }
    }
    plans
}

/// A file name that's safe to create: no folders, no characters Windows refuses, no
/// trailing dots or spaces, not empty.
pub fn safe_name(name: &str) -> String {
    let base = name.rsplit(['\\', '/']).next().unwrap_or(name);
    let cleaned: String = base.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    let cleaned = cleaned.trim().trim_end_matches(['.', ' ']).to_string();
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '_') {
        "Dropped".into()
    } else {
        cleaned
    }
}

/// Does what `plan` says for card `id` under `root` (ReUI's shelf folder): writes or copies
/// what needs it. `None` when nothing could be kept (every copy failed).
pub fn carry_out(plan: Plan, id: u64, root: &Path) -> Option<What> {
    match plan {
        Plan::Refer(paths) => Some(What::Files { paths, owned: false, bytes: 0 }),
        Plan::Link(url) => Some(What::Link { url }),
        Plan::Text(text) => Some(What::Text { text }),
        Plan::Save { name, bytes } => {
            let dir = files_dir(root, id);
            let path = dir.join(&name);
            std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, &bytes)).map_err(|e| eprintln!("shelf: can't save {name}: {e}")).ok()?;
            Some(What::Files { paths: vec![path.to_string_lossy().into_owned()], owned: true, bytes: bytes.len() as u64 })
        }
        Plan::Rescue(paths) => {
            let dir = files_dir(root, id);
            _ = std::fs::create_dir_all(&dir);
            let mut kept = Vec::new();
            let mut bytes = 0;
            for path in paths {
                let to = unique(&dir, &safe_name(&path));
                match std::fs::copy(&path, &to) {
                    Ok(n) => {
                        bytes += n;
                        kept.push(to.to_string_lossy().into_owned());
                    }
                    // A folder, or already gone: nothing to rescue.
                    Err(e) => eprintln!("shelf: can't copy {path}: {e}"),
                }
            }
            if kept.is_empty() {
                _ = std::fs::remove_dir_all(&dir);
                return None;
            }
            Some(What::Files { paths: kept, owned: true, bytes })
        }
    }
}

/// `name` in `dir`, or "name (2).ext" if two rescued files share a name.
fn unique(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    (2..).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|p| !p.exists()).unwrap_or(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> Vec<String> {
        vec![r"c:\users\me\appdata\local\temp".into()]
    }

    #[test]
    fn temporary_files_are_told_apart_by_whole_folders() {
        let r = roots();
        assert!(is_transient(r"C:\Users\Me\AppData\Local\Temp\7zO1\Report.pdf", &r));
        assert!(!is_transient(r"C:\Users\Me\AppData\Local\Temporary Projects\a.txt", &r));
        assert!(!is_transient(r"C:\Users\Me\AppData\Local\Temp", &r), "the folder itself isn't in it");
        assert!(!is_transient(r"\\server\share\a.txt", &r));
    }

    #[test]
    fn a_drop_becomes_cards() {
        let drop = vec![
            Dropped::Files(vec![r"C:\docs\a.pdf".into(), r"C:\Users\me\AppData\Local\Temp\x\b.png".into(), r"D:\c.txt".into()]),
            Dropped::Text("  https://example.com/page  ".into()),
            Dropped::Text("Meeting notes\nSecond line".into()),
            Dropped::Text("   ".into()),
            Dropped::Link("https://x.dev".into()),
            Dropped::Data { name: "../cat?.png".into(), bytes: vec![1, 2, 3] },
            Dropped::Data { name: "empty.png".into(), bytes: vec![] },
        ];
        assert_eq!(
            plan(drop, &roots()),
            [
                Plan::Refer(vec![r"C:\docs\a.pdf".into(), r"D:\c.txt".into()]),
                Plan::Rescue(vec![r"C:\Users\me\AppData\Local\Temp\x\b.png".into()]),
                Plan::Link("https://example.com/page".into()),
                Plan::Text("Meeting notes\nSecond line".into()),
                Plan::Link("https://x.dev".into()),
                Plan::Save { name: "cat_.png".into(), bytes: vec![1, 2, 3] },
            ]
        );
    }

    #[test]
    fn names_are_safe_to_create() {
        assert_eq!(safe_name(r"C:\a\Report.pdf"), "Report.pdf");
        assert_eq!(safe_name("what: why?.txt"), "what_ why_.txt");
        assert_eq!(safe_name("trailing. . "), "trailing");
        assert_eq!(safe_name(""), "Dropped");
        assert_eq!(safe_name("???"), "Dropped");
    }

    #[test]
    fn saved_and_rescued_files_land_in_the_cards_folder() {
        let root = std::env::temp_dir().join(format!("reui-shelf-intake-{}", std::process::id()));
        let what = carry_out(Plan::Save { name: "Image.png".into(), bytes: vec![7; 10] }, 3, &root).unwrap();
        let What::Files { paths, owned: true, bytes: 10 } = &what else { panic!("{what:?}") };
        assert!(paths[0].ends_with(r"files\3\Image.png") && Path::new(&paths[0]).exists());

        let src = root.join("src");
        std::fs::create_dir_all(src.join("b")).unwrap();
        std::fs::write(src.join("a.txt"), b"hello").unwrap();
        std::fs::write(src.join("b").join("a.txt"), b"hi").unwrap();
        let from = |p: &Path| p.to_string_lossy().into_owned();
        let what = carry_out(Plan::Rescue(vec![from(&src.join("a.txt")), from(&src.join("b").join("a.txt")), from(&src.join("gone.txt"))]), 4, &root).unwrap();
        let What::Files { paths, owned: true, bytes: 7 } = &what else { panic!("{what:?}") };
        assert_eq!(paths.len(), 2);
        assert!(paths[1].ends_with("a (2).txt"), "{paths:?}");
        assert!(carry_out(Plan::Rescue(vec![from(&src.join("gone.txt"))]), 5, &root).is_none());
        assert!(!files_dir(&root, 5).exists());
        _ = std::fs::remove_dir_all(&root);
    }
}
