//! The laptop's battery: whether there is one (`GetSystemPowerStatus`; desktops have the
//! feature off, S10), `AggregateBattery` reports for the charge rate and capacity, Windows'
//! slow-charger state, and the time to full from them, smoothed over two minutes and only
//! shown once it's stable and below 95 % (D-8).

use std::collections::VecDeque;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;
use tokio::time::Instant;
use windows::core::{IInspectable, Ref};
use windows::Devices::Power::Battery;
use windows::Foundation::{EventHandler, TypedEventHandler};
use windows::System::Power::{BatteryStatus, PowerManager, PowerSupplyStatus};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

/// Whether this machine has a battery (`BatteryFlag` 128 = no system battery, 255 unknown).
pub fn has_battery() -> bool {
    let mut status = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut status) }.is_ok() && status.BatteryFlag != 128 && status.BatteryFlag != 255
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PowerEvent {
    Report { rate_mw: Option<i32>, remaining_mwh: Option<i32>, full_mwh: Option<i32>, charging: bool },
    /// Windows says the charger can't keep up.
    SlowCharger(bool),
}

/// The subscriptions; dropping them stops the events.
pub struct PowerWatch {
    battery: Battery,
    tokens: [i64; 2],
}

impl PowerWatch {
    pub fn start(tx: UnboundedSender<PowerEvent>) -> windows::core::Result<Self> {
        let battery = Battery::AggregateBattery()?;
        let t = tx.clone();
        let report = battery.ReportUpdated(&TypedEventHandler::new(move |battery: Ref<Battery>, _: Ref<IInspectable>| {
            if let Some(event) = battery.as_ref().and_then(read) {
                _ = t.send(event);
            }
            Ok(())
        }))?;
        let supply = PowerManager::PowerSupplyStatusChanged(&EventHandler::new(move |_: Ref<IInspectable>, _: Ref<IInspectable>| {
            _ = tx.send(PowerEvent::SlowCharger(PowerManager::PowerSupplyStatus()? == PowerSupplyStatus::Inadequate));
            Ok(())
        }));
        let supply = match supply {
            Ok(token) => token,
            Err(e) => {
                _ = battery.RemoveReportUpdated(report);
                return Err(e);
            }
        };
        Ok(Self { battery, tokens: [report, supply] })
    }

    /// The current report, for a sample when nothing has changed for a while.
    pub fn read(&self) -> Option<PowerEvent> {
        read(&self.battery)
    }
}

impl Drop for PowerWatch {
    fn drop(&mut self) {
        _ = self.battery.RemoveReportUpdated(self.tokens[0]);
        _ = PowerManager::RemovePowerSupplyStatusChanged(self.tokens[1]);
    }
}

fn read(battery: &Battery) -> Option<PowerEvent> {
    let report = battery.GetReport().ok()?;
    let value = |r: windows::core::Result<windows::Foundation::IReference<i32>>| r.ok().and_then(|v| v.Value().ok());
    Some(PowerEvent::Report {
        rate_mw: value(report.ChargeRateInMilliwatts()),
        remaining_mwh: value(report.RemainingCapacityInMilliwattHours()),
        full_mwh: value(report.FullChargeCapacityInMilliwattHours()),
        charging: report.Status().is_ok_and(|s| s == BatteryStatus::Charging),
    })
}

/// Samples kept for the estimate.
const WINDOW: Duration = Duration::from_secs(120);
/// The samples must cover at least this before an estimate shows.
const MIN_SPAN: Duration = Duration::from_secs(60);
/// The charge rate may vary this much (max − min, relative to the mean) and still count as
/// stable.
const MAX_SPREAD: f64 = 0.25;
/// Above this the rate tapers and estimates jump: no time to full.
const HIDE_ABOVE: u32 = 95;

#[derive(Debug, Clone, Copy)]
struct Sample {
    at: Instant,
    rate_mw: f64,
    remaining_mwh: f64,
    full_mwh: f64,
}

/// Time to full from recent battery reports.
#[derive(Debug, Default)]
pub struct Eta {
    samples: VecDeque<Sample>,
}

impl Eta {
    /// A report while charging (anything else starts over).
    pub fn push(&mut self, now: Instant, rate_mw: Option<i32>, remaining_mwh: Option<i32>, full_mwh: Option<i32>, charging: bool) {
        match (charging, rate_mw, remaining_mwh, full_mwh) {
            (true, Some(rate), Some(remaining), Some(full)) if rate > 0 && full > 0 => {
                self.samples.push_back(Sample { at: now, rate_mw: f64::from(rate), remaining_mwh: f64::from(remaining), full_mwh: f64::from(full) });
                self.prune(now);
            }
            _ => self.samples.clear(),
        }
    }

    fn prune(&mut self, now: Instant) {
        while self.samples.front().is_some_and(|s| now.saturating_duration_since(s.at) > WINDOW) {
            self.samples.pop_front();
        }
    }

    /// The smoothed time to full, once stable; `None` above 95 %.
    pub fn estimate(&mut self, now: Instant, percent: u32) -> Option<Duration> {
        self.prune(now);
        if percent > HIDE_ABOVE || self.samples.len() < 2 {
            return None;
        }
        let (first, last) = (self.samples.front()?, self.samples.back()?);
        if last.at.saturating_duration_since(first.at) < MIN_SPAN {
            return None;
        }
        let rates = self.samples.iter().map(|s| s.rate_mw);
        let mean = rates.clone().sum::<f64>() / self.samples.len() as f64;
        let (min, max) = rates.fold((f64::MAX, f64::MIN), |(lo, hi), r| (lo.min(r), hi.max(r)));
        if mean <= 0.0 || (max - min) / mean > MAX_SPREAD {
            return None;
        }
        let missing = (last.full_mwh - last.remaining_mwh).max(0.0);
        Some(Duration::from_secs_f64(missing / mean * 3600.0))
    }
}

/// "Full in 1 h 10 min", "Full in 25 min", "Full in 2 h"; minutes rounded up.
pub fn full_in(eta: Duration) -> String {
    let minutes = eta.as_secs().div_ceil(60).max(1);
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("Full in {m} min"),
        (h, 0) => format!("Full in {h} h"),
        (h, m) => format!("Full in {h} h {m} min"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(base: Instant, secs: u64) -> Instant {
        base + Duration::from_secs(secs)
    }

    #[test]
    fn devices_time_to_full_needs_a_stable_rate() {
        let t = Instant::now();
        let mut eta = Eta::default();
        // 30 Wh missing at a steady 30 W: one hour, but only after a minute of samples.
        eta.push(t, Some(30_000), Some(20_000), Some(50_000), true);
        assert_eq!(eta.estimate(t, 40), None);
        eta.push(s(t, 30), Some(31_000), Some(20_250), Some(50_000), true);
        assert_eq!(eta.estimate(s(t, 30), 40), None);
        eta.push(s(t, 60), Some(29_000), Some(20_500), Some(50_000), true);
        let e = eta.estimate(s(t, 60), 41).unwrap();
        assert!((e.as_secs_f64() - 29_500.0 / 30_000.0 * 3600.0).abs() < 1.0);
        // Above 95 %: hidden.
        assert_eq!(eta.estimate(s(t, 60), 96), None);

        // A jumpy rate isn't stable.
        eta.push(s(t, 90), Some(10_000), Some(20_600), Some(50_000), true);
        assert_eq!(eta.estimate(s(t, 90), 41), None);
        // Old samples age out; the newest window decides again.
        eta.push(s(t, 200), Some(10_000), Some(21_000), Some(50_000), true);
        eta.push(s(t, 270), Some(10_500), Some(21_200), Some(50_000), true);
        assert!(eta.estimate(s(t, 270), 42).is_some());

        // Unplugged (or a report without a rate): start over.
        eta.push(s(t, 280), Some(-5_000), Some(21_200), Some(50_000), false);
        assert_eq!(eta.estimate(s(t, 280), 42), None);
    }

    #[test]
    fn devices_time_to_full_text() {
        assert_eq!(full_in(Duration::from_secs(70 * 60)), "Full in 1 h 10 min");
        assert_eq!(full_in(Duration::from_secs(25 * 60 - 20)), "Full in 25 min");
        assert_eq!(full_in(Duration::from_secs(2 * 3600)), "Full in 2 h");
        assert_eq!(full_in(Duration::from_secs(5)), "Full in 1 min");
    }
}
