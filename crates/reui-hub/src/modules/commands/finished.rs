//! A long terminal command that finished (docs/PRD-terminal-commands.md): what the shell's
//! hook reported, and how it reads. The pill: a check or a cross and Done / Failed. Open:
//! the shell as eyebrow, the command as the hero (a code chip, cut by the island), the
//! exit code (when it failed) and how long it took, and Open terminal when the bridge said
//! where the shell runs. Pure.

use std::time::Duration;

use reui_protocol::{Art, Button, ButtonContent, ButtonRole, CardSize, CompactView, Element, ExpandedView, Headline, Priority, Rgb, Row, Tone};
use serde_json::Value;

use crate::hub::View;
use crate::modules::agent::terminal::Origin;
use crate::modules::clipboard::secrets;

const DONE_ACCENT: Rgb = Rgb(48, 209, 88);
const FAILED_ACCENT: Rgb = Rgb(255, 69, 58);
const GLYPH_DONE: char = '\u{E73E}';
const GLYPH_FAILED: char = '\u{E711}';
const GLYPH_TERMINAL: char = '\u{E756}';

/// The card's one button.
pub const ACTION_TERMINAL: &str = "terminal";

/// How long a finished command stays: long enough to come back to after the coffee it was
/// started for; a failure longer, as it's the one you need to act on.
pub const DONE_STAYS: Duration = Duration::from_secs(5 * 60);
pub const FAILED_STAYS: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Debug, PartialEq)]
pub struct Finished {
    pub command: String,
    pub exit: i64,
    pub seconds: u64,
    /// "powershell", "bash": the eyebrow.
    pub shell: String,
    pub origin: Option<Origin>,
}

impl Finished {
    /// From the bridge's `finished:<json>` payload.
    pub fn parse(json: &str) -> Option<Self> {
        let v: Value = serde_json::from_str(json).ok()?;
        let command = v.get("command")?.as_str()?.trim().to_string();
        if command.is_empty() {
            return None;
        }
        Some(Self {
            command,
            exit: v.get("exit")?.as_i64()?,
            seconds: v.get("seconds")?.as_u64()?,
            shell: v.get("shell").and_then(Value::as_str).unwrap_or_default().to_string(),
            origin: Origin::of(&v),
        })
    }

    pub fn ok(&self) -> bool {
        self.exit == 0
    }

    /// Which terminal it ran in, so a newer command there replaces it: the shell's process
    /// (the bridge's nearest ancestor), else one card for every terminal we can't tell apart.
    pub fn key(&self) -> String {
        match self.origin.as_ref().and_then(|o| o.parents.first()) {
            Some(pid) => format!("t{pid}"),
            None => "t".into(),
        }
    }

    pub fn priority(&self) -> Priority {
        Priority::Alert
    }

    pub fn stays(&self) -> Duration {
        if self.ok() { DONE_STAYS } else { FAILED_STAYS }
    }
}

/// The first line of the command, secrets masked (a token typed on the command line).
fn shown_command(command: &str) -> String {
    let first = command.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    let more = command.lines().filter(|l| !l.trim().is_empty()).count() > 1;
    secrets::mask(&if more { format!("{first} \u{2026}") } else { first.to_string() })
}

fn shell_name(shell: &str) -> &str {
    match shell {
        "powershell" | "pwsh" => "PowerShell",
        "bash" => "Bash",
        "zsh" => "Zsh",
        "" => "Terminal",
        other => other,
    }
}

/// "2m 14s", "1h 3m", "45s".
pub fn took(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m {}s", seconds / 60, seconds % 60),
        _ => format!("{}h {}m", seconds / 3600, seconds % 3600 / 60),
    }
}

pub fn view(f: &Finished) -> View {
    let ok = f.ok();
    let (word, sign, accent, tone) = if ok { ("Done", GLYPH_DONE, DONE_ACCENT, Tone::Positive) } else { ("Failed", GLYPH_FAILED, FAILED_ACCENT, Tone::Negative) };
    let detail = match ok {
        true => format!("{word} \u{00B7} {}", took(f.seconds)),
        false => format!("Exit {} \u{00B7} {}", f.exit, took(f.seconds)),
    };
    let rows = match f.origin {
        Some(_) => vec![Row::Buttons(vec![Button { action: ACTION_TERMINAL.into(), content: ButtonContent::Text("Open terminal".into()), role: ButtonRole::Secondary }])],
        None => Vec::new(),
    };
    View {
        accent: Some(accent),
        audio: None,
        compact: CompactView { leading: Element::Glyph { glyph: sign, tone }, trailing: Element::Text { text: word.into(), tone } },
        minimal: Element::Glyph { glyph: sign, tone },
        expanded: Some(ExpandedView {
            context: None,
            leading: Element::Art(Art::Glyph(GLYPH_TERMINAL)),
            eyebrow: Some(shell_name(&f.shell).to_string()),
            headline: Headline::Code(shown_command(&f.command)),
            detail: Some(detail),
            trailing: Element::Empty,
            rows,
            size: CardSize::Regular,
            tint: false,
            pager: None,
            open: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finished(exit: i64, parents: &str) -> Finished {
        Finished::parse(&format!(r#"{{"command":"cargo build --release","exit":{exit},"seconds":134,"shell":"powershell","reui_parents":{parents},"reui_console":0}}"#)).unwrap()
    }

    #[test]
    fn the_report_is_read_and_keyed_by_its_shell() {
        let f = finished(0, "[400, 300]");
        assert_eq!((f.command.as_str(), f.exit, f.seconds, f.shell.as_str()), ("cargo build --release", 0, 134, "powershell"));
        assert_eq!(f.key(), "t400");
        assert_eq!(finished(1, "[]").key(), "t", "no chain: one card for all");
        assert!(Finished::parse(r#"{"command":"  ","exit":0,"seconds":20}"#).is_none());
        assert!(Finished::parse(r#"{"command":"make","seconds":20}"#).is_none());
        assert!(Finished::parse("not json").is_none());
    }

    #[test]
    fn done_and_failed_read_differently() {
        let done = view(&finished(0, "[400]"));
        let card = done.expanded.unwrap();
        assert_eq!((card.eyebrow.as_deref(), card.detail.as_deref()), (Some("PowerShell"), Some("Done \u{00B7} 2m 14s")));
        assert_eq!(card.headline, Headline::Code("cargo build --release".into()));
        assert_eq!(card.rows.len(), 1, "Open terminal");
        let failed = view(&finished(101, "[]")).expanded.unwrap();
        assert_eq!(failed.detail.as_deref(), Some("Exit 101 \u{00B7} 2m 14s"));
        assert!(failed.rows.is_empty(), "nowhere to go");
        assert!(finished(1, "[]").stays() > finished(0, "[]").stays());
    }

    #[test]
    fn long_commands_show_their_first_line() {
        assert_eq!(shown_command("npm run build\nnpm test"), "npm run build \u{2026}");
        assert_eq!(took(45), "45s");
        assert_eq!(took(3_780), "1h 3m");
    }
}
