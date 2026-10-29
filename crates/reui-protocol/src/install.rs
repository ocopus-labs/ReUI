//! What `reui-settings --install-update` prints, one line per step, for whoever runs it
//! (the hub's updates module reads it to drive the island's card). Plain text on stdout
//! rather than the pipe: the child lives for one install and has nothing else to say.
//!
//! `progress <0..1>` while downloading, `verifying`, `installing` (the setup is starting;
//! it stops ReUI, upgrades and starts it again), or `error <message>`. Exit code 0 means
//! the setup was started, 1 that it wasn't.

use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum InstallStep {
    /// How much of the installer has arrived, 0..=1.
    Progress(f32),
    Verifying,
    Installing,
    /// Why it stopped, as a sentence for the card.
    Error(String),
}

impl fmt::Display for InstallStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstallStep::Progress(p) => write!(f, "progress {:.3}", p.clamp(0.0, 1.0)),
            InstallStep::Verifying => f.write_str("verifying"),
            InstallStep::Installing => f.write_str("installing"),
            // One line: a message never breaks the framing.
            InstallStep::Error(message) => write!(f, "error {}", message.replace(['\r', '\n'], " ")),
        }
    }
}

impl InstallStep {
    /// A printed line back into its step; anything else (a stray log line) is `None`.
    pub fn parse(line: &str) -> Option<Self> {
        let line = line.trim();
        let (word, rest) = line.split_once(' ').map_or((line, ""), |(w, r)| (w, r.trim()));
        match word {
            "progress" => rest.parse::<f32>().ok().filter(|p| p.is_finite()).map(|p| InstallStep::Progress(p.clamp(0.0, 1.0))),
            "verifying" => Some(InstallStep::Verifying),
            "installing" => Some(InstallStep::Installing),
            "error" => Some(InstallStep::Error(if rest.is_empty() { "The update stopped".into() } else { rest.into() })),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_survive_a_round_trip_through_a_line() {
        for step in [InstallStep::Progress(0.0), InstallStep::Progress(0.5), InstallStep::Progress(1.0), InstallStep::Verifying, InstallStep::Installing, InstallStep::Error("Not found on github.com".into())] {
            assert_eq!(InstallStep::parse(&step.to_string()), Some(step));
        }
    }

    #[test]
    fn odd_lines_are_ignored_or_tamed() {
        assert_eq!(InstallStep::parse("progress 1.7"), Some(InstallStep::Progress(1.0)));
        assert_eq!(InstallStep::parse("progress -1"), Some(InstallStep::Progress(0.0)));
        assert_eq!(InstallStep::parse("progress NaN"), None);
        assert_eq!(InstallStep::parse("progress"), None);
        assert_eq!(InstallStep::parse("  installing\r"), Some(InstallStep::Installing));
        assert_eq!(InstallStep::parse("error"), Some(InstallStep::Error("The update stopped".into())));
        assert_eq!(InstallStep::parse("settings: something"), None);
        assert_eq!(InstallStep::parse(""), None);
        assert_eq!(InstallStep::Error("two\nlines".into()).to_string(), "error two lines");
    }
}
