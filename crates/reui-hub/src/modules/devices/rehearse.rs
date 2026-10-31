//! A rehearsed connect, for recording the app without a pair of earbuds to hand: in a sandbox
//! (`REUI_CONFIG_DIR`) with `REUI_DEMO_DEVICE` set, creating the file `demo-device` in the
//! local folder shows the connect card of a pair of AirPods Pro, drawn exactly as a real
//! connect is (`view::connect_view`, the earbuds' drawing, left / right / case rows). Nothing
//! is read from Bluetooth for it. Off unless both variables are set; then the module looks
//! for the file four times a second, which only a rehearsal pays for.

use std::time::Duration;

use reui_protocol::Priority;

use super::airpods::Pods;
use super::kinds::Kind;
use super::{view, State};
use crate::hub::ModuleContext;

pub const POLL: Duration = Duration::from_millis(250);
const KEY: &str = "rehearsed-device";
const LINGER: Duration = Duration::from_secs(5);

pub fn enabled() -> bool {
    let set = |name| std::env::var_os(name).is_some_and(|v| !v.is_empty());
    set("REUI_CONFIG_DIR") && set("REUI_DEMO_DEVICE")
}

/// Whether the trigger file was there (and takes it away, so one file is one connect).
pub async fn due() -> bool {
    let path = crate::paths::local().join("demo-device");
    tokio::task::spawn_blocking(move || std::fs::remove_file(path).is_ok()).await.unwrap_or(false)
}

/// What a real pair would report a moment after connecting.
fn device() -> view::DeviceInfo {
    let pods = Pods { model: 0x2014, left: Some(92), right: Some(90), case: Some(64), left_charging: false, right_charging: false, case_charging: false, lid_open: None, colour: None };
    view::DeviceInfo { name: "AirPods Pro".into(), kind: Kind::Earbuds, model: Some("AirPods Pro (2nd generation)"), level: pods.buds(), pods: Some(pods) }
}

impl State {
    pub(super) fn rehearse_connect(&mut self, ctx: &ModuleContext) {
        let d = device();
        let mark = self.mark(ctx, d.kind);
        self.show(ctx, KEY, Priority::Brief, view::connect_view(&d, &mark), LINGER);
    }
}
