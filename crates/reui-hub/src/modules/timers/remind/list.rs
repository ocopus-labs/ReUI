//! Reminders as plain state on the wall clock (Unix ms the caller passes in): a reminder is
//! an event at a time of day, so a clock or time-zone change moves it, unlike a countdown.
//! Firing, snoozing and Done are pure, so every rule is tested without waiting.

use super::when::Repeat;

/// At most this many reminders at once.
pub const MAX_REMINDERS: usize = 50;
/// A reminder noticed this late (sleep, ReUI closed) says "Was due at …" and makes no sound.
pub const LATE_MS: u64 = 60_000;
/// Snooze puts it off this long.
pub const SNOOZE_MS: u64 = 10 * 60_000;

#[derive(Debug, Clone, PartialEq)]
pub struct Reminder {
    pub id: u32,
    pub text: Option<String>,
    /// When it fires (or fired, while ringing).
    pub due_unix_ms: u64,
    pub repeat: Option<Repeat>,
    /// It fired and its card waits for Done or Snooze.
    pub ringing: bool,
    /// It fired more than [`LATE_MS`] after its time.
    pub late: bool,
}

#[derive(Debug, Default)]
pub struct Reminders {
    list: Vec<Reminder>,
    next_id: u32,
}

impl Reminders {
    /// Keeps a reminder; `None` when [`MAX_REMINDERS`] are already kept.
    pub fn add(&mut self, text: Option<String>, due_unix_ms: u64, repeat: Option<Repeat>) -> Option<u32> {
        self.restore(Reminder { id: 0, text, due_unix_ms, repeat, ringing: false, late: false })
    }

    /// Puts back a saved reminder as it was (ringing or not), with a fresh id.
    pub fn restore(&mut self, reminder: Reminder) -> Option<u32> {
        if self.list.len() >= MAX_REMINDERS {
            return None;
        }
        self.next_id += 1;
        let id = self.next_id;
        self.list.push(Reminder { id, ..reminder });
        Some(id)
    }

    pub fn all(&self) -> &[Reminder] {
        &self.list
    }

    /// When the next one fires.
    pub fn next_due(&self) -> Option<u64> {
        self.list.iter().filter(|r| !r.ringing).map(|r| r.due_unix_ms).min()
    }

    /// Fires every reminder whose time has come; returns how many fired, and how many of
    /// those on time (the ones worth a sound).
    pub fn fire(&mut self, now_unix_ms: u64) -> (usize, usize) {
        let (mut fired, mut on_time) = (0, 0);
        for r in self.list.iter_mut().filter(|r| !r.ringing && r.due_unix_ms <= now_unix_ms) {
            r.ringing = true;
            r.late = now_unix_ms - r.due_unix_ms >= LATE_MS;
            fired += 1;
            on_time += usize::from(!r.late);
        }
        (fired, on_time)
    }

    /// The ones waiting for an answer, the oldest first.
    pub fn ringing(&self) -> Vec<&Reminder> {
        let mut out: Vec<&Reminder> = self.list.iter().filter(|r| r.ringing).collect();
        out.sort_by_key(|r| (r.due_unix_ms, r.id));
        out
    }

    /// The ones still to come, the soonest first.
    pub fn upcoming(&self) -> Vec<&Reminder> {
        let mut out: Vec<&Reminder> = self.list.iter().filter(|r| !r.ringing).collect();
        out.sort_by_key(|r| (r.due_unix_ms, r.id));
        out
    }

    pub fn get(&self, id: u32) -> Option<&Reminder> {
        self.list.iter().find(|r| r.id == id)
    }

    /// Done: a one-off goes; a repeat waits for `next` (its next time after now, which the
    /// caller works out on the local clock), so one left unanswered never piles up (E-R3).
    pub fn done(&mut self, id: u32, next: Option<u64>) {
        let Some(at) = self.list.iter().position(|r| r.id == id) else { return };
        match (self.list[at].repeat, next) {
            (Some(_), Some(next)) => {
                let r = &mut self.list[at];
                (r.due_unix_ms, r.ringing, r.late) = (next, false, false);
            }
            _ => _ = self.list.remove(at),
        }
    }

    /// Fires again [`SNOOZE_MS`] from now; a repeat carries on from there.
    pub fn snooze(&mut self, id: u32, now_unix_ms: u64) {
        if let Some(r) = self.list.iter_mut().find(|r| r.id == id) {
            (r.due_unix_ms, r.ringing, r.late) = (now_unix_ms + SNOOZE_MS, false, false);
        }
    }

    pub fn remove(&mut self, id: u32) -> Option<Reminder> {
        let at = self.list.iter().position(|r| r.id == id)?;
        Some(self.list.remove(at))
    }
}

#[cfg(test)]
mod tests {
    use super::super::when::Every;
    use super::*;

    const MIN: u64 = 60_000;

    #[test]
    fn fires_on_time_or_late_and_snoozes() {
        let mut r = Reminders::default();
        let call = r.add(Some("Call".into()), 10 * MIN, None).unwrap();
        let bins = r.add(Some("Bins".into()), 20 * MIN, None).unwrap();
        assert_eq!(r.next_due(), Some(10 * MIN));
        assert_eq!(r.fire(10 * MIN - 1), (0, 0));
        assert_eq!(r.fire(10 * MIN + 500), (1, 1));
        assert_eq!(r.next_due(), Some(20 * MIN), "a ringing one isn't due again");
        // Asleep through the second: it fires late, quietly.
        assert_eq!(r.fire(40 * MIN), (1, 0));
        assert!(r.get(bins).unwrap().late);
        assert_eq!(r.ringing().iter().map(|x| x.id).collect::<Vec<_>>(), [call, bins]);
        r.snooze(call, 40 * MIN);
        assert_eq!((r.next_due(), r.get(call).unwrap().ringing), (Some(50 * MIN), false));
        r.done(bins, None);
        assert!(r.get(bins).is_none());
        assert_eq!(r.upcoming().len(), 1);
    }

    #[test]
    fn a_repeat_moves_on_instead_of_going() {
        let mut r = Reminders::default();
        let repeat = Repeat { every: Every::Day, hour: 21, minute: 0 };
        let meds = r.add(Some("Meds".into()), 10 * MIN, Some(repeat)).unwrap();
        r.fire(10 * MIN);
        r.done(meds, Some(10 * MIN + 24 * 60 * MIN));
        let kept = r.get(meds).unwrap();
        assert_eq!((kept.ringing, kept.due_unix_ms), (false, 10 * MIN + 24 * 60 * MIN));
        // Fired once more and left: still one card, however long it waits.
        r.fire(u64::MAX / 2);
        r.fire(u64::MAX / 2 + 1);
        assert_eq!(r.ringing().len(), 1);
    }

    #[test]
    fn keeps_at_most_fifty() {
        let mut r = Reminders::default();
        for i in 0..MAX_REMINDERS as u64 {
            assert!(r.add(None, i, None).is_some());
        }
        assert!(r.add(None, 0, None).is_none());
        let first = r.all()[0].id;
        assert!(r.remove(first).is_some());
        assert!(r.add(None, 0, None).is_some());
    }
}
