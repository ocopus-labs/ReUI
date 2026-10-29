//! The Now Playing card: what it shows (track, art, timeline, transport, volume, lyrics,
//! switcher) and the view built from it for the island.

use reui_protocol::{unix_ms, Button, ButtonContent, ButtonRole, CardSize, CompactView, Element, ExpandedView, Headline, LyricLine, Menu, MenuOption, Pager, Priority, Rgb, Row};
use windows::Media::MediaPlaybackAutoRepeatMode as RepeatMode;

use super::settings::{Settings, Visualizer};
use super::{audio, sessions, AudioInfo};
use crate::hub::View;

const GLYPH_PREV: char = '\u{E892}';
const GLYPH_NEXT: char = '\u{E893}';
const GLYPH_PLAY: char = '\u{E768}';
const GLYPH_PAUSE: char = '\u{E769}';
pub(super) const GLYPH_MUSIC: char = '\u{E8D6}';
const GLYPH_SHUFFLE: char = '\u{E8B1}';
const GLYPH_REPEAT_ALL: char = '\u{E8EE}';
const GLYPH_REPEAT_ONE: char = '\u{E8ED}';
const GLYPH_VOLUME: char = '\u{E767}';
const GLYPH_MUTED: char = '\u{E74F}';
const GLYPH_OUTPUT: char = '\u{E7F3}';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Repeat {
    Off,
    All,
    One,
}

impl Repeat {
    pub(super) fn from_mode(mode: RepeatMode) -> Self {
        match mode {
            RepeatMode::List => Self::All,
            RepeatMode::Track => Self::One,
            _ => Self::Off,
        }
    }

    pub(super) fn mode(self) -> RepeatMode {
        match self {
            Self::Off => RepeatMode::None,
            Self::All => RepeatMode::List,
            Self::One => RepeatMode::Track,
        }
    }

    /// None → List → Track → None, as players cycle it.
    pub(super) fn next(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }
}

/// Transport controls the player advertises; shuffle and repeat carry their state.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Controls {
    pub(super) prev: bool,
    pub(super) play_pause: bool,
    pub(super) next: bool,
    pub(super) shuffle: Option<bool>,
    pub(super) repeat: Option<Repeat>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Timeline {
    pub(super) position_ms: u64,
    /// 0 for a live stream.
    pub(super) length_ms: u64,
    pub(super) at_unix_ms: u64,
    pub(super) seekable: bool,
    pub(super) rate: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Volume {
    value: f32,
    muted: bool,
    outputs: Vec<audio::Output>,
    can_switch: bool,
}

/// Everything the card shows, so it can be rebuilt without asking the player again (an
/// optimistic play/pause, a volume drag, lyrics arriving).
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Card {
    pub(super) aumid: String,
    pub(super) track: String,
    pub(super) app: String,
    pub(super) title: String,
    pub(super) artist: String,
    pub(super) album: String,
    pub(super) ad: bool,
    pub(super) video: bool,
    pub(super) art: Element,
    pub(super) accent: Rgb,
    pub(super) playing: bool,
    pub(super) timeline: Option<Timeline>,
    pub(super) controls: Controls,
    pub(super) volume: Option<Volume>,
    pub(super) lyrics: Vec<LyricLine>,
    pub(super) pager: Option<Pager>,
    pub(super) visualizer: Visualizer,
    pub(super) audio_pid: Option<u32>,
}

impl Card {
    /// Playing or paused as of `now`, the timeline continuing from where it is.
    pub(super) fn set_playing(&mut self, playing: bool, now: u64) {
        if let Some(t) = &mut self.timeline {
            t.position_ms = Row::timeline_position(t.position_ms, t.length_ms, self.playing, t.at_unix_ms, t.rate, now);
            t.at_unix_ms = now;
        }
        self.playing = playing;
    }
}

/// Fills in what doesn't come from the player itself.
pub(super) fn decorate(card: &mut Card, settings: &Settings, audio: Option<&AudioInfo>, lyrics: Option<&(String, Option<Vec<LyricLine>>)>, pager: Option<Pager>) {
    let audio = audio.filter(|a| a.aumid == card.aumid);
    card.app = sessions::app_name(&card.aumid, audio.and_then(|a| a.exe.as_deref()));
    card.visualizer = settings.visualizer;
    card.audio_pid = audio.and_then(|a| a.pid);
    card.volume = audio.and_then(|a| {
        let (value, muted) = a.level()?;
        Some(Volume { value, muted, outputs: a.outputs.clone(), can_switch: a.can_switch })
    });
    let wanted = settings.lyrics && !card.ad;
    card.lyrics = lyrics.filter(|(track, _)| wanted && *track == card.track).and_then(|(_, lines)| lines.clone()).unwrap_or_default();
    card.pager = pager;
}

fn glyph_button(action: &str, glyph: char, role: ButtonRole) -> Button {
    Button { action: action.into(), content: ButtonContent::Glyph(glyph), role }
}

/// Shuffle · prev · play/pause · next · repeat, each only when the player offers it.
fn controls_row(c: &Controls, playing: bool) -> Vec<Button> {
    let lit = |on: bool| if on { ButtonRole::Accent } else { ButtonRole::Plain };
    let mut buttons = Vec::new();
    if let Some(on) = c.shuffle {
        buttons.push(glyph_button("shuffle", GLYPH_SHUFFLE, lit(on)));
    }
    if c.prev {
        buttons.push(glyph_button("prev", GLYPH_PREV, ButtonRole::Plain));
    }
    if c.play_pause {
        buttons.push(glyph_button("play", if playing { GLYPH_PAUSE } else { GLYPH_PLAY }, ButtonRole::Primary));
    }
    if c.next {
        buttons.push(glyph_button("next", GLYPH_NEXT, ButtonRole::Plain));
    }
    if let Some(repeat) = c.repeat {
        buttons.push(glyph_button("repeat", if repeat == Repeat::One { GLYPH_REPEAT_ONE } else { GLYPH_REPEAT_ALL }, lit(repeat != Repeat::Off)));
    }
    buttons
}

/// Volume with mute, and the output menu (Sound settings where outputs can't be switched).
fn volume_row(v: &Volume) -> Row {
    let options = if v.can_switch && !v.outputs.is_empty() {
        v.outputs.iter().map(|o| MenuOption { label: o.name.clone(), action: format!("output:{}", o.id), selected: o.default }).collect()
    } else {
        vec![MenuOption { label: "Sound settings".into(), action: "output:settings".into(), selected: false }]
    };
    Row::Slider {
        leading: glyph_button("mute", if v.muted { GLYPH_MUTED } else { GLYPH_VOLUME }, ButtonRole::Plain),
        value: v.value.clamp(0.0, 1.0),
        action: "volume:".into(),
        trailing: Some(Menu { content: ButtonContent::Glyph(GLYPH_OUTPUT), options }),
    }
}

/// The card as the island gets it: "App · Album" over the title over the artist.
pub(super) fn build_view(card: &Card) -> (Priority, View) {
    let bars = Element::Bars { live: card.playing && card.visualizer != Visualizer::Off };
    let mut rows = Vec::new();
    if let Some(t) = &card.timeline {
        rows.push(Row::Timeline { position_ms: t.position_ms, length_ms: t.length_ms, playing: card.playing, at_unix_ms: t.at_unix_ms, seekable: t.seekable && t.length_ms > 0, rate: t.rate });
    }
    let buttons = controls_row(&card.controls, card.playing);
    if !buttons.is_empty() {
        rows.push(Row::Buttons(buttons));
    }
    if let Some(volume) = &card.volume {
        rows.push(volume_row(volume));
    }
    if !card.lyrics.is_empty() && card.timeline.is_some_and(|t| t.length_ms > 0) {
        rows.push(Row::Lyrics(card.lyrics.clone()));
    }
    let album = card.album.trim();
    let eyebrow = if album.is_empty() || album.eq_ignore_ascii_case(card.title.trim()) || card.ad { card.app.clone() } else { format!("{} · {album}", card.app) };
    let detail = if card.ad {
        Some("Ad".to_string())
    } else {
        Some(card.artist.trim().to_string()).filter(|a| !a.is_empty())
    };
    let view = View {
        accent: Some(card.accent),
        audio: if card.visualizer == Visualizer::Real { card.audio_pid } else { None },
        compact: CompactView { leading: card.art.clone(), trailing: bars.clone() },
        // Live bars in the bubble, as in the pill (the user asked for motion there too).
        minimal: bars.clone(),
        expanded: Some(ExpandedView {
            leading: card.art.clone(),
            eyebrow: Some(eyebrow),
            headline: Headline::Text(card.title.clone()),
            detail,
            trailing: bars,
            rows,
            context: None, size: CardSize::Regular,
            // Black like the rest of the island: colour comes from the art, bars and progress.
            tint: false,
            pager: card.pager.clone(),
            open: Some("open".into()),
        }),
    };
    (if card.playing { Priority::Ongoing } else { Priority::Passive }, view)
}

/// The view with motion stopped: bars at rest and the timeline held where it is now.
pub(super) fn frozen(view: &View) -> View {
    let mut view = view.clone();
    let rest = |e: &mut Element| {
        if let Element::Bars { live } = e {
            *live = false;
        }
    };
    rest(&mut view.compact.trailing);
    rest(&mut view.minimal);
    if let Some(expanded) = &mut view.expanded {
        rest(&mut expanded.trailing);
        for row in &mut expanded.rows {
            if let Row::Timeline { position_ms, length_ms, playing, at_unix_ms, rate, .. } = row {
                let now = unix_ms();
                *position_ms = Row::timeline_position(*position_ms, *length_ms, *playing, *at_unix_ms, *rate, now);
                *playing = false;
                *at_unix_ms = now;
            }
        }
    }
    view
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::DEFAULT_ACCENT;
    use reui_protocol::Art;

    fn card() -> Card {
        Card {
            aumid: "Spotify.exe".into(),
            track: "t".into(),
            app: "Spotify".into(),
            title: "Song".into(),
            artist: "Band".into(),
            album: "Record".into(),
            ad: false,
            video: false,
            art: Element::Art(Art::Glyph(GLYPH_MUSIC)),
            accent: DEFAULT_ACCENT,
            playing: true,
            timeline: Some(Timeline { position_ms: 10_000, length_ms: 200_000, at_unix_ms: 1_000, seekable: true, rate: 1.0 }),
            controls: Controls { prev: true, play_pause: true, next: true, shuffle: Some(true), repeat: Some(Repeat::One) },
            volume: None,
            lyrics: Vec::new(),
            pager: None,
            visualizer: Visualizer::Real,
            audio_pid: Some(42),
        }
    }

    fn expanded(card: &Card) -> ExpandedView {
        build_view(card).1.expanded.unwrap()
    }

    #[test]
    fn the_card_reads_app_album_title_artist() {
        let e = expanded(&card());
        assert_eq!((e.eyebrow.as_deref(), &e.headline, e.detail.as_deref()), (Some("Spotify · Record"), &Headline::Text("Song".into()), Some("Band")));
        assert!(!e.tint, "the music card stays black");
        assert_eq!(e.open.as_deref(), Some("open"));
        // An album named like the track (singles) isn't repeated.
        let single = Card { album: "song".into(), ..card() };
        assert_eq!(expanded(&single).eyebrow.as_deref(), Some("Spotify"));
        let ad = Card { title: "Advertisement".into(), artist: String::new(), ad: true, ..card() };
        assert_eq!((expanded(&ad).eyebrow.as_deref(), expanded(&ad).detail.as_deref()), (Some("Spotify"), Some("Ad")));
    }

    #[test]
    fn controls_show_only_what_the_player_offers_with_their_state() {
        let row = controls_row(&card().controls, true);
        let summary: Vec<_> = row.iter().map(|b| (b.action.as_str(), b.content.clone(), b.role)).collect();
        assert_eq!(
            summary,
            [
                ("shuffle", ButtonContent::Glyph(GLYPH_SHUFFLE), ButtonRole::Accent),
                ("prev", ButtonContent::Glyph(GLYPH_PREV), ButtonRole::Plain),
                ("play", ButtonContent::Glyph(GLYPH_PAUSE), ButtonRole::Primary),
                ("next", ButtonContent::Glyph(GLYPH_NEXT), ButtonRole::Plain),
                ("repeat", ButtonContent::Glyph(GLYPH_REPEAT_ONE), ButtonRole::Accent),
            ]
        );
        let plain = Controls { play_pause: true, shuffle: Some(false), repeat: Some(Repeat::Off), ..Controls::default() };
        let row = controls_row(&plain, false);
        assert_eq!(row.iter().map(|b| (b.action.as_str(), b.role)).collect::<Vec<_>>(), [("shuffle", ButtonRole::Plain), ("play", ButtonRole::Primary), ("repeat", ButtonRole::Plain)]);
        assert_eq!(row[1].content, ButtonContent::Glyph(GLYPH_PLAY));
        assert_eq!(row[2].content, ButtonContent::Glyph(GLYPH_REPEAT_ALL));
        assert!(controls_row(&Controls::default(), true).is_empty());
        assert_eq!([Repeat::Off.next(), Repeat::All.next(), Repeat::One.next()], [Repeat::All, Repeat::One, Repeat::Off]);
    }

    #[test]
    fn optimistic_play_pause_holds_the_position() {
        let mut c = card();
        c.set_playing(false, 4_000);
        let t = c.timeline.unwrap();
        assert_eq!((c.playing, t.position_ms, t.at_unix_ms), (false, 13_000, 4_000));
        assert_eq!(controls_row(&c.controls, c.playing)[2].content, ButtonContent::Glyph(GLYPH_PLAY));
        assert_eq!(build_view(&c).0, Priority::Passive);
        c.set_playing(true, 9_000);
        assert_eq!(c.timeline.unwrap().position_ms, 13_000);
    }

    #[test]
    fn volume_row_offers_outputs_or_sound_settings() {
        let output = |id: &str, default| audio::Output { id: id.into(), name: format!("Speaker {id}"), default };
        let v = Volume { value: 0.4, muted: false, outputs: vec![output("a", true), output("b", false)], can_switch: true };
        let Row::Slider { leading, value, action, trailing } = volume_row(&v) else { panic!() };
        assert_eq!((leading.action.as_str(), leading.content, value, action.as_str()), ("mute", ButtonContent::Glyph(GLYPH_VOLUME), 0.4, "volume:"));
        let menu = trailing.unwrap();
        assert_eq!(menu.options.iter().map(|o| (o.action.as_str(), o.selected)).collect::<Vec<_>>(), [("output:a", true), ("output:b", false)]);

        let v = Volume { muted: true, can_switch: false, ..v };
        let Row::Slider { leading, trailing, .. } = volume_row(&v) else { panic!() };
        assert_eq!(leading.content, ButtonContent::Glyph(GLYPH_MUTED));
        assert_eq!(trailing.unwrap().options, [MenuOption { label: "Sound settings".into(), action: "output:settings".into(), selected: false }]);
    }

    #[test]
    fn rows_come_in_order_and_lyrics_need_a_timeline() {
        let lyric = vec![LyricLine { at_ms: 0, text: "la".into() }];
        let volume = Volume { value: 1.0, muted: false, outputs: vec![], can_switch: false };
        let c = Card { lyrics: lyric.clone(), volume: Some(volume), ..card() };
        let rows = expanded(&c).rows;
        assert!(matches!(&rows[..], [Row::Timeline { .. }, Row::Buttons(_), Row::Slider { .. }, Row::Lyrics(_)]), "{rows:?}");
        // A live stream: a length-0 timeline, never seekable, and no lyrics.
        let live = Card { timeline: Some(Timeline { position_ms: 0, length_ms: 0, at_unix_ms: 0, seekable: true, rate: 1.0 }), lyrics: lyric, ..card() };
        let rows = expanded(&live).rows;
        assert!(matches!(rows[0], Row::Timeline { length_ms: 0, seekable: false, .. }));
        assert!(!rows.iter().any(|r| matches!(r, Row::Lyrics(_))));
    }

    #[test]
    fn the_visualizer_setting_drives_bars_and_capture() {
        let (_, view) = build_view(&card());
        assert_eq!((view.compact.trailing, view.audio), (Element::Bars { live: true }, Some(42)));
        let (_, view) = build_view(&Card { visualizer: Visualizer::Simple, ..card() });
        assert_eq!((view.compact.trailing, view.audio), (Element::Bars { live: true }, None));
        let (_, view) = build_view(&Card { visualizer: Visualizer::Off, ..card() });
        assert_eq!((view.compact.trailing, view.audio), (Element::Bars { live: false }, None));
        assert_eq!(view.minimal, Element::Bars { live: false }, "the bubble shows the same bars as the pill");
    }

    #[test]
    fn decoration_follows_settings_and_the_shown_player() {
        let mut c = card();
        let lines = vec![LyricLine { at_ms: 0, text: "la".into() }];
        let lyrics = ("t".to_string(), Some(lines.clone()));
        let info = AudioInfo { aumid: "Spotify.exe".into(), pid: Some(7), exe: None, app: None, system: Some((0.3, false)), outputs: vec![], can_switch: false };
        let on = Settings { lyrics: true, ..Settings::default() };
        decorate(&mut c, &on, Some(&info), Some(&lyrics), None);
        assert_eq!((c.lyrics.clone(), c.audio_pid, c.volume.as_ref().map(|v| v.value)), (lines.clone(), Some(7), Some(0.3)));
        // Lyrics off, or for another track, or an ad: none.
        decorate(&mut c, &Settings::default(), Some(&info), Some(&lyrics), None);
        assert!(c.lyrics.is_empty());
        let mut ad = Card { ad: true, ..card() };
        decorate(&mut ad, &on, None, Some(&lyrics), None);
        assert!(ad.lyrics.is_empty() && ad.volume.is_none());
        // Audio read for another player doesn't apply.
        let other = AudioInfo { aumid: "Chrome".into(), ..info };
        decorate(&mut c, &on, Some(&other), None, None);
        assert_eq!((c.audio_pid, c.volume.is_none()), (None, true));
    }

    #[test]
    fn frozen_views_stop_moving() {
        let (_, view) = build_view(&card());
        let still = frozen(&view);
        assert_eq!(still.compact.trailing, Element::Bars { live: false });
        assert!(matches!(still.expanded.unwrap().rows[0], Row::Timeline { playing: false, .. }));
    }
}
