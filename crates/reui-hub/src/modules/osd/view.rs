//! The system OSD's pure parts: glyph tiers, the HUD views, and the timing rules (linger,
//! coalescing, echo filtering, the device-switch window, the automatic-brightness filter).
//! Everything here is a function of its inputs, so it's all unit-tested.
//!
//! Glyphs (Segoe Fluent Icons):
//! - volume: E992 (0), E993 (1 wave), E994 (2 waves), E995 (3 waves), E74F muted;
//! - output devices: E7F6 headphones / headset, E7F3 display with speaker (HDMI / DP audio),
//!   E7F5 speaker box (line out, S/PDIF, unknown); built-in speakers use the volume tiers;
//! - brightness: EC8A small sun (below half), E706 sun;
//! - states: E84B Caps Lock (⇧ABC), E765 keyboard (Num Lock), E8BE leaf (battery saver),
//!   E720 microphone, F781 microphone off.

use std::time::Duration;

use reui_protocol::{Art, Button, ButtonContent, ButtonRole, CardSize, Channel, CompactView, Element, ExpandedView, Headline, Menu, MenuOption, Row, Tone};
use tokio::time::Instant;

use crate::modules::audio::{DeviceKind, Output};
use crate::hub::View;

pub const GLYPH_VOLUME_0: char = '\u{E992}';
pub const GLYPH_VOLUME_1: char = '\u{E993}';
pub const GLYPH_VOLUME_2: char = '\u{E994}';
pub const GLYPH_VOLUME_3: char = '\u{E995}';
pub const GLYPH_MUTED: char = '\u{E74F}';
pub const GLYPH_HEADPHONES: char = '\u{E7F6}';
pub const GLYPH_DISPLAY_AUDIO: char = '\u{E7F3}';
pub const GLYPH_SPEAKER_BOX: char = '\u{E7F5}';
const GLYPH_OUTPUT: char = '\u{E7F3}';
/// The mixer's button: Segoe Fluent's equalizer.
pub const GLYPH_MIXER: char = '\u{E9E9}';
/// The volume card's mixer button flips the mixer.
pub const ACTION_MIXER: &str = "mixer";
pub const GLYPH_BRIGHTNESS_LOW: char = '\u{EC8A}';
pub const GLYPH_BRIGHTNESS_HIGH: char = '\u{E706}';
pub const GLYPH_CAPS_LOCK: char = '\u{E84B}';
pub const GLYPH_NUM_LOCK: char = '\u{E765}';
pub const GLYPH_BATTERY_SAVER: char = '\u{E8BE}';
pub const GLYPH_MIC: char = '\u{E720}';
pub const GLYPH_MIC_OFF: char = '\u{F781}';

/// A HUD stays this long after the last change...
pub const LINGER: Duration = Duration::from_millis(1500);
/// ...and this long after the user touched it (drag, mute, output).
pub const LINGER_INTERACTION: Duration = Duration::from_millis(2500);
/// Lock keys and other one-off states.
pub const LINGER_STATE: Duration = Duration::from_millis(1200);
/// At most one publish per this (≈30 Hz) while a level keeps changing.
pub const COALESCE: Duration = Duration::from_millis(33);
/// No volume HUD this long after the default output changed (its level jumps).
pub const DEVICE_SWITCH_QUIET: Duration = Duration::from_millis(500);
/// Our own volume echoes are trusted again this long after our last write.
pub const OURS_SETTLE: Duration = Duration::from_millis(300);
/// Brightness events this soon after our own write are its echo.
pub const BRIGHTNESS_ECHO: Duration = Duration::from_millis(400);
/// A brightness change counts as asked for when a key or action came this soon before it.
pub const BRIGHTNESS_INTENT: Duration = Duration::from_secs(1);

// ---- glyphs ------------------------------------------------------------------------------

/// Speaker by level: none, one, two or three waves; muted overrides.
pub fn volume_glyph(level: f32, muted: bool) -> char {
    if muted {
        return GLYPH_MUTED;
    }
    match level {
        l if l.is_nan() || l <= 0.005 => GLYPH_VOLUME_0,
        l if l < 0.34 => GLYPH_VOLUME_1,
        l if l < 0.67 => GLYPH_VOLUME_2,
        _ => GLYPH_VOLUME_3,
    }
}

/// The device's own glyph, `None` for speakers (the level tiers say more).
pub fn device_glyph(kind: DeviceKind) -> Option<char> {
    match kind {
        DeviceKind::Speakers => None,
        DeviceKind::Headphones | DeviceKind::Headset => Some(GLYPH_HEADPHONES),
        DeviceKind::Display => Some(GLYPH_DISPLAY_AUDIO),
        DeviceKind::Other => Some(GLYPH_SPEAKER_BOX),
    }
}

/// The compact HUD's glyph: muted, else the device when it isn't speakers, else the tier.
pub fn hud_glyph(level: f32, muted: bool, kind: DeviceKind) -> char {
    if muted {
        return GLYPH_MUTED;
    }
    device_glyph(kind).unwrap_or_else(|| volume_glyph(level, false))
}

pub fn brightness_glyph(level: f32) -> char {
    if level < 0.5 {
        GLYPH_BRIGHTNESS_LOW
    } else {
        GLYPH_BRIGHTNESS_HIGH
    }
}

fn percent(level: f32) -> String {
    format!("{}%", (level.clamp(0.0, 1.0) * 100.0).round() as u32)
}

// ---- views -------------------------------------------------------------------------------

/// What the volume HUD shows.
#[derive(Debug, Clone, PartialEq)]
pub struct VolumeHud {
    pub level: f32,
    pub muted: bool,
    pub device: String,
    pub kind: DeviceKind,
    pub outputs: Vec<Output>,
    pub can_switch: bool,
    pub show_percentage: bool,
    /// Each app's level under the slider, when the mixer is showing (`Some`, maybe still
    /// empty while the sessions are read).
    pub mixer: Option<Vec<Channel>>,
}

/// Compact: glyph · level bar. Minimal: a level ring. Open: "Volume" over the device, the
/// slider with mute and the output menu, the mixer's button at the header's end and, while
/// it's on, each app's level under the slider.
pub fn volume_view(v: &VolumeHud) -> View {
    let level = v.level.clamp(0.0, 1.0);
    let glyph = Element::Art(Art::Glyph(hud_glyph(level, v.muted, v.kind)));
    let options = if v.can_switch && !v.outputs.is_empty() {
        v.outputs.iter().map(|o| MenuOption { label: o.name.clone(), action: format!("output:{}", o.id), selected: o.default }).collect()
    } else {
        vec![MenuOption { label: "Sound settings".into(), action: "output:settings".into(), selected: false }]
    };
    let slider = Row::Slider {
        leading: Button { action: "mute".into(), content: ButtonContent::Glyph(volume_glyph(level, v.muted)), role: ButtonRole::Plain },
        value: level,
        action: "volume:".into(),
        trailing: Some(Menu { content: ButtonContent::Glyph(GLYPH_OUTPUT), options }),
    };
    let detail = match (v.device.is_empty(), v.show_percentage) {
        (false, true) => Some(format!("{} · {}", v.device, percent(level))),
        (false, false) => Some(v.device.clone()),
        (true, true) => Some(percent(level)),
        (true, false) => None,
    };
    let mixer_on = v.mixer.is_some();
    let mixer_button = Button { action: ACTION_MIXER.into(), content: ButtonContent::Glyph(GLYPH_MIXER), role: if mixer_on { ButtonRole::Accent } else { ButtonRole::Secondary } };
    let mut rows = vec![slider];
    if let Some(channels) = v.mixer.clone().filter(|c| !c.is_empty()) {
        rows.push(Row::Mixer(channels));
    }
    View {
        accent: None,
        compact: CompactView { leading: glyph.clone(), trailing: Element::Level { value: level, dim: v.muted } },
        minimal: Element::Ring(if v.muted { 0.0 } else { level }),
        expanded: Some(ExpandedView {
            leading: glyph,
            eyebrow: None,
            headline: Headline::Text("Volume".into()),
            detail,
            trailing: Element::Button(mixer_button),
            rows,
            context: None, size: CardSize::Regular,
            tint: false,
            pager: None,
            open: None,
        }),
        audio: None,
    }
}

/// The moment after the default output changed: its glyph and name, nothing else.
pub fn device_view(name: &str, kind: DeviceKind) -> View {
    let glyph = Element::Art(Art::Glyph(device_glyph(kind).unwrap_or(GLYPH_VOLUME_3)));
    View {
        accent: None,
        compact: CompactView { leading: glyph.clone(), trailing: Element::Text { text: name.to_owned(), tone: Tone::Secondary } },
        minimal: glyph,
        expanded: None,
        audio: None,
    }
}

/// One display the brightness HUD can show.
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayLevel {
    pub id: String,
    pub name: String,
    pub level: f32,
}

/// The brightness HUD: `front` is the display that changed (its level is the compact one).
/// Open: one slider per display, labelled by name when there are several.
pub fn brightness_view(displays: &[DisplayLevel], front: usize, show_percentage: bool) -> Option<View> {
    let shown = displays.get(front).or(displays.first())?;
    let level = shown.level.clamp(0.0, 1.0);
    let glyph = Element::Art(Art::Glyph(brightness_glyph(level)));
    let several = displays.len() > 1;
    let mut rows = Vec::new();
    for d in displays {
        if several {
            let label = if show_percentage { format!("{} · {}", d.name, percent(d.level)) } else { d.name.clone() };
            rows.push(Row::Note(label));
        }
        rows.push(Row::Slider {
            leading: Button { action: "display-settings".into(), content: ButtonContent::Glyph(brightness_glyph(d.level)), role: ButtonRole::Plain },
            value: d.level.clamp(0.0, 1.0),
            action: format!("brightness:{}:", d.id),
            trailing: None,
        });
    }
    let detail = match (several, show_percentage) {
        (true, _) => None,
        (false, true) => Some(format!("{} · {}", shown.name, percent(level))),
        (false, false) => Some(shown.name.clone()),
    };
    Some(View {
        accent: None,
        compact: CompactView { leading: glyph.clone(), trailing: Element::Level { value: level, dim: false } },
        minimal: Element::Ring(level),
        expanded: Some(ExpandedView {
            leading: glyph,
            eyebrow: None,
            headline: Headline::Text("Brightness".into()),
            detail,
            trailing: Element::Empty,
            rows,
            context: None, size: CardSize::Regular,
            tint: false,
            pager: None,
            open: None,
        }),
        audio: None,
    })
}

/// A one-off state: glyph and a quiet line ("Caps Lock on").
pub fn state_view(glyph: char, text: &str) -> View {
    let glyph = Element::Art(Art::Glyph(glyph));
    View {
        accent: None,
        compact: CompactView { leading: glyph.clone(), trailing: Element::Text { text: text.to_owned(), tone: Tone::Secondary } },
        minimal: glyph,
        expanded: None,
        audio: None,
    }
}

/// `brightness:<id>:<0..1>` → (id, level).
pub fn parse_brightness_action(action: &str) -> Option<(&str, f32)> {
    let (id, value) = action.strip_prefix("brightness:")?.rsplit_once(':')?;
    let value = value.parse::<f32>().ok().filter(|v| v.is_finite())?;
    (!id.is_empty()).then_some((id, value.clamp(0.0, 1.0)))
}

/// `volume:<0..1>` → level.
pub fn parse_volume_action(action: &str) -> Option<f32> {
    action.strip_prefix("volume:")?.parse::<f32>().ok().filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 1.0))
}

// ---- timing ------------------------------------------------------------------------------

/// One HUD's timing: visible or not, open (card expanded), when it goes, and coalesced
/// publishing. The module calls `changed` / `interacted` and publishes when told to.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Hud {
    pub visible: bool,
    pub open: bool,
    pub hide_at: Option<Instant>,
    last_publish: Option<Instant>,
    /// A publish held back by coalescing.
    publish_at: Option<Instant>,
}

/// What a HUD should do now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Due {
    pub publish: bool,
    pub retract: bool,
}

impl Hud {
    /// Something to show changed: stay `linger` longer (unless open) and publish now, or once
    /// the coalescing interval is over. `true` = publish now.
    pub fn changed(&mut self, now: Instant, linger: Duration) -> bool {
        self.visible = true;
        if !self.open {
            self.hide_at = Some(self.hide_at.map_or(now + linger, |at| at.max(now + linger)));
        }
        match self.last_publish {
            Some(last) if now < last + COALESCE => {
                self.publish_at = Some(last + COALESCE);
                false
            }
            _ => {
                self.publish_at = None;
                self.last_publish = Some(now);
                true
            }
        }
    }

    /// The user's own change: shown at once (it's their gesture), lingering longer.
    pub fn interacted(&mut self, now: Instant) {
        self.visible = true;
        if !self.open {
            self.hide_at = Some(now + LINGER_INTERACTION);
        }
        self.publish_at = None;
        self.last_publish = Some(now);
    }

    /// A value changed while showing, not worth extending the linger for (our own echo).
    /// `true` = publish now.
    pub fn refresh(&mut self, now: Instant) -> bool {
        if !self.visible {
            return false;
        }
        match self.last_publish {
            Some(last) if now < last + COALESCE => {
                self.publish_at = Some(last + COALESCE);
                false
            }
            _ => {
                self.last_publish = Some(now);
                true
            }
        }
    }

    pub fn expanded(&mut self) {
        if self.visible {
            self.open = true;
            self.hide_at = None;
        }
    }

    pub fn collapsed(&mut self, now: Instant) {
        if self.open {
            self.open = false;
            self.hide_at = Some(now + LINGER_INTERACTION);
        }
    }

    /// Gone (retracted, or a setting turned it off).
    pub fn hide(&mut self) {
        *self = Self::default();
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        match (self.hide_at, self.publish_at) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Call when a deadline passed.
    pub fn tick(&mut self, now: Instant) -> Due {
        let mut due = Due::default();
        if self.hide_at.is_some_and(|at| now >= at) && !self.open {
            *self = Self::default();
            due.retract = true;
            return due;
        }
        if self.publish_at.is_some_and(|at| now >= at) {
            self.publish_at = None;
            self.last_publish = Some(now);
            due.publish = self.visible;
        }
        due
    }
}

/// Whether a volume change should show the HUD: not our own (the HUD already shows it), and
/// not the level jump that follows a default-device switch.
pub fn volume_shows(ours: bool, now: Instant, switched_at: Option<Instant>) -> bool {
    !ours && switched_at.is_none_or(|at| now >= at + DEVICE_SWITCH_QUIET)
}

/// Whether an echo of our own volume write may update the HUD: not while a drag is still
/// sending values (it would pull the bar back to an older one).
pub fn ours_applies(last_write: Option<Instant>, now: Instant) -> bool {
    last_write.is_none_or(|at| now >= at + OURS_SETTLE)
}

/// Whether a built-in panel brightness change should show. Our own writes never do. With
/// `show_automatic` off, only changes a brightness key or a ReUI action asked for in the
/// last second; but only once a brightness key has actually been seen (many laptops handle
/// those keys in firmware, invisible to apps, and filtering there would hide every change).
pub fn brightness_shows(now: Instant, our_write: Option<Instant>, last_key: Option<Instant>, keys_seen: bool, show_automatic: bool) -> bool {
    if our_write.is_some_and(|at| now < at + BRIGHTNESS_ECHO) {
        return false;
    }
    show_automatic || !keys_seen || last_key.is_some_and(|at| now < at + BRIGHTNESS_INTENT)
}

/// Display labels: the monitor's own name, else "Built-in display" / "Display N".
pub fn display_label(friendly: Option<&str>, internal: bool, n: usize) -> String {
    match friendly.map(str::trim).filter(|s| !s.is_empty()) {
        Some(name) => name.to_owned(),
        None if internal => "Built-in display".into(),
        None => format!("Display {n}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn osd_volume_glyph_tiers() {
        assert_eq!(volume_glyph(0.0, false), GLYPH_VOLUME_0);
        assert_eq!(volume_glyph(f32::NAN, false), GLYPH_VOLUME_0);
        assert_eq!(volume_glyph(0.02, false), GLYPH_VOLUME_1);
        assert_eq!(volume_glyph(0.33, false), GLYPH_VOLUME_1);
        assert_eq!(volume_glyph(0.5, false), GLYPH_VOLUME_2);
        assert_eq!(volume_glyph(0.67, false), GLYPH_VOLUME_3);
        assert_eq!(volume_glyph(1.0, false), GLYPH_VOLUME_3);
        assert_eq!(volume_glyph(0.8, true), GLYPH_MUTED);
        assert_eq!(hud_glyph(0.8, false, DeviceKind::Headphones), GLYPH_HEADPHONES);
        assert_eq!(hud_glyph(0.8, true, DeviceKind::Headphones), GLYPH_MUTED);
        assert_eq!(hud_glyph(0.8, false, DeviceKind::Speakers), GLYPH_VOLUME_3);
        assert_eq!(hud_glyph(0.1, false, DeviceKind::Display), GLYPH_DISPLAY_AUDIO);
        assert_eq!(brightness_glyph(0.2), GLYPH_BRIGHTNESS_LOW);
        assert_eq!(brightness_glyph(0.9), GLYPH_BRIGHTNESS_HIGH);
    }

    fn hud() -> VolumeHud {
        let output = |id: &str, default| Output { id: id.into(), name: format!("Speaker {id}"), default };
        VolumeHud { level: 0.42, muted: false, device: "Speakers".into(), kind: DeviceKind::Speakers, outputs: vec![output("a", true), output("b", false)], can_switch: true, show_percentage: false, mixer: None }
    }

    #[test]
    fn osd_volume_view_is_glyph_and_level() {
        let v = volume_view(&hud());
        assert_eq!(v.compact.leading, Element::Art(Art::Glyph(GLYPH_VOLUME_2)));
        assert_eq!(v.compact.trailing, Element::Level { value: 0.42, dim: false });
        assert_eq!(v.minimal, Element::Ring(0.42));
        let e = v.expanded.unwrap();
        assert_eq!((e.headline, e.detail.as_deref(), e.eyebrow), (Headline::Text("Volume".into()), Some("Speakers"), None));
        let Row::Slider { leading, value, action, trailing } = &e.rows[0] else { panic!() };
        assert_eq!((leading.action.as_str(), *value, action.as_str()), ("mute", 0.42, "volume:"));
        let options: Vec<_> = trailing.as_ref().unwrap().options.iter().map(|o| (o.action.as_str(), o.selected)).collect();
        assert_eq!(options, [("output:a", true), ("output:b", false)]);

        let muted = volume_view(&VolumeHud { muted: true, show_percentage: true, can_switch: false, ..hud() });
        assert_eq!(muted.compact.trailing, Element::Level { value: 0.42, dim: true });
        assert_eq!(muted.compact.leading, Element::Art(Art::Glyph(GLYPH_MUTED)));
        let e = muted.expanded.unwrap();
        assert_eq!(e.detail.as_deref(), Some("Speakers · 42%"));
        let Row::Slider { trailing, .. } = &e.rows[0] else { panic!() };
        assert_eq!(trailing.as_ref().unwrap().options[0].action, "output:settings");
    }

    #[test]
    fn osd_the_mixer_sits_under_the_slider() {
        let closed = volume_view(&hud()).expanded.unwrap();
        assert_eq!(closed.rows.len(), 1);
        assert!(matches!(&closed.trailing, Element::Button(b) if b.action == ACTION_MIXER && b.role == ButtonRole::Secondary));
        let channel = Channel { label: "Spotify".into(), icon: None, value: 0.5, muted: false, action: "app:1:".into(), mute: "app-mute:1".into() };
        let open = volume_view(&VolumeHud { mixer: Some(vec![channel.clone()]), ..hud() }).expanded.unwrap();
        assert_eq!(open.rows[1], Row::Mixer(vec![channel]));
        assert!(matches!(&open.trailing, Element::Button(b) if b.role == ButtonRole::Accent));
        // Nothing playing: the button says it's on, no empty row, no note.
        assert_eq!(volume_view(&VolumeHud { mixer: Some(Vec::new()), ..hud() }).expanded.unwrap().rows.len(), 1);
    }


    #[test]
    fn osd_device_and_state_views() {
        let v = device_view("AirPods", DeviceKind::Headphones);
        assert_eq!(v.compact.leading, Element::Art(Art::Glyph(GLYPH_HEADPHONES)));
        assert_eq!(v.compact.trailing, Element::Text { text: "AirPods".into(), tone: Tone::Secondary });
        assert!(v.expanded.is_none());
        let s = state_view(GLYPH_CAPS_LOCK, "Caps Lock on");
        assert_eq!(s.compact.trailing, Element::Text { text: "Caps Lock on".into(), tone: Tone::Secondary });
    }

    #[test]
    fn osd_brightness_view_has_a_slider_per_display() {
        let d = |id: &str, level| DisplayLevel { id: id.into(), name: format!("Name {id}"), level };
        assert!(brightness_view(&[], 0, false).is_none());
        let one = brightness_view(&[d("internal", 0.7)], 0, true).unwrap();
        assert_eq!(one.compact.trailing, Element::Level { value: 0.7, dim: false });
        let e = one.expanded.unwrap();
        assert_eq!((e.detail.as_deref(), e.rows.len()), (Some("Name internal · 70%"), 1));
        let Row::Slider { action, .. } = &e.rows[0] else { panic!() };
        assert_eq!(action, "brightness:internal:");

        let two = brightness_view(&[d("internal", 0.7), d("ddc-DISPLAY2-0", 0.2)], 1, false).unwrap();
        assert_eq!(two.compact.leading, Element::Art(Art::Glyph(GLYPH_BRIGHTNESS_LOW)));
        let e = two.expanded.unwrap();
        assert_eq!(e.detail, None);
        assert_eq!(e.rows.len(), 4);
        assert_eq!(e.rows[2], Row::Note("Name ddc-DISPLAY2-0".into()));
    }

    #[test]
    fn osd_actions_parse() {
        assert_eq!(parse_volume_action("volume:0.5"), Some(0.5));
        assert_eq!(parse_volume_action("volume:7"), Some(1.0));
        assert_eq!(parse_volume_action("volume:NaN"), None);
        assert_eq!(parse_brightness_action("brightness:internal:0.25"), Some(("internal", 0.25)));
        assert_eq!(parse_brightness_action("brightness:ddc-DISPLAY2-0:-1"), Some(("ddc-DISPLAY2-0", 0.0)));
        assert_eq!(parse_brightness_action("brightness::0.3"), None);
        assert_eq!(parse_brightness_action("brightness:0.3"), None);
    }

    #[test]
    fn osd_bursts_coalesce_and_linger() {
        let t0 = Instant::now();
        let mut h = Hud::default();
        assert!(h.changed(t0, LINGER));
        // A held key at ~30/s: the next change inside 33 ms waits for the interval.
        assert!(!h.changed(t0 + ms(10), LINGER));
        assert_eq!(h.next_deadline(), Some(t0 + COALESCE));
        assert_eq!(h.tick(t0 + COALESCE), Due { publish: true, retract: false });
        assert!(h.changed(t0 + ms(80), LINGER));
        // Lingers 1.5 s after the last change.
        assert_eq!(h.hide_at, Some(t0 + ms(80) + LINGER));
        assert_eq!(h.tick(t0 + ms(1000)), Due::default());
        assert_eq!(h.tick(t0 + ms(80) + LINGER), Due { publish: false, retract: true });
        assert!(!h.visible && h.next_deadline().is_none());
    }

    #[test]
    fn osd_open_cards_stay_and_interactions_linger_longer() {
        let t0 = Instant::now();
        let mut h = Hud::default();
        h.changed(t0, LINGER);
        h.expanded();
        assert!(h.open && h.hide_at.is_none());
        h.changed(t0 + ms(100), LINGER);
        assert!(!h.tick(t0 + ms(10_000)).retract);
        h.collapsed(t0 + ms(10_000));
        assert_eq!(h.hide_at, Some(t0 + ms(10_000) + LINGER_INTERACTION));
        h.interacted(t0 + ms(11_000));
        assert_eq!(h.hide_at, Some(t0 + ms(11_000) + LINGER_INTERACTION));
        // An echo refreshes without extending.
        assert!(h.refresh(t0 + ms(11_100)));
        assert_eq!(h.hide_at, Some(t0 + ms(11_000) + LINGER_INTERACTION));
        // Expanding a hidden HUD does nothing.
        let mut idle = Hud::default();
        idle.expanded();
        assert!(!idle.open && !idle.refresh(t0));
    }

    #[test]
    fn osd_echoes_and_device_switches_stay_quiet() {
        let t0 = Instant::now();
        assert!(volume_shows(false, t0, None));
        assert!(!volume_shows(true, t0, None));
        assert!(!volume_shows(false, t0 + ms(499), Some(t0)));
        assert!(volume_shows(false, t0 + ms(500), Some(t0)));
        assert!(!ours_applies(Some(t0), t0 + ms(100)));
        assert!(ours_applies(Some(t0), t0 + ms(300)));
        assert!(ours_applies(None, t0));
    }

    #[test]
    fn osd_automatic_brightness_needs_a_reason() {
        let t0 = Instant::now();
        // Our own write's echo never shows.
        assert!(!brightness_shows(t0 + ms(399), Some(t0), Some(t0), true, true));
        // Keys never seen: the filter can't work, show everything.
        assert!(brightness_shows(t0, None, None, false, false));
        // Keys seen: only within a second of one.
        assert!(brightness_shows(t0 + ms(900), None, Some(t0), true, false));
        assert!(!brightness_shows(t0 + ms(1000), None, Some(t0), true, false));
        assert!(!brightness_shows(t0, None, None, true, false));
        // Unless automatic changes are wanted.
        assert!(brightness_shows(t0, None, None, true, true));
    }

    #[test]
    fn osd_display_labels() {
        assert_eq!(display_label(Some("DELL U2720Q"), false, 2), "DELL U2720Q");
        assert_eq!(display_label(Some("  "), true, 1), "Built-in display");
        assert_eq!(display_label(None, false, 3), "Display 3");
    }
}
