//! Rehearses the real flows end to end with fake data, covering the design system's
//! building blocks: now-playing (art or a real image, bars, progress, transport), a
//! countdown timer the island runs by itself (ring, hero numerals, inline controls, minimal
//! bubble), and recurring agent moments (`moments.rs`: a permission, a question, a plan)
//! that take over the island and must be answered or time out and fail open, as the real
//! bridge does.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use reui_protocol::{unix_ms, ACTION_COLLAPSED, ACTION_EXPANDED, Art, Button, ButtonContent, ButtonRole, CardSize, Clock, CompactView, Element, ExpandedView, Headline, ImageId, Priority, Rgb, Row};
use tokio::time::{sleep, sleep_until, Instant};

use self::moments::Moments;
use super::Module;
use crate::hub::{ModuleContext, ModuleInput, View};

mod moments;

const FIRST_REQUEST: Duration = Duration::from_secs(6);
const BETWEEN_REQUESTS: Duration = Duration::from_secs(25);
const TIMER_LENGTH: u32 = 5 * 60;

const GLYPH_PREV: char = '\u{E892}';
const GLYPH_NEXT: char = '\u{E893}';
const GLYPH_PLAY: char = '\u{E768}';
const GLYPH_PAUSE: char = '\u{E769}';
pub(super) const GLYPH_TERMINAL: char = '\u{E756}';
const GLYPH_STOP: char = '\u{E711}';

pub(super) const AGENT_ACCENT: Rgb = Rgb(217, 119, 87);
const TIMER_ACCENT: Rgb = Rgb(255, 159, 10);

/// Real artwork for the first track, to exercise image delivery; any JPEG or PNG works.
const DEMO_ART: &str = r"C:\Windows\Web\Wallpaper\Windows\img0.jpg";
const IMAGE_ACCENT: Rgb = Rgb(64, 156, 255);

struct Track {
    title: &'static str,
    artist: &'static str,
    from: Rgb,
    to: Rgb,
    length: u32,
}

const TRACKS: [Track; 3] = [
    Track { title: "Midnight City", artist: "M83", from: Rgb(245, 89, 140), to: Rgb(102, 51, 217), length: 243 },
    Track { title: "Nightcall", artist: "Kavinsky", from: Rgb(250, 158, 51), to: Rgb(217, 46, 77), length: 258 },
    Track { title: "Intro (a much longer title to test trimming)", artist: "The xx", from: Rgb(51, 191, 179), to: Rgb(38, 77, 204), length: 128 },
];

pub struct Demo {
    /// Only the playing track: no timer and no agent moments, so a measurement of what
    /// music costs at rest (`tools/perf-sample.ps1`) isn't interrupted.
    pub music_only: bool,
}

impl Module for Demo {
    fn name(&self) -> &'static str {
        "demo"
    }

    fn run(self: Box<Self>, ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(run(ctx, self.music_only))
    }
}

async fn run(mut ctx: ModuleContext, music_only: bool) {
    let image = std::fs::read(DEMO_ART).ok().map(|bytes| ctx.image(bytes));
    let mut track = 0;
    let mut playing = true;
    publish_music(&ctx, track, playing, image);

    // The island runs the clock; the hub only wakes when it ends or is paused.
    let mut timer = (!music_only).then(|| Clock { value_ms: u64::from(TIMER_LENGTH) * 1000, at_unix_ms: unix_ms(), running: true, down: true });
    if let Some(clock) = timer {
        publish_timer(&ctx, clock);
    }

    let mut moments = Moments::new();
    // Music only: the first moment is a year away.
    let mut next_request = Instant::now() + if music_only { Duration::from_secs(365 * 86_400) } else { FIRST_REQUEST };

    loop {
        let deadline = moments.deadline().unwrap_or(next_request);
        let timer_left = timer.filter(|c| c.running).map(|c| Duration::from_millis(c.value_at(unix_ms())));
        tokio::select! {
            input = ctx.next() => {
                let action = match input {
                    Some(ModuleInput::Action(action)) => action,
                    Some(_) => continue,
                    None => break,
                };
                match (action.key.as_str(), action.action.as_str()) {
                    ("music", "prev") => track = (track + TRACKS.len() - 1) % TRACKS.len(),
                    ("music", "next") => track = (track + 1) % TRACKS.len(),
                    ("music", "play") => playing = !playing,
                    ("timer", "pause") => {
                        timer = timer.map(|c| {
                            let now = unix_ms();
                            Clock { value_ms: c.value_at(now), at_unix_ms: now, running: !c.running, down: true }
                        });
                    }
                    ("timer", "stop") => {
                        timer = None;
                        ctx.retract("timer");
                    }
                    // The island saying the card opened or closed isn't an answer.
                    (_, ACTION_EXPANDED | ACTION_COLLAPSED) => continue,
                    (key, decision) if moments.owns(key) => {
                        if moments.act(&ctx, decision) {
                            next_request = Instant::now() + BETWEEN_REQUESTS;
                        }
                        continue;
                    }
                    (key, other) => println!("demo: ignoring {other} on {key}"),
                }
                publish_music(&ctx, track, playing, image);
                if let Some(clock) = timer {
                    publish_timer(&ctx, clock);
                }
            }
            _ = sleep(timer_left.unwrap_or_default()), if timer_left.is_some() => {
                timer = None;
                ctx.retract("timer");
            }
            _ = sleep_until(deadline) => {
                if moments.deadline().is_some() {
                    moments.time_out(&ctx);
                    next_request = Instant::now() + BETWEEN_REQUESTS;
                } else {
                    moments.ask(&ctx);
                }
            }
        }
    }
}

fn clock(seconds: u32) -> String {
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn glyph_button(action: &str, glyph: char, role: ButtonRole) -> Button {
    Button { action: action.into(), content: ButtonContent::Glyph(glyph), role }
}

fn publish_music(ctx: &ModuleContext, track: usize, playing: bool, image: Option<ImageId>) {
    let t = &TRACKS[track];
    let (art, accent) = match image {
        Some(id) if track == 0 => (Art::Image(id), IMAGE_ACCENT),
        _ => (Art::Gradient { from: t.from, to: t.to }, t.from),
    };
    let position = t.length * 38 / 100;
    ctx.publish(
        "music",
        Priority::Ongoing,
        View {
            accent: Some(accent),
            audio: None,
            compact: CompactView { leading: Element::Art(art.clone()), trailing: Element::Bars { live: playing } },
            minimal: Element::Bars { live: playing },
            expanded: Some(ExpandedView {
                leading: Element::Art(art),
                eyebrow: Some("Now Playing".into()),
                headline: Headline::Text(t.title.into()),
                detail: Some(t.artist.into()),
                trailing: Element::Bars { live: playing },
                context: None, size: CardSize::Regular,
                tint: false,
                pager: None,
                open: None,
                rows: vec![
                    Row::Progress {
                        fraction: position as f32 / t.length as f32,
                        leading: Some(clock(position)),
                        trailing: Some(format!("-{}", clock(t.length - position))),
                    },
                    Row::Buttons(vec![
                        glyph_button("prev", GLYPH_PREV, ButtonRole::Plain),
                        glyph_button("play", if playing { GLYPH_PAUSE } else { GLYPH_PLAY }, ButtonRole::Primary),
                        glyph_button("next", GLYPH_NEXT, ButtonRole::Plain),
                    ]),
                ],
            }),
        },
    );
}

fn publish_timer(ctx: &ModuleContext, clock: Clock) {
    let ring = Element::ClockRing { clock, total_ms: u64::from(TIMER_LENGTH) * 1000 };
    let running = clock.running;
    ctx.publish(
        "timer",
        Priority::Ongoing,
        View {
            accent: Some(TIMER_ACCENT),
            audio: None,
            compact: CompactView { leading: ring.clone(), trailing: Element::Clock(clock) },
            minimal: ring.clone(),
            expanded: Some(ExpandedView {
                leading: ring,
                eyebrow: Some("Timer".into()),
                headline: Headline::Clock { clock, tenths: false },
                detail: (!running).then(|| "Paused".into()),
                trailing: Element::Empty,
                // A lone glyph row sits inline beside the value.
                context: None, size: CardSize::Regular,
                tint: false,
                pager: None,
                open: None,
                rows: vec![Row::Buttons(vec![
                    glyph_button("pause", if running { GLYPH_PAUSE } else { GLYPH_PLAY }, ButtonRole::Accent),
                    glyph_button("stop", GLYPH_STOP, ButtonRole::Secondary),
                ])],
            }),
        },
    );
}
