//! `reui-bridge`: the program coding agents run as a hook.
//!
//! `reui-bridge claude` reads one hook event from stdin and forwards it to the hub. Only
//! permission requests wait for an answer; everything else is fire-and-forget (and is
//! installed with `async: true`, so the agent never waits at all). Contract with the agent:
//! stdout is either empty or exactly one decision JSON object, and the exit code is always 0,
//! so a missing hub, a timeout or any error means "no decision" and the agent carries on.
//!
//! `reui-bridge install claude [--scope user|project|local] [--project DIR]` and
//! `reui-bridge uninstall claude [...]` add or remove the hook entries.

mod install;

use std::io::{Read, Write};
use std::time::Duration;

use reui_protocol::{pipe_name, read_frame, write_frame, ClientMsg, HubMsg, SurfaceKind, PROTOCOL_VERSION};
use tokio::net::windows::named_pipe::ClientOptions;
use tokio::time::timeout;

const MAX_PAYLOAD: u64 = 4 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
/// Longer than the hub's own decision timeout, shorter than the hook timeout we install.
const REPLY_TIMEOUT: Duration = Duration::from_secs(110);
const ERROR_PIPE_BUSY: i32 = 231;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("install") | Some("uninstall") => match install::run(&args) {
            Ok(message) => {
                println!("{message}");
                0
            }
            Err(e) => {
                eprintln!("reui-bridge: {e}");
                1
            }
        },
        Some(agent) => {
            hook(agent);
            0
        }
        None => {
            eprintln!("usage: reui-bridge <agent>  |  reui-bridge install|uninstall <agent> [--scope user|project|local] [--project DIR]");
            0
        }
    };
    std::process::exit(code);
}

/// Forward one hook event. Never fails: every problem degrades to "no decision".
fn hook(agent: &str) {
    let mut payload = String::new();
    if std::io::stdin().take(MAX_PAYLOAD).read_to_string(&mut payload).is_err() {
        return;
    }
    let event = serde_json::from_str::<serde_json::Value>(&payload)
        .ok()
        .and_then(|v| v.get("hook_event_name").and_then(|e| e.as_str()).map(str::to_owned))
        .unwrap_or_default();
    let wants_reply = event == "PermissionRequest";

    let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
    let output = runtime.block_on(forward(agent, payload, wants_reply));
    if let Some(output) = output {
        let mut stdout = std::io::stdout().lock();
        _ = stdout.write_all(output.as_bytes());
        _ = stdout.flush();
    }
}

async fn forward(agent: &str, payload: String, wants_reply: bool) -> Option<String> {
    let pipe = timeout(CONNECT_TIMEOUT, connect()).await.ok()??;
    let (mut reader, mut writer) = tokio::io::split(pipe);
    write_frame(&mut writer, &ClientMsg::Hello { version: PROTOCOL_VERSION, surface: SurfaceKind::Bridge }).await.ok()?;
    match timeout(CONNECT_TIMEOUT, read_frame::<_, HubMsg>(&mut reader)).await.ok()?.ok()? {
        HubMsg::Welcome { .. } => {}
        _ => return None,
    }
    write_frame(&mut writer, &ClientMsg::Hook { agent: agent.to_string(), payload, wants_reply }).await.ok()?;
    if !wants_reply {
        return None;
    }
    match timeout(REPLY_TIMEOUT, read_frame::<_, HubMsg>(&mut reader)).await.ok()?.ok()? {
        HubMsg::HookReply { output } => output,
        _ => None,
    }
}

/// Opens the hub's pipe; retries briefly while all instances are busy, gives up at once if
/// the hub isn't running.
async fn connect() -> Option<tokio::net::windows::named_pipe::NamedPipeClient> {
    let name = pipe_name();
    loop {
        match ClientOptions::new().open(&name) {
            Ok(pipe) => return Some(pipe),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => tokio::time::sleep(Duration::from_millis(20)).await,
            Err(_) => return None,
        }
    }
}
