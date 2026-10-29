//! The island's update card as a small state machine: offered, downloading, installing,
//! or failed, moved on by the install's printed steps and its exit. Pure, so every path
//! (a crash mid-download, a setup that never restarts ReUI) is tested without installing
//! anything.

use reui_protocol::install::InstallStep;

use super::state::Offer;

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    /// Later / Install.
    Offered,
    /// How much has arrived, 0..=1 (1 while it's verified).
    Downloading(f32),
    /// The setup has started; it stops ReUI, so this is normally the last thing shown.
    Installing,
    /// Why, for the card; Later / Try again.
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub offer: Offer,
    pub phase: Phase,
}

/// When the install ended without saying why.
pub const STOPPED: &str = "The update stopped before it finished";
/// When the setup started but ReUI is still this version long after.
pub const NO_RESTART: &str = "The installer didn't restart ReUI";

impl Card {
    pub fn new(offer: Offer) -> Self {
        Self { offer, phase: Phase::Offered }
    }

    /// An install is under way: Install and the tray do nothing more meanwhile.
    pub fn busy(&self) -> bool {
        matches!(self.phase, Phase::Downloading(_) | Phase::Installing)
    }

    /// Install (or Try again) was pressed.
    pub fn start(&mut self) {
        self.phase = Phase::Downloading(0.0);
    }

    /// A line the install printed. Once failed, the card waits for an answer.
    pub fn step(&mut self, step: InstallStep) {
        if matches!(self.phase, Phase::Failed(_) | Phase::Offered) {
            return;
        }
        self.phase = match step {
            InstallStep::Progress(p) => Phase::Downloading(p.clamp(0.0, 1.0)),
            InstallStep::Verifying => Phase::Downloading(1.0),
            InstallStep::Installing => Phase::Installing,
            InstallStep::Error(message) => Phase::Failed(message),
        };
    }

    /// The install process ended. A clean exit after "installing" is the setup taking
    /// over; any other end that didn't already say why is a failure.
    pub fn exited(&mut self, success: bool) {
        match self.phase {
            Phase::Installing if success => {}
            Phase::Downloading(_) | Phase::Installing => self.phase = Phase::Failed(STOPPED.into()),
            Phase::Offered | Phase::Failed(_) => {}
        }
    }

    /// Still here long after the setup started: it didn't do its job.
    pub fn stalled(&mut self) {
        if self.phase == Phase::Installing {
            self.phase = Phase::Failed(NO_RESTART.into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card() -> Card {
        Card::new(Offer { version: "0.2.0".into(), line: None })
    }

    #[test]
    fn an_install_runs_from_download_to_the_setup() {
        let mut card = card();
        assert!(!card.busy());
        card.step(InstallStep::Progress(0.5));
        assert_eq!(card.phase, Phase::Offered, "nothing moves before Install");
        card.start();
        assert!(card.busy());
        card.step(InstallStep::Progress(0.4));
        assert_eq!(card.phase, Phase::Downloading(0.4));
        card.step(InstallStep::Verifying);
        assert_eq!(card.phase, Phase::Downloading(1.0));
        card.step(InstallStep::Installing);
        card.exited(true);
        assert_eq!(card.phase, Phase::Installing, "the setup took over");
        card.stalled();
        assert_eq!(card.phase, Phase::Failed(NO_RESTART.into()));
        assert!(!card.busy());
    }

    #[test]
    fn failures_say_why_and_can_be_tried_again() {
        let mut card = card();
        card.start();
        card.step(InstallStep::Error("Not found on github.com".into()));
        card.exited(false);
        assert_eq!(card.phase, Phase::Failed("Not found on github.com".into()), "the reason printed is kept");
        card.step(InstallStep::Progress(0.9));
        assert_eq!(card.phase, Phase::Failed("Not found on github.com".into()));

        card.start();
        card.step(InstallStep::Progress(0.2));
        card.exited(true);
        assert_eq!(card.phase, Phase::Failed(STOPPED.into()), "ended mid-download");
        card.start();
        card.exited(false);
        assert_eq!(card.phase, Phase::Failed(STOPPED.into()), "crashed without a word");
    }
}
