//! What one `ReadDirectoryChangesW` buffer says, as [`Change`]s. Pure, so the record
//! walking (offsets, UTF-16 names, renames split over two records) is tested without a
//! folder.

use std::path::{Path, PathBuf};

/// A change in a watched folder, with full paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Added(PathBuf),
    Removed(PathBuf),
    /// Its size or last write changed (a download growing).
    Modified(PathBuf),
    /// Renamed within the folder: a download finishing, Chrome settling a name.
    Renamed { from: PathBuf, to: PathBuf },
    /// Windows dropped changes (the buffer overflowed); whoever cares looks again.
    Overflow(PathBuf),
}

const ADDED: u32 = 1;
const REMOVED: u32 = 2;
const MODIFIED: u32 = 3;
const RENAMED_OLD: u32 = 4;
const RENAMED_NEW: u32 = 5;

/// The `FILE_NOTIFY_INFORMATION` records in `buf` (the bytes the read returned) for
/// `folder`. A rename's two records become one [`Change::Renamed`]; an old name without its
/// new one (moved out) is a removal, a new name without its old one (moved in) an addition.
/// Repeated modifications of one file in the same buffer are reported once: a growing
/// file only needs to be looked at, not counted.
pub fn parse(folder: &Path, buf: &[u8]) -> Vec<Change> {
    let mut changes: Vec<Change> = Vec::new();
    let mut old: Option<PathBuf> = None;
    let mut at = 0usize;
    while let Some((next, action, name)) = record(buf, at) {
        let path = folder.join(name);
        match action {
            ADDED => changes.push(Change::Added(path)),
            REMOVED => changes.push(Change::Removed(path)),
            MODIFIED => {
                let change = Change::Modified(path);
                if !changes.contains(&change) {
                    changes.push(change);
                }
            }
            RENAMED_OLD => {
                if let Some(gone) = old.replace(path) {
                    changes.push(Change::Removed(gone));
                }
            }
            RENAMED_NEW => changes.push(match old.take() {
                Some(from) => Change::Renamed { from, to: path },
                None => Change::Added(path),
            }),
            _ => {}
        }
        if next == 0 {
            break;
        }
        at += next;
    }
    if let Some(gone) = old {
        changes.push(Change::Removed(gone));
    }
    changes
}

/// The record at `at`: (offset to the next, 0 for the last; action; file name).
fn record(buf: &[u8], at: usize) -> Option<(usize, u32, String)> {
    let word = |i: usize| buf.get(at + i..at + i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let (next, action, len) = (word(0)? as usize, word(4)?, word(8)? as usize);
    let bytes = buf.get(at + 12..at + 12 + len)?;
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some((next, action, String::from_utf16_lossy(&units)))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A buffer as Windows writes it: each record DWORD-aligned, the last with offset 0.
    pub fn buffer(records: &[(u32, &str)]) -> Vec<u8> {
        let mut buf = Vec::new();
        for (i, (action, name)) in records.iter().enumerate() {
            let name: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
            let len = (12 + name.len()).next_multiple_of(4);
            let next = if i + 1 == records.len() { 0 } else { len as u32 };
            buf.extend_from_slice(&next.to_le_bytes());
            buf.extend_from_slice(&action.to_le_bytes());
            buf.extend_from_slice(&(name.len() as u32).to_le_bytes());
            buf.extend_from_slice(&name);
            buf.resize(buf.len().next_multiple_of(4), 0);
        }
        buf
    }

    #[test]
    fn records_become_changes_with_full_paths() {
        let dir = Path::new(r"C:\Users\me\Downloads");
        let buf = buffer(&[(ADDED, "a.zip.crdownload"), (MODIFIED, "a.zip.crdownload"), (MODIFIED, "a.zip.crdownload"), (RENAMED_OLD, "a.zip.crdownload"), (RENAMED_NEW, "a.zip"), (REMOVED, "b.txt")]);
        assert_eq!(
            parse(dir, &buf),
            vec![
                Change::Added(dir.join("a.zip.crdownload")),
                Change::Modified(dir.join("a.zip.crdownload")),
                Change::Renamed { from: dir.join("a.zip.crdownload"), to: dir.join("a.zip") },
                Change::Removed(dir.join("b.txt")),
            ]
        );
    }

    #[test]
    fn half_renames_are_moves_in_and_out() {
        let dir = Path::new(r"D:\x");
        assert_eq!(parse(dir, &buffer(&[(RENAMED_NEW, "in.png")])), vec![Change::Added(dir.join("in.png"))]);
        assert_eq!(parse(dir, &buffer(&[(RENAMED_OLD, "out.png")])), vec![Change::Removed(dir.join("out.png"))]);
        assert_eq!(parse(dir, &buffer(&[(RENAMED_OLD, "a"), (RENAMED_OLD, "b"), (RENAMED_NEW, "c")])), vec![Change::Removed(dir.join("a")), Change::Renamed { from: dir.join("b"), to: dir.join("c") }]);
    }

    #[test]
    fn names_keep_their_unicode_and_a_cut_buffer_stops_cleanly() {
        let dir = Path::new(r"C:\d");
        let buf = buffer(&[(ADDED, "Résumé – 日本.pdf.part")]);
        assert_eq!(parse(dir, &buf), vec![Change::Added(dir.join("Résumé – 日本.pdf.part"))]);
        assert_eq!(parse(dir, &buf[..10]), Vec::new());
        assert_eq!(parse(dir, &[]), Vec::new());
    }
}
