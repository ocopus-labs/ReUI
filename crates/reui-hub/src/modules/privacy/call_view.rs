//! The ongoing call as a view (docs/PRD-calls.md 5.1). Pure. The time is a `Clock` the
//! island advances from the microphone's start, so the hub publishes only when something
//! about the call changes (muted, the camera, a screen share), never per second.

use reui_protocol::{Art, Button, ButtonContent, ButtonRole, CardSize, Clock, CompactView, Element, ExpandedView, Headline, ImageId, Rgb, Row, Tone};

use super::calls::Call;
use super::view::{ACTION_MUTE, ACTION_UNMUTE, GLYPH_MIC, GLYPH_MIC_OFF};
use crate::hub::View;

/// iOS's call green (the island's `color.system.green`).
const GREEN: Rgb = Rgb(48, 209, 88);
pub const GLYPH_PHONE: char = '\u{E717}';
pub const ACTION_OPEN_APP: &str = "open-app";

/// A call with what the module knows about its app.
#[derive(Debug, Clone, PartialEq)]
pub struct CallItem {
    pub call: Call,
    pub name: String,
    pub icon: Option<ImageId>,
}

/// The activity key: one per app, so two call apps at once are two activities (D4).
pub fn key(call: &Call) -> String {
    let id = match &call.app {
        super::consent::App::Exe(_) => call.app.file_name().unwrap_or_default(),
        super::consent::App::Packaged(family) => family.to_lowercase(),
    };
    format!("call:{id}")
}

/// `muted` is the default microphone's mute, when known: the system's, which is why the
/// card says "Microphone muted" rather than "Muted" (the app's own mute may disagree).
pub fn call(item: &CallItem, muted: Option<bool>) -> View {
    let muted = muted == Some(true);
    let clock = Clock { value_ms: 0, at_unix_ms: item.call.started_ms, running: true, down: false };
    let art = Element::Art(item.icon.map_or(Art::Glyph(GLYPH_PHONE), Art::Image));
    let mic_off = Element::Glyph { glyph: GLYPH_MIC_OFF, tone: Tone::Negative };
    let fact = if muted {
        Some("Microphone muted")
    } else if item.call.screen {
        Some("Sharing screen")
    } else {
        None
    };
    let detail = match fact {
        Some(fact) => format!("{} · {fact}", item.name),
        None => item.name.clone(),
    };
    let (glyph, action, role) = if muted { (GLYPH_MIC_OFF, ACTION_UNMUTE, ButtonRole::Accent) } else { (GLYPH_MIC, ACTION_MUTE, ButtonRole::Secondary) };
    let buttons = vec![
        Button { action: action.into(), content: ButtonContent::Glyph(glyph), role },
        Button { action: ACTION_OPEN_APP.into(), content: ButtonContent::Text(format!("Open {}", item.name)), role: ButtonRole::Secondary },
    ];
    let expanded = ExpandedView {
        leading: art.clone(),
        eyebrow: Some(if item.call.camera { "Video call" } else { "Call" }.into()),
        headline: Headline::Clock { clock, tenths: false },
        detail: Some(detail),
        trailing: Element::Empty,
        rows: vec![Row::Buttons(buttons)],
        context: None,
        size: CardSize::Regular,
        tint: false,
        pager: None,
        open: Some(ACTION_OPEN_APP.into()),
    };
    View {
        accent: Some(GREEN),
        compact: CompactView { leading: if muted { mic_off.clone() } else { art }, trailing: Element::Clock(clock) },
        minimal: if muted { mic_off } else { Element::Glyph { glyph: GLYPH_PHONE, tone: Tone::Accent } },
        expanded: Some(expanded),
        audio: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::privacy::calls::Family;
    use crate::modules::privacy::consent::App;

    fn item(camera: bool, screen: bool) -> CallItem {
        let app = App::Exe(r"C:\Apps\ms-teams.exe".into());
        CallItem { call: Call { app, family: Family::Teams, started_ms: 1_000, camera, screen, exe: None }, name: "Teams".into(), icon: None }
    }

    #[test]
    fn privacy_a_call_reads_as_its_time() {
        let v = call(&item(false, false), Some(false));
        assert_eq!(v.accent, Some(GREEN));
        let clock = Clock { value_ms: 0, at_unix_ms: 1_000, running: true, down: false };
        assert_eq!(v.compact, CompactView { leading: Element::Art(Art::Glyph(GLYPH_PHONE)), trailing: Element::Clock(clock) });
        let e = v.expanded.unwrap();
        assert_eq!((e.eyebrow.as_deref(), e.detail.as_deref()), (Some("Call"), Some("Teams")));
        assert_eq!(e.headline, Headline::Clock { clock, tenths: false });
        let Row::Buttons(b) = &e.rows[0] else { panic!() };
        assert_eq!((b[0].action.as_str(), &b[1].content), (ACTION_MUTE, &ButtonContent::Text("Open Teams".into())));
        assert_eq!(key(&item(false, false).call), "call:ms-teams.exe");
    }

    #[test]
    fn privacy_a_muted_call_says_so_first() {
        let v = call(&item(true, true), Some(true));
        assert_eq!(v.compact.leading, Element::Glyph { glyph: GLYPH_MIC_OFF, tone: Tone::Negative });
        let e = v.expanded.unwrap();
        assert_eq!((e.eyebrow.as_deref(), e.detail.as_deref()), (Some("Video call"), Some("Teams · Microphone muted")));
        let Row::Buttons(b) = &e.rows[0] else { panic!() };
        assert_eq!((b[0].action.as_str(), b[0].role), (ACTION_UNMUTE, ButtonRole::Accent));
        let sharing = call(&item(false, true), None).expanded.unwrap();
        assert_eq!(sharing.detail.as_deref(), Some("Teams · Sharing screen"));
    }
}
