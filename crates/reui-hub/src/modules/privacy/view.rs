//! The indicator and the "started" moment as views (PRD 5.1). Pure. Colour comes only from
//! the activity accent: the most important kind in use (the iOS colours: green camera,
//! orange microphone; purple screen, blue location).

use reui_protocol::{Art, Button, ButtonContent, ButtonRole, CardSize, CompactView, Element, ExpandedView, Headline, ImageId, Rgb, Row, Tone};

use super::consent::{App, Kind};
use crate::hub::View;

/// Apple's dark system colours (the island's `color.system` tokens).
const GREEN: Rgb = Rgb(48, 209, 88);
const ORANGE: Rgb = Rgb(255, 159, 10);
const PURPLE: Rgb = Rgb(191, 90, 242);
const BLUE: Rgb = Rgb(10, 132, 255);

pub const GLYPH_MIC: char = '\u{E720}';
pub const GLYPH_MIC_OFF: char = '\u{EC54}';
pub const GLYPH_CAMERA: char = '\u{E722}';
pub const GLYPH_SCREEN: char = '\u{E7F4}';
pub const GLYPH_LOCATION: char = '\u{E81D}';

pub const ACTION_MUTE: &str = "mute";
pub const ACTION_UNMUTE: &str = "unmute";
pub const ACTION_SETTINGS: &str = "settings";

pub fn accent(kind: Kind) -> Rgb {
    match kind {
        Kind::Screen => PURPLE,
        Kind::Camera => GREEN,
        Kind::Microphone => ORANGE,
        Kind::Location => BLUE,
    }
}

pub fn glyph(kind: Kind) -> char {
    match kind {
        Kind::Screen => GLYPH_SCREEN,
        Kind::Camera => GLYPH_CAMERA,
        Kind::Microphone => GLYPH_MIC,
        Kind::Location => GLYPH_LOCATION,
    }
}

fn noun(kind: Kind) -> &'static str {
    match kind {
        Kind::Screen => "Screen",
        Kind::Camera => "Camera",
        Kind::Microphone => "Microphone",
        Kind::Location => "Location",
    }
}

/// "Camera", "Camera and microphone", "Screen, camera and microphone".
pub fn kinds_label(kinds: &[Kind]) -> String {
    let words: Vec<String> = kinds.iter().enumerate().map(|(i, k)| if i == 0 { noun(*k).to_owned() } else { noun(*k).to_lowercase() }).collect();
    match words.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// A readable name for a packaged app's family ("Microsoft.WindowsCamera_8wekyb3d8bbwe" →
/// "Camera"); desktop apps go through the clipboard's exe naming.
pub fn packaged_name(family: &str) -> String {
    const KNOWN: [(&str, &str); 6] = [
        ("Microsoft.WindowsCamera", "Camera"),
        ("Microsoft.WindowsSoundRecorder", "Sound Recorder"),
        ("Microsoft.ScreenSketch", "Snipping Tool"),
        ("MSTeams", "Teams"),
        ("Microsoft.BingWeather", "Weather"),
        ("Microsoft.WindowsMaps", "Maps"),
    ];
    let name = family.rsplit_once('_').map_or(family, |(name, _)| name);
    if let Some((_, known)) = KNOWN.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
        return (*known).to_owned();
    }
    let last = name.rsplit('.').next().unwrap_or(name);
    let last = last.strip_prefix("Windows").filter(|rest| !rest.is_empty()).unwrap_or(last);
    let mut chars = last.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_else(|| "App".into())
}

/// One use as shown: which app (`app` groups an app's kinds), its name, icon and start.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub kind: Kind,
    pub app: App,
    pub name: String,
    pub icon: Option<ImageId>,
    /// "10:42" (the time of day the use started).
    pub since: String,
}

fn app_art(item: &Item) -> Element {
    Element::Art(item.icon.map_or(Art::Glyph(glyph(item.kind)), Art::Image))
}

/// The indicator: `items` most important first (see `tracker::Tracker::shown`). `muted` is
/// the default microphone's mute, when known.
pub fn indicator(items: &[Item], muted: Option<bool>) -> Option<View> {
    let main = items.first()?;
    let muted = muted == Some(true);
    let mine: Vec<Kind> = items.iter().filter(|i| i.app == main.app).map(|i| i.kind).collect();
    let mic_only_muted = muted && main.kind == Kind::Microphone;
    let trailing = if mic_only_muted { Element::Glyph { glyph: GLYPH_MIC_OFF, tone: Tone::Secondary } } else { Element::Glyph { glyph: glyph(main.kind), tone: Tone::Accent } };

    // Other apps, one line each with what they use.
    let mut others: Vec<(&Item, Vec<Kind>)> = Vec::new();
    for item in items.iter().filter(|i| i.app != main.app) {
        match others.iter_mut().find(|(o, _)| o.app == item.app) {
            Some((_, kinds)) => kinds.push(item.kind),
            None => others.push((item, vec![item.kind])),
        }
    }
    // What they use is said only when it isn't what the eyebrow already says.
    let mut rows: Vec<Row> = others.iter().map(|(o, kinds)| Row::Note(if *kinds == mine { o.name.clone() } else { format!("{} · {}", o.name, kinds_label(kinds)) })).collect();
    let mut buttons = Vec::new();
    if items.iter().any(|i| i.kind == Kind::Microphone) {
        let (glyph, action, role) = if muted { (GLYPH_MIC_OFF, ACTION_UNMUTE, ButtonRole::Accent) } else { (GLYPH_MIC, ACTION_MUTE, ButtonRole::Secondary) };
        buttons.push(Button { action: action.into(), content: ButtonContent::Glyph(glyph), role });
    }
    buttons.push(Button { action: ACTION_SETTINGS.into(), content: ButtonContent::Text("Privacy settings".into()), role: ButtonRole::Secondary });
    rows.push(Row::Buttons(buttons));

    let detail = if muted && mine.contains(&Kind::Microphone) { "Muted".to_owned() } else { format!("Since {}", main.since) };
    let expanded = ExpandedView {
        leading: app_art(main),
        eyebrow: Some(kinds_label(&mine)),
        headline: Headline::Text(main.name.clone()),
        detail: Some(detail),
        trailing: Element::Empty,
        rows,
        context: None,
        size: CardSize::Regular,
        tint: false,
        pager: None,
        open: None,
    };
    Some(View { accent: Some(accent(main.kind)), compact: CompactView { leading: app_art(main), trailing }, minimal: Element::Dot, expanded: Some(expanded), audio: None })
}

/// Another app (or kind) started while the indicator was up: its icon and what it uses,
/// for a moment. Clicking opens the indicator.
pub fn started(item: &Item) -> View {
    let mark = Element::Glyph { glyph: glyph(item.kind), tone: Tone::Accent };
    View { accent: Some(accent(item.kind)), compact: CompactView { leading: app_art(item), trailing: mark.clone() }, minimal: mark, expanded: None, audio: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: Kind, exe: &str) -> Item {
        Item { kind, app: App::Exe(exe.into()), name: exe.trim_end_matches(".exe").into(), icon: None, since: "10:42".into() }
    }

    #[test]
    fn privacy_labels_read_as_a_phrase() {
        assert_eq!(kinds_label(&[Kind::Camera]), "Camera");
        assert_eq!(kinds_label(&[Kind::Camera, Kind::Microphone]), "Camera and microphone");
        assert_eq!(kinds_label(&[Kind::Screen, Kind::Camera, Kind::Microphone]), "Screen, camera and microphone");
        assert_eq!(packaged_name("Microsoft.WindowsCamera_8wekyb3d8bbwe"), "Camera");
        assert_eq!(packaged_name("Microsoft.WindowsAlarms_8wekyb3d8bbwe"), "Alarms");
        assert_eq!(packaged_name("SpotifyAB.SpotifyMusic_zpdnekdrzrea0"), "SpotifyMusic");
    }

    #[test]
    fn privacy_indicator_leads_with_the_main_app() {
        let items = [item(Kind::Camera, "Zoom.exe"), item(Kind::Microphone, "Zoom.exe"), item(Kind::Microphone, "chrome.exe")];
        let v = indicator(&items, Some(false)).unwrap();
        assert_eq!(v.accent, Some(GREEN));
        assert_eq!(v.compact.trailing, Element::Glyph { glyph: GLYPH_CAMERA, tone: Tone::Accent });
        assert_eq!(v.compact.leading, Element::Art(Art::Glyph(GLYPH_CAMERA)), "a glyph until the icon arrives");
        let e = v.expanded.unwrap();
        assert_eq!((e.eyebrow.as_deref(), e.detail.as_deref()), (Some("Camera and microphone"), Some("Since 10:42")));
        assert_eq!(e.rows[0], Row::Note("chrome · Microphone".into()));
        let same = indicator(&[item(Kind::Microphone, "a.exe"), item(Kind::Microphone, "b.exe")], None).unwrap().expanded.unwrap();
        assert_eq!(same.rows[0], Row::Note("b".into()), "the eyebrow already says microphone");
        let Row::Buttons(b) = &e.rows[1] else { panic!() };
        assert_eq!((b[0].action.as_str(), b[1].action.as_str()), (ACTION_MUTE, ACTION_SETTINGS));
        assert!(indicator(&[], None).is_none());
    }

    #[test]
    fn privacy_a_muted_microphone_reads_as_muted() {
        let v = indicator(&[item(Kind::Microphone, "Discord.exe")], Some(true)).unwrap();
        assert_eq!(v.compact.trailing, Element::Glyph { glyph: GLYPH_MIC_OFF, tone: Tone::Secondary });
        let e = v.expanded.unwrap();
        assert_eq!(e.detail.as_deref(), Some("Muted"));
        let Row::Buttons(b) = &e.rows[0] else { panic!() };
        assert_eq!((b[0].action.as_str(), b[0].role), (ACTION_UNMUTE, ButtonRole::Accent));
        // No microphone, no mute button.
        let e = indicator(&[item(Kind::Screen, "obs64.exe")], Some(true)).unwrap().expanded.unwrap();
        assert_eq!(e.rows, vec![Row::Buttons(vec![Button { action: ACTION_SETTINGS.into(), content: ButtonContent::Text("Privacy settings".into()), role: ButtonRole::Secondary }])]);
        assert_eq!(started(&item(Kind::Screen, "obs64.exe")).accent, Some(PURPLE));
    }
}
