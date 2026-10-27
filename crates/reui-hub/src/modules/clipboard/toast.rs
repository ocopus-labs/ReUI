//! What the clipboard shows on the island: the copy toast (the thing itself and a glimpse
//! of it), the paste stack's counter, and the Press Ctrl+V hint.

use reui_protocol::{Art, CompactView, Element, Priority, Tone};
use tokio::time::Instant;

use super::preview::prefix;
use super::store::{Kind, Saved};
use super::{App, ACCENT, BLOCKED_TOAST, KEY, STACK_KEY, TOAST};
use crate::hub::{ModuleContext, View};

const GLYPH_TEXT: char = '\u{E8D2}';
const GLYPH_LINK: char = '\u{E71B}';
const GLYPH_FILE: char = '\u{E8A5}';
const GLYPH_COLOR: char = '\u{E790}';
const GLYPH_COPY: char = '\u{E8C8}';
const GLYPH_PASTE: char = '\u{E77F}';

impl App {
    /// "Stack 2/5" in the island while a stack is running.
    pub(super) fn show_stack(&self) {
        let Some(stack) = &self.stack else { return };
        let leading = Element::Art(Art::Glyph(GLYPH_PASTE));
        let text = format!("Stack {}/{}", stack.at + 1, stack.items.len());
        let view = View {
            accent: Some(ACCENT),
            audio: None,
            compact: CompactView { leading: leading.clone(), trailing: Element::Text { text, tone: Tone::Secondary } },
            minimal: leading,
            expanded: None,
        };
        self.ctx.publish(STACK_KEY, Priority::Ongoing, view);
    }

    /// The item is on the clipboard; the target wouldn't take our keystrokes (elevated,
    /// gone, or focus couldn't be given back), so the user pastes it (E-P1, W-2, W-5).
    pub(super) fn press_ctrl_v(&mut self) {
        let leading = Element::Art(Art::Glyph(GLYPH_PASTE));
        let view = View {
            accent: Some(ACCENT),
            audio: None,
            compact: CompactView { leading: leading.clone(), trailing: Element::Text { text: "Press Ctrl+V".into(), tone: Tone::Secondary } },
            minimal: leading,
            expanded: None,
        };
        self.ctx.publish(KEY, Priority::Alert, view);
        self.hide_at = Some(Instant::now() + BLOCKED_TOAST);
    }

    /// The copy confirmation: the newest item's thumbnail or glyph and a glimpse of it.
    pub(super) async fn toast(&mut self) {
        let Some(Some(latest)) = self.db.call(|s| s.latest()).await else { return };
        self.ctx.publish(KEY, Priority::Brief, toast_view(&self.ctx, &latest));
        self.hide_at = Some(Instant::now() + TOAST);
    }
}

/// The toast: no panel, just the thing itself (thumbnail or glyph) and a glimpse of it. In
/// the bubble beside something else playing, the copy icon says what happened.
fn toast_view(ctx: &ModuleContext, latest: &Saved) -> View {
    let leading = match &latest.thumb {
        Some(thumb) if latest.category == Kind::Images.name() => Element::Art(Art::Image(ctx.image(thumb.clone()))),
        _ => Element::Art(Art::Glyph(glyph_for(latest))),
    };
    View {
        accent: Some(ACCENT),
        audio: None,
        compact: CompactView { leading, trailing: glimpse(latest).map_or(Element::Empty, |text| Element::Text { text, tone: Tone::Secondary }) },
        minimal: Element::Art(Art::Glyph(GLYPH_COPY)),
        expanded: None,
    }
}

fn glyph_for(saved: &Saved) -> char {
    match saved.category.as_str() {
        "links" => GLYPH_LINK,
        "files" => GLYPH_FILE,
        "colors" => GLYPH_COLOR,
        _ => GLYPH_TEXT,
    }
}

/// A few words of what was copied, for the toast: the start of the text, an image's size,
/// a file's name, a link's site.
fn glimpse(saved: &Saved) -> Option<String> {
    const CHARS: usize = 24;
    let text = match saved.category.as_str() {
        "images" => return saved.size.map(|(w, h)| format!("{w} × {h}")),
        "files" => {
            let name = saved.files.first().map(|f| f.rsplit(['\\', '/']).next().unwrap_or(f))?;
            return Some(match saved.files.len() {
                1 => name.to_string(),
                n => format!("{name} +{}", n - 1),
            });
        }
        "links" => return saved.link.site.clone().or_else(|| saved.text.as_deref().map(|t| host_of(t).to_string())),
        _ => saved.text.as_deref()?.trim().lines().next()?.trim(),
    };
    let cut = prefix(text, CHARS);
    Some(if cut.len() < text.len() { format!("{}…", cut.trim_end()) } else { cut.to_string() })
}

/// `example.com` from `https://www.example.com/path`.
fn host_of(url: &str) -> &str {
    let rest = url.trim().split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    host.strip_prefix("www.").unwrap_or(host)
}

#[cfg(test)]
mod tests {
    use crate::hub::Hub;

    use super::*;

    fn saved(category: &str) -> Saved {
        Saved { id: 7, category: category.into(), created_ms: 0, ..Default::default() }
    }

    #[test]
    fn toasts_show_the_thing_and_never_a_panel() {
        let hub = Hub::new();
        let ctx = hub.context("clipboard");
        let view = toast_view(&ctx, &Saved { text: Some("Hello there, this is a long first line\nsecond".into()), ..saved("text") });
        assert!(view.expanded.is_none());
        assert_eq!(view.compact.trailing, Element::Text { text: "Hello there, this is a l…".into(), tone: Tone::Secondary });
        assert_eq!(view.minimal, Element::Art(Art::Glyph(GLYPH_COPY)));
        let image = toast_view(&ctx, &Saved { thumb: Some(vec![1, 2]), size: Some((640, 400)), ..saved("images") });
        assert!(matches!(image.compact.leading, Element::Art(Art::Image(_))));
        assert_eq!(glimpse(&Saved { size: Some((640, 400)), ..saved("images") }).as_deref(), Some("640 × 400"));
        assert_eq!(glimpse(&Saved { files: vec![r"C:\a\b.txt".into(), "c".into()], ..saved("files") }).as_deref(), Some("b.txt +1"));
        assert_eq!(glimpse(&Saved { text: Some("https://www.figma.com/x".into()), ..saved("links") }).as_deref(), Some("figma.com"));
    }
}
