//! The OSD hand-off (S8): when a Bluetooth audio device connects with a card, it usually
//! becomes the default output a moment later, and the OSD would show its own "Headphones"
//! moment on top. The Devices module claims that window here when it shows (or merges) a
//! card for an audio device; the OSD skips its device moment while a claim is live. Both
//! modules run on the hub thread, so a plain shared value is enough. If the default device
//! changes before the Bluetooth watcher reports the connect (rare: the audio endpoint comes
//! up after the link), both moments show.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long after the card a default-device change still counts as this connect (the PRD
/// says ~3 s; A2DP endpoints often take a little longer to appear).
pub const WINDOW: Duration = Duration::from_secs(4);

static CLAIMED_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);

/// The Devices module is showing a card for an audio device that just connected.
pub fn claim_audio_connect() {
    *CLAIMED_UNTIL.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now() + WINDOW);
}

/// Whether the OSD should leave the device moment to the Devices card.
pub fn audio_connect_claimed() -> bool {
    live(*CLAIMED_UNTIL.lock().unwrap_or_else(|e| e.into_inner()), Instant::now())
}

fn live(until: Option<Instant>, now: Instant) -> bool {
    until.is_some_and(|until| now < until)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_handoff_window() {
        let t = Instant::now();
        assert!(!live(None, t));
        assert!(live(Some(t + WINDOW), t + Duration::from_secs(3)));
        assert!(!live(Some(t + WINDOW), t + WINDOW));
    }
}
