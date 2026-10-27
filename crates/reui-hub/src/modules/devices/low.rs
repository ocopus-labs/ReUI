//! Low-battery thresholds (PRD S2/S4): a moment at 20 % and at 10 %, each once per
//! discharge. A threshold re-arms once the level is 5 % above it again, or when the device
//! charges; nothing fires while charging. The first level seen only sets the baseline (a
//! device that connects at 15 % shows it on its connect card, in red).

pub const THRESHOLDS: [u8; 2] = [20, 10];
/// The level at or below which a low-battery moment is critical (shown even in "only when it
/// needs me" mode, D8).
pub const CRITICAL: u8 = 10;
const REARM: u8 = 5;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LowBattery {
    /// Per threshold in [`THRESHOLDS`]; `None` until the first level.
    armed: Option<[bool; 2]>,
}

impl LowBattery {
    /// A new level; returns the threshold crossed, if one is due.
    pub fn update(&mut self, level: u8, charging: bool) -> Option<u8> {
        let Some(armed) = &mut self.armed else {
            self.armed = Some(THRESHOLDS.map(|t| charging || level > t));
            return None;
        };
        if charging {
            *armed = [true; 2];
            return None;
        }
        for (i, t) in THRESHOLDS.iter().enumerate() {
            if level >= t + REARM {
                armed[i] = true;
            }
        }
        // The lowest threshold reached wins and also spends the ones above it.
        let hit = THRESHOLDS.iter().enumerate().rev().find(|(i, t)| armed[*i] && level <= **t).map(|(i, t)| (i, *t))?;
        for (j, slot) in armed.iter_mut().enumerate() {
            if j <= hit.0 {
                *slot = false;
            }
        }
        Some(hit.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_low_battery_once_per_discharge() {
        let mut b = LowBattery::default();
        assert_eq!(b.update(50, false), None);
        assert_eq!(b.update(21, false), None);
        assert_eq!(b.update(20, false), Some(20));
        assert_eq!(b.update(19, false), None);
        assert_eq!(b.update(18, false), None);
        assert_eq!(b.update(10, false), Some(10));
        assert_eq!(b.update(9, false), None);
        assert_eq!(b.update(0, false), None);
    }

    #[test]
    fn devices_low_battery_rearms_and_skips_charging() {
        let mut b = LowBattery::default();
        b.update(40, false);
        assert_eq!(b.update(19, false), Some(20));
        // Recovering a little isn't enough; 5 % above is.
        assert_eq!(b.update(22, false), None);
        assert_eq!(b.update(20, false), None);
        assert_eq!(b.update(25, false), None);
        assert_eq!(b.update(20, false), Some(20));
        // Charging: never, and re-armed afterwards.
        assert_eq!(b.update(8, true), None);
        assert_eq!(b.update(9, true), None);
        assert_eq!(b.update(9, false), Some(10));
    }

    #[test]
    fn devices_low_battery_coarse_steps_and_baseline() {
        // A controller's 10/40/70/100 steps: only the 10 step alerts, as the 10 % moment.
        let mut c = LowBattery::default();
        c.update(70, false);
        assert_eq!(c.update(40, false), None);
        assert_eq!(c.update(10, false), Some(10));
        assert_eq!(c.update(10, false), None);
        // A device first seen at 15 % only sets the baseline.
        let mut d = LowBattery::default();
        assert_eq!(d.update(15, false), None);
        assert_eq!(d.update(14, false), None);
        assert_eq!(d.update(10, false), Some(10));
    }
}
