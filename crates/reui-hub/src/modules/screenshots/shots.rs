//! One screenshot, however it arrives. Snipping Tool puts a shot on the clipboard and,
//! moments later, saves it in `Pictures\Screenshots`; Win+PrtScn only saves it. The two
//! halves within a few seconds are one shot (one card, filled in as they come), and the
//! same file reported twice (created, then written) is one file. Pure, on a millisecond
//! clock the tests fake.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

/// A clipboard image and a saved file this close together are one shot.
pub const MERGE_MS: u64 = 3000;
/// How long the card stays (once closed, if it was opened).
pub const SHOW_FOR_MS: u64 = 6000;
/// A file's changes settle for this long before it's read: Snipping Tool writes in passes.
pub const SETTLE_MS: u64 = 300;
/// Files remembered, so one written twice isn't a second shot.
const SEEN: usize = 16;

/// A shot's small PNG and its pixel size, when known.
pub type Thumb = (Vec<u8>, Option<(u32, u32)>);

#[derive(Clone, Debug, PartialEq)]
pub struct Shot {
    pub id: u64,
    /// A small PNG of it, from the clipboard's capture or the file.
    pub thumb: Option<Vec<u8>>,
    /// Its pixel size.
    pub size: Option<(u32, u32)>,
    /// Where it was saved, once it was.
    pub file: Option<PathBuf>,
    /// It's on the clipboard (Snipping Tool put it there).
    pub copied: bool,
    at_ms: u64,
    until_ms: u64,
}

#[derive(Debug, Default)]
pub struct Shots {
    current: Option<Shot>,
    seen: VecDeque<PathBuf>,
    /// A file waiting for its writes to settle, and when it may be read.
    settling: Option<(PathBuf, u64)>,
    held: bool,
    next_id: u64,
}

/// A file a screenshot tool saves (Snipping Tool: PNG; JPEG when the user picked it).
pub fn is_image(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| ["png", "jpg", "jpeg"].iter().any(|x| e.eq_ignore_ascii_case(x)))
}

impl Shots {
    pub fn current(&self) -> Option<&Shot> {
        self.current.as_ref()
    }

    /// The current shot when this part belongs to it (recent, and `fits`: that half is
    /// still missing), else a new one; either way it gets its full time again.
    fn join(&mut self, now_ms: u64, fits: impl Fn(&Shot) -> bool) -> &mut Shot {
        let joins = self.current.as_ref().is_some_and(|s| now_ms <= s.at_ms + MERGE_MS && fits(s));
        if !joins {
            self.next_id += 1;
            self.current = Some(Shot { id: self.next_id, thumb: None, size: None, file: None, copied: false, at_ms: now_ms, until_ms: 0 });
        }
        let shot = self.current.as_mut().expect("just set");
        shot.until_ms = now_ms + SHOW_FOR_MS;
        shot
    }

    /// Snipping Tool put an image on the clipboard.
    pub fn copied(&mut self, thumb: Vec<u8>, size: Option<(u32, u32)>, now_ms: u64) {
        let shot = self.join(now_ms, |s| !s.copied);
        shot.copied = true;
        // The clipboard's thumbnail is ready now; a file's may not be yet.
        shot.thumb = Some(thumb);
        shot.size = size.or(shot.size);
    }

    /// The Screenshots folder changed at `path`. A new image file waits [`SETTLE_MS`]
    /// after its last write before it's read.
    pub fn file_changed(&mut self, path: &Path, now_ms: u64) {
        if !is_image(path) || self.seen.iter().any(|p| p == path) {
            return;
        }
        self.settling = Some((path.to_path_buf(), now_ms + SETTLE_MS));
    }

    /// The file to read now, once its writes have settled.
    pub fn due_read(&mut self, now_ms: u64) -> Option<PathBuf> {
        let (_, due) = self.settling.as_ref()?;
        if *due > now_ms {
            return None;
        }
        self.settling.take().map(|(path, _)| path)
    }

    /// The file was read: its thumbnail (and size), or `None` when it couldn't be decoded
    /// (still being written: the next change tries again).
    pub fn file_read(&mut self, path: PathBuf, thumb: Option<Thumb>, now_ms: u64) {
        let Some((thumb, size)) = thumb else { return };
        if self.seen.contains(&path) {
            return;
        }
        if self.seen.len() == SEEN {
            self.seen.pop_front();
        }
        self.seen.push_back(path.clone());
        let shot = self.join(now_ms, |s| s.file.is_none());
        shot.file = Some(path);
        shot.thumb.get_or_insert(thumb);
        shot.size = shot.size.or(size);
    }

    /// The shot's file went away (deleted, moved) before its card did.
    pub fn file_gone(&mut self, path: &Path) {
        if let Some(shot) = self.current.as_mut().filter(|s| s.file.as_deref() == Some(path)) {
            shot.file = None;
        }
    }

    pub fn tick(&mut self, now_ms: u64) {
        if !self.held && self.current.as_ref().is_some_and(|s| now_ms >= s.until_ms) {
            self.current = None;
        }
    }

    pub fn wake_at(&self) -> Option<u64> {
        let expires = self.current.as_ref().filter(|_| !self.held).map(|s| s.until_ms);
        expires.into_iter().chain(self.settling.as_ref().map(|(_, t)| *t)).min()
    }

    /// The card was opened (held) or closed: closing gives it its full time again.
    pub fn hold(&mut self, held: bool, now_ms: u64) {
        if self.held && !held {
            if let Some(shot) = self.current.as_mut() {
                shot.until_ms = now_ms + SHOW_FOR_MS;
            }
        }
        self.held = held;
    }

    /// Takes the card away (its file opened or shown).
    pub fn dismiss(&mut self) -> Option<Shot> {
        self.held = false;
        self.current.take()
    }
}

/// A PNG's pixel size, from its header (the IHDR chunk is always first).
pub fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 24 || !bytes.starts_with(SIGNATURE) || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let word = |at: usize| u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    Some((word(16), word(20)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> PathBuf {
        PathBuf::from(r"C:\Users\me\Pictures\Screenshots")
    }

    /// The file at `path` changing at `at`, then read when due.
    fn save(s: &mut Shots, path: &Path, at: u64) {
        s.file_changed(path, at);
        s.file_changed(path, at + 100);
        assert_eq!(s.due_read(at + 100 + SETTLE_MS - 1), None, "its writes settle first");
        let due = s.due_read(at + 100 + SETTLE_MS).expect("due once settled");
        s.file_read(due, Some((vec![9], Some((1920, 1080)))), at + 100 + SETTLE_MS);
    }

    #[test]
    fn a_snip_copied_then_saved_is_one_shot() {
        let mut s = Shots::default();
        s.copied(vec![1], Some((800, 600)), 0);
        let id = s.current().unwrap().id;
        save(&mut s, &dir().join("Screenshot 1.png"), 600);
        let shot = s.current().unwrap();
        assert_eq!((shot.id, shot.copied, shot.thumb.as_deref(), shot.size), (id, true, Some(&[1u8][..]), Some((800, 600))), "the clipboard's half stays");
        assert_eq!(shot.file.as_deref(), Some(dir().join("Screenshot 1.png").as_path()));
    }

    #[test]
    fn a_file_first_and_the_copy_after_is_one_shot_too() {
        let mut s = Shots::default();
        save(&mut s, &dir().join("a.png"), 0);
        s.copied(vec![1], None, 1_000);
        let shot = s.current().unwrap();
        assert_eq!((shot.id, shot.copied, shot.size, shot.thumb.as_deref()), (1, true, Some((1920, 1080)), Some(&[1u8][..])));
    }

    #[test]
    fn shots_apart_are_separate_and_a_file_written_twice_is_one() {
        let mut s = Shots::default();
        s.copied(vec![1], None, 0);
        s.copied(vec![2], None, 500);
        assert_eq!(s.current().unwrap().id, 2, "two copies are two shots");
        save(&mut s, &dir().join("a.png"), 10_000);
        assert_eq!((s.current().unwrap().id, s.current().unwrap().copied), (3, false), "too late to be the copy's file");
        s.file_changed(&dir().join("a.png"), 11_000);
        assert_eq!(s.due_read(20_000), None, "a file already seen isn't read again");
        save(&mut s, &dir().join("b.png"), 12_000);
        assert_eq!(s.current().unwrap().id, 4, "a second file is a second shot");
    }

    #[test]
    fn other_files_and_unreadable_ones_are_no_shot() {
        let mut s = Shots::default();
        s.file_changed(&dir().join("notes.txt"), 0);
        s.file_changed(&dir().join("desktop.ini"), 0);
        assert_eq!(s.due_read(10_000), None);
        s.file_changed(&dir().join("x.PNG"), 0);
        let due = s.due_read(SETTLE_MS).unwrap();
        s.file_read(due, None, SETTLE_MS);
        assert!(s.current().is_none(), "not decodable yet");
        save(&mut s, &dir().join("x.PNG"), 1_000);
        assert!(s.current().is_some(), "the next write tries again");
    }

    #[test]
    fn the_card_expires_unless_open_and_a_deleted_file_loses_its_buttons() {
        let mut s = Shots::default();
        s.copied(vec![1], None, 0);
        assert_eq!(s.wake_at(), Some(SHOW_FOR_MS));
        s.hold(true, 100);
        s.tick(SHOW_FOR_MS * 4);
        assert!(s.current().is_some());
        s.hold(false, 50_000);
        s.tick(50_000 + SHOW_FOR_MS);
        assert!(s.current().is_none());
        save(&mut s, &dir().join("a.png"), 60_000);
        s.file_gone(&dir().join("a.png"));
        assert_eq!(s.current().unwrap().file, None);
        assert!(s.dismiss().is_some() && s.current().is_none());
    }

    #[test]
    fn png_sizes_come_from_the_header() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&1920u32.to_be_bytes());
        png.extend_from_slice(&1080u32.to_be_bytes());
        assert_eq!(png_size(&png), Some((1920, 1080)));
        assert_eq!(png_size(&png[..20]), None);
        assert_eq!(png_size(b"GIF89a................."), None);
        assert!(is_image(Path::new("a.JPG")) && is_image(Path::new("a.jpeg")) && !is_image(Path::new("a.gif")) && !is_image(Path::new("png")));
    }
}
