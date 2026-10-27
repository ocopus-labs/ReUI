//! Rehearses the real flows end to end with fake data, covering the design system's
//! building blocks: now-playing (art or a real image, bars, progress, transport), a
//! countdown timer (ring, hero numerals, inline controls, minimal bubble), and a recurring
//! agent permission request that takes over the island and must be answered or times out
//! and fails open, as the real bridge will.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use reui_protocol::{Art, Button, ButtonContent, ButtonRole, CompactView, Element, ExpandedView, Headline, ImageId, Priority, Rgb, Row, Tone};
use tokio::time::{interval_at, sleep_until, Instant, MissedTickBehavior};

use super::Module;
use crate::hub::{ModuleContext, ModuleInput, View};

const FIRST_REQUEST: Duration = Duration::from_secs(6);
const BETWEEN_REQUESTS: Duration = Duration::from_secs(25);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const TIMER_LENGTH: u32 = 5 * 60;

const GLYPH_PREV: char = '\u{E892}';
const GLYPH_NEXT: char = '\u{E893}';
const GLYPH_PLAY: char = '\u{E768}';
const GLYPH_PAUSE: char = '\u{E769}';
const GLYPH_TERMINAL: char = '\u{E756}';
const GLYPH_STOP: char = '\u{E711}';

const AGENT_ACCENT: Rgb = Rgb(217, 119, 87);
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

const COMMANDS: [&str; 3] = ["cargo build --release", "git push origin main", "rm -rf target/"];

pub struct Demo;

impl Module for Demo {
    fn name(&self) -> &'static str {
        "demo"
    }

    fn run(self: Box<Self>, ctx: ModuleContext) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(run(ctx))
    }
}

async fn run(mut ctx: ModuleContext) {
    let image = std::fs::read(DEMO_ART).ok().map(|bytes| ctx.image(bytes));
    let mut track = 0;
    let mut playing = true;
    publish_music(&ctx, track, playing, image);

    let mut timer_left = Some(TIMER_LENGTH);
    let mut timer_running = true;
    publish_timer(&ctx, TIMER_LENGTH, timer_running);
    // One wake-up a second, only while the timer runs (a countdown is information, not decoration).
    let mut tick = interval_at(Instant::now() + Duration::from_secs(1), Duration::from_secs(1));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let mut requests = 0usize;
    let mut pending: Option<(String, Instant)> = None;
    let mut next_request = Instant::now() + FIRST_REQUEST;

    loop {
        let deadline = pending.as_ref().map(|(_, at)| *at).unwrap_or(next_request);
        let ticking = timer_running && timer_left.is_some();
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
                    ("timer", "pause") => timer_running = !timer_running,
                    ("timer", "stop") => {
                        timer_left = None;
                        ctx.retract("timer");
                    }
                    (key, decision) if pending.as_ref().is_some_and(|(k, _)| k == key) => {
                        println!("demo: {key} answered from the island: {decision}");
                        ctx.retract(key);
                        pending = None;
                        next_request = Instant::now() + BETWEEN_REQUESTS;
                    }
                    (key, other) => println!("demo: ignoring {other} on {key}"),
                }
                publish_music(&ctx, track, playing, image);
                if let Some(left) = timer_left {
                    publish_timer(&ctx, left, timer_running);
                }
            }
            _ = tick.tick(), if ticking => {
                let left = timer_left.unwrap_or(0).saturating_sub(1);
                if left == 0 {
                    timer_left = None;
                    ctx.retract("timer");
                } else {
                    timer_left = Some(left);
                    publish_timer(&ctx, left, timer_running);
                }
            }
            _ = sleep_until(deadline) => {
                if let Some((key, _)) = pending.take() {
                    println!("demo: {key} timed out; failing open to the agent's own prompt");
                    ctx.retract(&key);
                    next_request = Instant::now() + BETWEEN_REQUESTS;
                } else {
                    let command = COMMANDS[requests % COMMANDS.len()];
                    requests += 1;
                    let key = format!("permission-{requests}");
                    println!("demo: {key} asking to run `{command}`");
                    publish_request(&ctx, &key, command);
                    pending = Some((key, Instant::now() + REQUEST_TIMEOUT));
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

fn text_button(action: &str, label: &str, role: ButtonRole) -> Button {
    Button { action: action.into(), content: ButtonContent::Text(label.into()), role }
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
                wide: false,
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

fn publish_timer(ctx: &ModuleContext, left: u32, running: bool) {
    let ring = Element::Ring(left as f32 / TIMER_LENGTH as f32);
    ctx.publish(
        "timer",
        Priority::Ongoing,
        View {
            accent: Some(TIMER_ACCENT),
            audio: None,
            compact: CompactView { leading: ring.clone(), trailing: Element::Text { text: clock(left), tone: Tone::Accent } },
            minimal: ring.clone(),
            expanded: Some(ExpandedView {
                leading: ring,
                eyebrow: Some("Timer".into()),
                headline: Headline::Value(clock(left)),
                detail: (!running).then(|| "Paused".into()),
                trailing: Element::Empty,
                // A lone glyph row sits inline beside the value.
                wide: false,
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

fn publish_request(ctx: &ModuleContext, key: &str, command: &str) {
    let tile = Element::Art(Art::Glyph(GLYPH_TERMINAL));
    ctx.publish(
        key,
        Priority::Decision,
        View {
            accent: Some(AGENT_ACCENT),
            audio: None,
            compact: CompactView { leading: tile.clone(), trailing: Element::Text { text: "Allow?".into(), tone: Tone::Accent } },
            minimal: tile.clone(),
            expanded: Some(ExpandedView {
                leading: tile,
                eyebrow: Some("Claude Code".into()),
                headline: Headline::Code(command.into()),
                detail: Some("wants to run this in ReUI".into()),
                trailing: Element::Empty,
                wide: false,
                tint: false,
                pager: None,
                open: None,
                rows: vec![Row::Buttons(vec![
                    text_button("deny", "Deny", ButtonRole::Destructive),
                    text_button("allow", "Allow once", ButtonRole::Primary),
                ])],
            }),
        },
    );
}
