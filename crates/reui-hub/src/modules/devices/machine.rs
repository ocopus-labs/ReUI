//! The machine's own events as the module handles them: the shared window's power source,
//! battery %, resume / unlock, volume and phone arrivals; battery reports; drives and Eject.

use std::time::Duration;

use reui_protocol::Priority;
use tokio::time::Instant;

use super::drives::{self, Drive};
use super::eject::{self, Refused};
use super::kinds::{battery_glyph, GLYPH_BOLT, GLYPH_PHONE};
use super::power::{self, PowerEvent};
use super::spam::{COOLDOWN, MERGE};
use super::{view, State, DriveCard, Internal};
use crate::hub::ModuleContext;
use crate::modules::system_events::{PowerSource, SystemEvent};

const POWER: &str = "power";
const CHARGING: Duration = Duration::from_secs(4);
const STATE: Duration = Duration::from_secs(3);
const DRIVE: Duration = Duration::from_secs(5);
/// A refused eject stays readable this long (longer while its card is open).
const REFUSED: Duration = Duration::from_secs(6);
/// After a volume arrives, Windows assigns its letter a moment later: look a few times.
const RESCANS: [Duration; 3] = [Duration::from_millis(300), Duration::from_millis(1500), Duration::from_secs(4)];

impl State {
    pub(super) fn on_system(&mut self, ctx: &ModuleContext, event: SystemEvent) {
        let now = Instant::now();
        match event {
            SystemEvent::Resumed | SystemEvent::Unlocked => self.gate.quiet(now),
            SystemEvent::PowerSource { source, initial } => {
                let before = self.source.replace(source);
                if initial || self.has_battery != Some(true) || before == Some(source) {
                    return;
                }
                match source {
                    PowerSource::Ac => {
                        if let Some(p) = self.percent {
                            self.laptop_low.update(p.min(100) as u8, true);
                        }
                        if let Some(PowerEvent::Report { rate_mw, remaining_mwh, full_mwh, charging }) = self.power.as_ref().and_then(|w| w.read()) {
                            self.eta.push(now, rate_mw, remaining_mwh, full_mwh, charging);
                        }
                        if self.settings.charging {
                            if let Some(v) = self.charging_view() {
                                self.show(ctx, POWER, Priority::Brief, v, CHARGING);
                            }
                        }
                    }
                    // Unplugging is quiet (D3).
                    PowerSource::Battery => {
                        if self.moments.remove(POWER) {
                            ctx.retract(POWER);
                        }
                    }
                    // D-9: a quiet moment.
                    PowerSource::Ups => {
                        let glyph = battery_glyph(self.percent.unwrap_or(100));
                        self.show(ctx, "ups", Priority::Brief, view::state_view(glyph, "On UPS power"), STATE);
                    }
                }
                self.refresh_overview(ctx);
            }
            SystemEvent::BatteryPercent { percent, .. } => {
                self.percent = Some(percent);
                if self.has_battery == Some(false) {
                    return;
                }
                let level = percent.min(100) as u8;
                // The first value is the baseline (startup at 15 % isn't news).
                if let Some(threshold) = self.laptop_low.update(level, self.source == Some(PowerSource::Ac)) {
                    self.show_low(ctx, "low:laptop", battery_glyph(percent), level, threshold);
                }
                self.refresh_power(ctx);
                self.refresh_overview(ctx);
            }
            SystemEvent::Volume => {
                if self.settings.drives || self.moments.is_live(super::OVERVIEW) {
                    self.rescans.extend(RESCANS.iter().map(|d| now + *d));
                }
            }
            SystemEvent::PortableDevice { arrived: true, apple, id } => {
                let recent = self.phones_shown.get(&id).is_some_and(|at| now < *at + COOLDOWN);
                if self.settings.phones && !recent && !self.gate.in_grace(now) {
                    self.phones_shown.insert(id, now);
                    let text = if apple { "iPhone connected" } else { "Phone connected" };
                    self.show(ctx, "phone", Priority::Brief, view::state_view(GLYPH_PHONE, text), STATE);
                }
            }
            SystemEvent::PortableDevice { arrived: false, .. } => {}
            // The OSD's and the timers'.
            SystemEvent::TimeChanged | SystemEvent::Brightness { .. } | SystemEvent::BrightnessKey | SystemEvent::BatterySaver { .. } | SystemEvent::Lock { .. } | SystemEvent::TaskbarCreated | SystemEvent::DisplaysChanged => {}
        }
    }

    pub(super) fn on_power(&mut self, ctx: &ModuleContext, event: PowerEvent) {
        match event {
            PowerEvent::Report { rate_mw, remaining_mwh, full_mwh, charging } => {
                self.eta.push(Instant::now(), rate_mw, remaining_mwh, full_mwh, charging);
                self.refresh_power(ctx);
            }
            PowerEvent::SlowCharger(true) if self.settings.slow_charger && self.has_battery == Some(true) => {
                self.show(ctx, "slow-charger", Priority::Brief, view::state_view(GLYPH_BOLT, "Charging slowly"), STATE);
            }
            PowerEvent::SlowCharger(_) => {}
        }
    }

    fn charging_view(&mut self) -> Option<crate::hub::View> {
        let percent = self.percent?;
        let eta = self.eta.estimate(Instant::now(), percent).map(power::full_in);
        Some(view::charging_view(percent.min(100) as u8, eta))
    }

    /// The charging card follows the level and the time to full while it shows.
    fn refresh_power(&mut self, ctx: &ModuleContext) {
        if self.moments.is_live(POWER) {
            if let Some(v) = self.charging_view() {
                ctx.publish(POWER, Priority::Brief, v);
            }
        }
    }

    // ---- drives ------------------------------------------------------------------------

    pub(super) fn rescan_drives(&self, after: Duration) {
        let tx = self.internal.clone();
        tokio::spawn(async move {
            tokio::time::sleep(after).await;
            if let Ok(list) = tokio::task::spawn_blocking(drives::scan).await {
                _ = tx.send(Internal::Drives(list));
            }
        });
    }

    pub(super) fn on_drives(&mut self, ctx: &ModuleContext, list: Vec<Drive>) {
        let now = Instant::now();
        // The first scan is the baseline: drives plugged in before the hub aren't news.
        let Some(known) = self.drives.replace(list.clone()) else { return };
        let gone: Vec<char> = known.iter().filter(|k| !list.iter().any(|d| d.letter == k.letter)).map(|d| d.letter).collect();
        let arrived: Vec<char> = list.iter().filter(|d| !known.iter().any(|k| k.letter == d.letter)).map(|d| d.letter).collect();
        for letter in &gone {
            // Pulled without Eject: silent (D-12).
            self.refused.remove(letter);
        }
        if let Some(card) = &mut self.drive_card {
            card.letters.retain(|l| !gone.contains(l));
        }
        if !arrived.is_empty() && self.settings.drives {
            match &mut self.drive_card {
                // Docks: arrivals within 2 s share the card.
                Some(card) if now < card.since + MERGE && self.moments.is_live(&card.key) => card.letters.extend(arrived),
                _ => self.drive_card = Some(DriveCard { key: format!("drive:{}", arrived[0]), letters: arrived, since: now }),
            }
            self.publish_drives(ctx, DRIVE);
        } else if !gone.is_empty() {
            self.publish_drives(ctx, Duration::ZERO);
        }
        self.refresh_overview(ctx);
    }

    /// The drive card as it is now; retracted once its drives are all gone.
    fn publish_drives(&mut self, ctx: &ModuleContext, linger: Duration) {
        let Some(card) = &self.drive_card else { return };
        let drives = self.drives.as_deref().unwrap_or_default();
        let list: Vec<(Drive, Option<String>)> = card.letters.iter().filter_map(|l| drives.iter().find(|d| d.letter == *l)).map(|d| (d.clone(), self.refused.get(&d.letter).cloned())).collect();
        let key = card.key.clone();
        match view::drive_view(&list) {
            Some(v) => self.show(ctx, &key, Priority::Brief, v, linger),
            None => {
                if self.moments.remove(&key) {
                    ctx.retract(&key);
                }
                self.drive_card = None;
            }
        }
    }

    pub(super) fn eject(&mut self, letter: char) {
        let letter = letter.to_ascii_uppercase();
        if !self.drives.as_ref().is_some_and(|d| d.iter().any(|d| d.letter == letter)) {
            return;
        }
        let tx = self.internal.clone();
        tokio::spawn(async move {
            if let Ok(result) = tokio::task::spawn_blocking(move || eject::eject(letter)).await {
                _ = tx.send(Internal::Ejected(letter, result));
            }
        });
    }

    pub(super) fn on_ejected(&mut self, ctx: &ModuleContext, letter: char, result: Result<(), Refused>) {
        match result {
            Ok(()) => {
                self.refused.remove(&letter);
                if let Some(list) = &mut self.drives {
                    list.retain(|d| d.letter != letter);
                }
                if let Some(card) = &mut self.drive_card {
                    card.letters.retain(|l| *l != letter);
                }
                self.publish_drives(ctx, Duration::ZERO);
            }
            Err(refused) => {
                eprintln!("devices: eject refused ({})", refused.text());
                self.refused.insert(letter, refused.text());
                self.publish_drives(ctx, REFUSED);
            }
        }
        self.refresh_overview(ctx);
    }
}
