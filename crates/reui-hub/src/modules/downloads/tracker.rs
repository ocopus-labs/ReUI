//! Downloads from the folder's changes: a partial file appearing, growing, being renamed to
//! its final name (done), or deleted (cancelled, unless its final name turns up). Sizes are
//! read by the module, at most once a second and only after a change; this decides what to
//! read and when, and what the island says. Pure, on a millisecond clock the tests fake.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use super::partial;
use crate::modules::folders::Change;

/// A partial file that's gone within this long (a tiny file, a cancel at the save prompt)
/// never shows as in progress; it still says Downloaded when it finishes.
pub const SHOW_AFTER_MS: u64 = 1000;
/// Sizes are read at most this often (DESIGN: progress once a second).
pub const READ_EVERY_MS: u64 = 1000;
/// The speed is the growth over about this long, so one slow write doesn't swing it.
const SPEED_WINDOW_MS: u64 = 5000;
/// No growth for this long and the speed is left out: a stalled download shows its size,
/// not a number falling towards zero.
pub const STALL_MS: u64 = 5000;
/// A partial file that hasn't grown for this long is paused (Chrome keeps the file) or
/// abandoned (a crash left it): it stops showing until it grows again.
pub const QUIET_MS: u64 = 30_000;
/// A deleted partial file may be a rename done as copy-and-delete: its final name has this
/// long to appear before it counts as cancelled.
pub const VANISH_GRACE_MS: u64 = 1000;
/// How long a Downloaded card stays (once closed, if it was opened).
pub const DONE_FOR_MS: u64 = 6000;

/// A download in progress.
#[derive(Clone, Debug, PartialEq)]
pub struct Download {
    pub id: u64,
    /// Its partial file now.
    pub path: PathBuf,
    /// Its final name, once the browser has settled it.
    pub name: Option<String>,
    pub size: u64,
    /// The full size. No browser's partial file records it (PRD §7), so this stays `None`
    /// until a source that knows it exists; the views handle both.
    pub total: Option<u64>,
    pub started_ms: u64,
    /// (when, size) at each read that saw it grow, oldest first, within the speed window.
    samples: VecDeque<(u64, u64)>,
}

impl Download {
    /// Bytes a second over the last few seconds; `None` before there are two readings a
    /// second apart, and once it has stalled.
    pub fn speed(&self, now_ms: u64) -> Option<f64> {
        let (&(t0, s0), &(t1, s1)) = (self.samples.front()?, self.samples.back()?);
        (t1 >= t0 + READ_EVERY_MS && now_ms < t1 + STALL_MS).then(|| (s1 - s0) as f64 * 1000.0 / (t1 - t0) as f64)
    }

    /// How far along, when the total is known.
    pub fn fraction(&self) -> Option<f32> {
        self.total.filter(|&t| t > 0).map(|t| (self.size as f64 / t as f64).clamp(0.0, 1.0) as f32)
    }

    /// When its speed stops being shown, if nothing grows before then.
    fn stalls_at(&self) -> Option<u64> {
        self.samples.back().map(|&(t, _)| t + STALL_MS)
    }

    /// When it stops showing, if nothing grows before then.
    fn quiet_at(&self) -> u64 {
        self.samples.back().map_or(self.started_ms, |&(t, _)| t.max(self.started_ms)) + QUIET_MS
    }
}

/// A finished download, for its Downloaded card.
#[derive(Clone, Debug, PartialEq)]
pub struct Done {
    pub id: u64,
    pub path: PathBuf,
    pub size: u64,
    /// Read once at its final path, so the size is the file's, not the last reading.
    sized: bool,
    /// Its card goes then, unless held open.
    until_ms: u64,
}

impl Done {
    pub fn name(&self) -> String {
        self.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    }
}

/// A partial file that's gone; done if its final name appears in time, else cancelled.
#[derive(Debug)]
struct Vanished {
    download: Download,
    expect: Option<PathBuf>,
    until_ms: u64,
}

#[derive(Debug, Default)]
pub struct Tracker {
    active: Vec<Download>,
    vanished: Vec<Vanished>,
    done: Vec<Done>,
    /// Something changed since the last read.
    dirty: bool,
    last_read_ms: Option<u64>,
    /// The Downloaded card is open: nothing on it expires.
    held: bool,
    next_id: u64,
}

impl Tracker {
    pub fn on_change(&mut self, change: &Change, now_ms: u64) {
        match change {
            Change::Added(path) | Change::Modified(path) => {
                if partial::of_path(path).is_some() {
                    self.track(path, now_ms);
                } else if let Some(at) = self.vanished.iter().position(|v| v.expect.as_deref() == Some(path)) {
                    let v = self.vanished.remove(at);
                    self.finish(v.download, path.clone(), now_ms);
                }
            }
            Change::Renamed { from, to } => match self.active.iter().position(|d| &d.path == from) {
                Some(at) if partial::of_path(to).is_some() => {
                    let d = &mut self.active[at];
                    d.path = to.clone();
                    d.name = partial::of_path(to).and_then(|p| p.name);
                }
                Some(at) => {
                    let d = self.active.remove(at);
                    self.finish(d, to.clone(), now_ms);
                }
                None if partial::of_path(to).is_some() => self.track(to, now_ms),
                None => {}
            },
            Change::Removed(path) => self.vanish(path, now_ms),
            // Look at everything again; a file that's gone reads as `None`.
            Change::Overflow(_) => self.dirty = true,
        }
    }

    fn track(&mut self, path: &Path, now_ms: u64) {
        self.dirty = true;
        if self.active.iter().any(|d| d.path == path) {
            return;
        }
        self.next_id += 1;
        let name = partial::of_path(path).and_then(|p| p.name);
        self.active.push(Download { id: self.next_id, path: path.into(), name, size: 0, total: None, started_ms: now_ms, samples: VecDeque::new() });
    }

    fn vanish(&mut self, path: &Path, now_ms: u64) {
        if let Some(at) = self.active.iter().position(|d| d.path == path) {
            let download = self.active.remove(at);
            self.vanished.push(Vanished { expect: partial::final_path(path), download, until_ms: now_ms + VANISH_GRACE_MS });
        }
    }

    /// Done at `path`. The same file finishing twice (a rename seen, then its final name
    /// written to) is one card.
    fn finish(&mut self, download: Download, path: PathBuf, now_ms: u64) {
        self.dirty = true;
        self.done.retain(|d| d.path != path);
        self.done.push(Done { id: download.id, path, size: download.size, sized: false, until_ms: now_ms + DONE_FOR_MS });
    }

    /// When sizes should next be read, if anything changed.
    fn read_at(&self) -> Option<u64> {
        let changed = self.dirty || self.vanished.iter().any(|v| v.expect.is_some());
        changed.then(|| self.last_read_ms.map_or(0, |t| t + READ_EVERY_MS))
    }

    /// The paths to read sizes of now (`None` when it's not time), marking them read.
    pub fn due_reads(&mut self, now_ms: u64) -> Option<Vec<PathBuf>> {
        if self.read_at()? > now_ms {
            return None;
        }
        self.dirty = false;
        self.last_read_ms = Some(now_ms);
        let active = self.active.iter().map(|d| d.path.clone());
        let done = self.done.iter().filter(|d| !d.sized).map(|d| d.path.clone());
        let expected = self.vanished.iter().filter_map(|v| v.expect.clone());
        Some(active.chain(done).chain(expected).collect())
    }

    /// The sizes read (`None`: no such file).
    pub fn on_sizes(&mut self, sizes: &[(PathBuf, Option<u64>)], now_ms: u64) {
        for (path, size) in sizes {
            if let Some(d) = self.active.iter_mut().find(|d| &d.path == path) {
                match *size {
                    Some(size) => {
                        if d.samples.back().is_none_or(|&(_, s)| size > s) {
                            d.samples.push_back((now_ms, size));
                            while d.samples.front().is_some_and(|&(t, _)| t + SPEED_WINDOW_MS < now_ms) && d.samples.len() > 2 {
                                d.samples.pop_front();
                            }
                        }
                        d.size = size;
                    }
                    // Gone without our hearing of it (an overflowed buffer).
                    None => self.vanish(&path.clone(), now_ms),
                }
            } else if let Some(d) = self.done.iter_mut().find(|d| &d.path == path) {
                d.sized = true;
                d.size = size.unwrap_or(d.size);
            } else if let (Some(size), Some(at)) = (size, self.vanished.iter().position(|v| v.expect.as_ref() == Some(path))) {
                let mut v = self.vanished.remove(at);
                v.download.size = *size;
                self.finish(v.download, path.clone(), now_ms);
            }
        }
    }

    /// Lets time pass: cancelled downloads are dropped, Downloaded cards expire.
    pub fn tick(&mut self, now_ms: u64) {
        // An expected final name gets one read after its grace before it's given up.
        let read = self.last_read_ms.unwrap_or(0);
        self.vanished.retain(|v| now_ms < v.until_ms || (v.expect.is_some() && read < v.until_ms));
        if !self.held {
            self.done.retain(|d| now_ms < d.until_ms);
        }
    }

    /// The next time something changes without an event: a read, a download showing, a
    /// speed stalling, a cancel or a card expiring.
    pub fn wake_at(&self, now_ms: u64) -> Option<u64> {
        let reads = self.read_at();
        let shows = self.active.iter().map(|d| d.started_ms + SHOW_AFTER_MS).filter(|&t| t > now_ms);
        let stalls = self.active.iter().filter_map(Download::stalls_at).chain(self.active.iter().map(Download::quiet_at)).filter(|&t| t > now_ms);
        let vanished = self.vanished.iter().map(|v| v.until_ms);
        let expires = self.done.iter().filter(|_| !self.held).map(|d| d.until_ms);
        reads.into_iter().chain(shows).chain(stalls).chain(vanished).chain(expires).min()
    }

    /// Downloads to show as in progress, newest first: past their first second, and not
    /// quiet for long.
    pub fn showing(&self, now_ms: u64) -> Vec<&Download> {
        self.active.iter().rev().filter(|d| now_ms >= d.started_ms + SHOW_AFTER_MS && now_ms < d.quiet_at()).collect()
    }

    /// Finished downloads, newest first.
    pub fn done(&self) -> Vec<&Done> {
        self.done.iter().rev().collect()
    }

    /// Takes a finished download off its card (opened, shown in its folder).
    pub fn dismiss(&mut self, id: u64) -> Option<Done> {
        let at = self.done.iter().position(|d| d.id == id)?;
        Some(self.done.remove(at))
    }

    pub fn active_path(&self, id: u64) -> Option<&Path> {
        self.active.iter().find(|d| d.id == id).map(|d| d.path.as_path())
    }

    /// The Downloaded card was opened (held) or closed: closing gives what's on it its
    /// full time again, from now.
    pub fn hold(&mut self, held: bool, now_ms: u64) {
        if self.held && !held {
            for d in &mut self.done {
                d.until_ms = now_ms + DONE_FOR_MS;
            }
        }
        self.held = held;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> PathBuf {
        PathBuf::from(r"C:\Users\me\Downloads")
    }

    /// Reads whatever is due at `now` with `size` for every path.
    fn read(t: &mut Tracker, now: u64, size: impl Fn(&Path) -> Option<u64>) {
        if let Some(paths) = t.due_reads(now) {
            let sizes: Vec<_> = paths.iter().map(|p| (p.clone(), size(p))).collect();
            t.on_sizes(&sizes, now);
        }
        t.tick(now);
    }

    #[test]
    fn a_chrome_download_grows_settles_its_name_and_finishes() {
        let mut t = Tracker::default();
        let unconfirmed = dir().join("Unconfirmed 4711.crdownload");
        let named = dir().join("big.iso.crdownload");
        t.on_change(&Change::Added(unconfirmed.clone()), 0);
        assert_eq!(t.wake_at(0), Some(0), "the first size is read at once");
        read(&mut t, 0, |_| Some(0));
        assert!(t.showing(500).is_empty(), "not for its first second");
        assert_eq!(t.wake_at(0), Some(SHOW_AFTER_MS));
        t.on_change(&Change::Renamed { from: unconfirmed, to: named.clone() }, 600);
        assert_eq!(t.showing(1_000)[0].name.as_deref(), Some("big.iso"));
        t.on_change(&Change::Modified(named.clone()), 700);
        assert_eq!(t.due_reads(700), None, "a second after the last read, not before");
        read(&mut t, 1_000, |_| Some(3_000_000));
        t.on_change(&Change::Modified(named.clone()), 1_500);
        read(&mut t, 2_000, |_| Some(6_000_000));
        let d = t.showing(2_000)[0];
        assert_eq!((d.size, d.total, d.fraction()), (6_000_000, None, None));
        assert_eq!(d.speed(2_000), Some(3_000_000.0));
        t.on_change(&Change::Renamed { from: named, to: dir().join("big.iso") }, 2_500);
        assert!(t.showing(2_500).is_empty());
        read(&mut t, 3_000, |_| Some(6_500_000));
        let done = t.done();
        assert_eq!((done[0].name(), done[0].size), ("big.iso".to_string(), 6_500_000), "the final file's own size");
        t.tick(2_500 + DONE_FOR_MS);
        assert!(t.done().is_empty(), "the card goes after its time");
    }

    #[test]
    fn firefox_finishes_by_renaming_its_part_over_the_placeholder() {
        let mut t = Tracker::default();
        let part = dir().join("ubuntu.iso.part");
        t.on_change(&Change::Added(dir().join("ubuntu.iso")), 0); // the 0-byte placeholder
        t.on_change(&Change::Added(part.clone()), 0);
        read(&mut t, 0, |_| Some(10));
        t.on_change(&Change::Removed(dir().join("ubuntu.iso")), 4_000);
        t.on_change(&Change::Renamed { from: part, to: dir().join("ubuntu.iso") }, 4_000);
        assert_eq!(t.done().len(), 1);
        assert_eq!(t.showing(4_000).len(), 0);
    }

    #[test]
    fn a_deleted_partial_is_cancelled_unless_its_final_name_appears() {
        let mut t = Tracker::default();
        let a = dir().join("a.zip.crdownload");
        t.on_change(&Change::Added(a.clone()), 0);
        read(&mut t, 0, |_| Some(5));
        t.on_change(&Change::Removed(a.clone()), 2_000);
        assert_eq!(t.wake_at(2_000), Some(READ_EVERY_MS), "a read is due: the final name may be there");
        read(&mut t, 2_000, |_| None);
        read(&mut t, 2_000 + VANISH_GRACE_MS, |_| None);
        read(&mut t, 4_000, |_| None);
        assert!(t.done().is_empty() && t.showing(4_000).is_empty() && t.wake_at(4_000).is_none(), "cancelled, and nothing left to wake for");

        // Copy-and-delete: the final name turns up.
        t.on_change(&Change::Added(a.clone()), 10_000);
        t.on_change(&Change::Removed(a), 12_000);
        t.on_change(&Change::Added(dir().join("a.zip")), 12_300);
        assert_eq!(t.done()[0].name(), "a.zip");
    }

    #[test]
    fn a_final_file_found_on_the_last_read_still_finishes() {
        let mut t = Tracker::default();
        let a = dir().join("a.zip.crdownload");
        t.on_change(&Change::Added(a.clone()), 0);
        read(&mut t, 0, |_| Some(5));
        t.on_change(&Change::Removed(a), 5_000);
        // Its rename was lost (an overflow); the read finds the final file.
        read(&mut t, 5_000, |p| (p.extension().unwrap() == "zip").then_some(42));
        assert_eq!(t.done()[0].size, 42);
    }

    #[test]
    fn a_stalled_download_drops_its_speed_and_stays() {
        let mut t = Tracker::default();
        let a = dir().join("a.bin.part");
        t.on_change(&Change::Added(a.clone()), 0);
        read(&mut t, 0, |_| Some(0));
        t.on_change(&Change::Modified(a.clone()), 900);
        read(&mut t, 1_000, |_| Some(2_000));
        assert_eq!(t.showing(1_000)[0].speed(1_000), Some(2_000.0));
        assert_eq!(t.wake_at(1_000), Some(1_000 + STALL_MS));
        assert_eq!(t.showing(1_000 + STALL_MS)[0].speed(1_000 + STALL_MS), None);
        t.on_change(&Change::Modified(a.clone()), 9_000);
        read(&mut t, 9_000, |_| Some(2_000));
        assert_eq!(t.showing(9_000)[0].speed(9_000), None, "the same size is no growth");
        assert_eq!(t.wake_at(9_000), Some(1_000 + QUIET_MS));
        assert!(t.showing(1_000 + QUIET_MS).is_empty(), "paused or abandoned: it stops showing");
        t.on_change(&Change::Modified(a.clone()), 40_000);
        read(&mut t, 40_000, |_| Some(9_000));
        assert_eq!(t.showing(40_000).len(), 1, "and comes back when it grows");
    }

    #[test]
    fn the_speed_averages_over_the_last_seconds() {
        let mut t = Tracker::default();
        let a = dir().join("a.bin.part");
        t.on_change(&Change::Added(a.clone()), 0);
        for s in 0..=10u64 {
            t.on_change(&Change::Modified(a.clone()), s * 1_000);
            read(&mut t, s * 1_000, |_| Some(s * if s < 6 { 1_000 } else { 3_000 }));
        }
        // 6 s: 18 000 bytes, 10 s: 30 000, over the window's 5 s readings (5 s → 10 s).
        let speed = t.showing(10_000)[0].speed(10_000).unwrap();
        assert!((speed - (30_000.0 - 5_000.0) / 5.0).abs() < 1.0, "{speed}");
    }

    #[test]
    fn an_open_card_stays_and_gets_its_time_again_when_closed() {
        let mut t = Tracker::default();
        let a = dir().join("a.zip.crdownload");
        t.on_change(&Change::Added(a.clone()), 0);
        t.on_change(&Change::Renamed { from: a, to: dir().join("a.zip") }, 100);
        t.hold(true, 200);
        t.tick(100 + DONE_FOR_MS * 3);
        assert_eq!(t.done().len(), 1);
        t.hold(false, 50_000);
        t.tick(50_000 + DONE_FOR_MS - 1);
        assert_eq!(t.done().len(), 1);
        t.tick(50_000 + DONE_FOR_MS);
        assert!(t.done().is_empty());
    }

    #[test]
    fn several_downloads_show_newest_first_and_finish_once_each() {
        let mut t = Tracker::default();
        for (i, name) in ["a.zip.crdownload", "b.zip.crdownload"].iter().enumerate() {
            t.on_change(&Change::Added(dir().join(name)), i as u64 * 100);
            t.on_change(&Change::Added(dir().join(name)), i as u64 * 100 + 50);
        }
        let showing: Vec<_> = t.showing(2_000).iter().map(|d| d.name.clone().unwrap()).collect();
        assert_eq!(showing, ["b.zip", "a.zip"]);
        t.on_change(&Change::Renamed { from: dir().join("a.zip.crdownload"), to: dir().join("a.zip") }, 3_000);
        t.on_change(&Change::Modified(dir().join("a.zip")), 3_100);
        assert_eq!(t.done().len(), 1);
        let id = t.done()[0].id;
        assert_eq!(t.dismiss(id).map(|d| d.name()), Some("a.zip".into()));
        assert!(t.done().is_empty());
    }

    #[test]
    fn other_files_in_downloads_are_ignored() {
        let mut t = Tracker::default();
        t.on_change(&Change::Added(dir().join("notes.txt")), 0);
        t.on_change(&Change::Renamed { from: dir().join("x.txt"), to: dir().join("y.txt") }, 0);
        t.on_change(&Change::Removed(dir().join("y.txt")), 0);
        assert!(t.wake_at(0).is_none() && t.showing(10_000).is_empty() && t.done().is_empty());
    }

    #[test]
    fn a_known_total_gives_a_fraction() {
        let d = Download { id: 1, path: dir(), name: None, size: 25, total: Some(100), started_ms: 0, samples: VecDeque::new() };
        assert_eq!(d.fraction(), Some(0.25));
        assert_eq!(Download { total: Some(0), ..d.clone() }.fraction(), None);
        assert_eq!(Download { size: 200, ..d }.fraction(), Some(1.0));
    }
}
