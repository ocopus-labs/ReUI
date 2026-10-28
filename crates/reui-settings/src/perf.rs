//! `--perf`: frame statistics written once a second (while frames happen) to
//! `settings-perf.log` in the local folder: frames drawn, CPU time per frame (average and
//! worst), page layouts run and DirectWrite text layouts created. Off by default, and then
//! every call here is a cheap check.

use std::cell::RefCell;
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Default)]
struct Stats {
    frames: u32,
    total_ms: f64,
    max_ms: f64,
    layouts: u32,
    texts: u32,
    /// Why frames were drawn: (reason, count).
    reasons: Vec<(&'static str, u32)>,
    /// Where frame time went: (part, total ms).
    parts: Vec<(&'static str, f64)>,
}

struct Perf {
    path: PathBuf,
    since: Instant,
    stats: Stats,
}

thread_local! {
    static PERF: RefCell<Option<Perf>> = const { RefCell::new(None) };
}

pub fn init(path: PathBuf) {
    if let Some(dir) = path.parent() {
        _ = std::fs::create_dir_all(dir);
    }
    _ = std::fs::write(&path, "");
    PERF.with(|p| *p.borrow_mut() = Some(Perf { path, since: Instant::now(), stats: Stats::default() }));
}

pub fn enabled() -> bool {
    PERF.with(|p| p.borrow().is_some())
}

fn with(f: impl FnOnce(&mut Perf)) {
    PERF.with(|p| {
        if let Some(perf) = p.borrow_mut().as_mut() {
            f(perf);
        }
    });
}

/// A frame took `ms` of CPU on the UI thread (building it, drawing it, presenting it).
pub fn frame(ms: f64) {
    with(|p| {
        p.stats.frames += 1;
        p.stats.total_ms += ms;
        p.stats.max_ms = p.stats.max_ms.max(ms);
        if p.since.elapsed().as_secs_f64() >= 1.0 {
            let s = std::mem::take(&mut p.stats);
            let mut reasons: Vec<String> = s.reasons.iter().map(|(r, n)| format!("{r} {n}")).collect();
            reasons.extend(s.parts.iter().map(|(part, ms)| format!("{part} {:.3} ms", ms / f64::from(s.frames.max(1)))));
            let line = format!(
                "frames {:>4}  avg {:>6.3} ms  max {:>6.3} ms  layouts {:>3}  text layouts {:>5}  {}\n",
                s.frames,
                s.total_ms / f64::from(s.frames.max(1)),
                s.max_ms,
                s.layouts,
                s.texts,
                reasons.join(", ")
            );
            if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(&p.path) {
                _ = file.write_all(line.as_bytes());
            }
            p.since = Instant::now();
        }
    });
}

/// The page was laid out again.
pub fn layout() {
    with(|p| p.stats.layouts += 1);
}

/// A DirectWrite text layout was made.
pub fn text_layout() {
    with(|p| p.stats.texts += 1);
}

/// Counts a reason a frame was drawn.
pub fn reason(why: &'static str) {
    with(|p| match p.stats.reasons.iter_mut().find(|(r, _)| *r == why) {
        Some((_, n)) => *n += 1,
        None => p.stats.reasons.push((why, 1)),
    });
}

/// Time a part of the frame took (averaged per frame in the log).
pub fn part(name: &'static str, ms: f64) {
    with(|p| match p.stats.parts.iter_mut().find(|(r, _)| *r == name) {
        Some((_, total)) => *total += ms,
        None => p.stats.parts.push((name, ms)),
    });
}
