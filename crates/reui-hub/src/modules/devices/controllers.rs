//! Game controllers through `Windows.Gaming.Input`: added / removed events, and the battery
//! from `TryGetBatteryReport` (Xbox controllers report coarse steps, D-7), polled by the
//! module every 60 s while one is connected. Wired controllers have no battery report.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc::UnboundedSender;
use windows::core::{IInspectable, Ref};
use windows::Foundation::EventHandler;
use windows::Gaming::Input::RawGameController;
use windows::System::Power::BatteryStatus;

use super::registry::hash;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    /// Per-session hash of the controller's id.
    pub key: u64,
    pub name: String,
    pub level: Option<u8>,
    pub charging: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PadEvent {
    Added(Reading),
    Removed(u64),
}

type Pads = Arc<Mutex<HashMap<u64, RawGameController>>>;

/// The subscription; dropping it stops the events.
pub struct Controllers {
    pads: Pads,
    tokens: [i64; 2],
}

impl Controllers {
    pub fn start(tx: UnboundedSender<PadEvent>) -> windows::core::Result<Self> {
        let pads: Pads = Arc::default();
        let (p, t) = (pads.clone(), tx.clone());
        let added = RawGameController::RawGameControllerAdded(&EventHandler::new(move |_: Ref<IInspectable>, pad: Ref<RawGameController>| {
            if let Some(pad) = pad.as_ref() {
                let reading = read(pad);
                p.lock().unwrap_or_else(|e| e.into_inner()).insert(reading.key, pad.clone());
                _ = t.send(PadEvent::Added(reading));
            }
            Ok(())
        }))?;
        let p = pads.clone();
        let removed = RawGameController::RawGameControllerRemoved(&EventHandler::new(move |_: Ref<IInspectable>, pad: Ref<RawGameController>| {
            if let Some(pad) = pad.as_ref() {
                let key = key_of(pad);
                p.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
                _ = tx.send(PadEvent::Removed(key));
            }
            Ok(())
        }));
        let removed = match removed {
            Ok(token) => token,
            Err(e) => {
                _ = RawGameController::RemoveRawGameControllerAdded(added);
                return Err(e);
            }
        };
        Ok(Self { pads, tokens: [added, removed] })
    }

    pub fn shared(&self) -> Arc<Mutex<HashMap<u64, RawGameController>>> {
        self.pads.clone()
    }
}

pub fn poll_shared(pads: &Mutex<HashMap<u64, RawGameController>>) -> Vec<Reading> {
    pads.lock().unwrap_or_else(|e| e.into_inner()).values().map(read).collect()
}

impl Drop for Controllers {
    fn drop(&mut self) {
        _ = RawGameController::RemoveRawGameControllerAdded(self.tokens[0]);
        _ = RawGameController::RemoveRawGameControllerRemoved(self.tokens[1]);
    }
}

fn key_of(pad: &RawGameController) -> u64 {
    hash(pad.NonRoamableId().map(|id| id.to_string()).unwrap_or_default())
}

/// Remaining / full, as a percentage; `None` without both.
pub fn level(remaining: Option<i32>, full: Option<i32>) -> Option<u8> {
    let (remaining, full) = (remaining?, full?);
    (full > 0 && remaining >= 0).then(|| ((f64::from(remaining) / f64::from(full)) * 100.0).round().min(100.0) as u8)
}

fn read(pad: &RawGameController) -> Reading {
    let report = pad.TryGetBatteryReport().ok();
    let status = report.as_ref().and_then(|r| r.Status().ok());
    let value = |r: windows::core::Result<windows::Foundation::IReference<i32>>| r.ok().and_then(|v| v.Value().ok());
    let level = report.as_ref().filter(|_| status != Some(BatteryStatus::NotPresent)).and_then(|r| level(value(r.RemainingCapacityInMilliwattHours()), value(r.FullChargeCapacityInMilliwattHours())));
    Reading { key: key_of(pad), name: pad.DisplayName().map(|n| n.to_string()).unwrap_or_default(), level, charging: status == Some(BatteryStatus::Charging) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_controller_levels() {
        assert_eq!(level(Some(40), Some(100)), Some(40));
        assert_eq!(level(Some(1), Some(3)), Some(33));
        assert_eq!(level(Some(120), Some(100)), Some(100));
        assert_eq!(level(None, Some(100)), None);
        assert_eq!(level(Some(10), Some(0)), None);
    }
}
