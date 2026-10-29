//! `reui-bridge`: the program coding agents run as a hook.
//!
//! `reui-bridge claude` reads one hook event from stdin and forwards it to the hub. Only
//! what needs you waits for an answer: permission requests, MCP elicitation, OpenCode's
//! questions (`Question`, from our plugin), and, run as
//! `reui-bridge claude ask` (the blocking `PreToolUse` entry), Claude's AskUserQuestion and
//! ExitPlanMode. Everything else is fire-and-forget (and is installed with `async: true`,
//! so the agent never waits at all); the async `PreToolUse` for those two tools forwards
//! without waiting, and the hub makes the pair one moment. Contract with the agent:
//! stdout is either empty or exactly one decision JSON object, and the exit code is always 0,
//! so a missing hub, a timeout or any error means "no decision" and the agent carries on.
//! Each event goes with the bridge's parent process chain (`reui_parents`, `parents.rs`),
//! so the island's "Open terminal" can find the window the agent runs in.
//!
//! `reui-bridge install claude [--scope user|project|local] [--project DIR]` and
//! `reui-bridge uninstall claude [...]` add or remove the hook entries.
//!
//! `reui-bridge test claude` sends a synthetic event through the same pipe a hook uses
//! (Settings' "Send test event"); the agent module answers it with a short "connected"
//! moment in the island, never a session. Exits 1 if ReUI isn't running.
//!
//! `reui-bridge timer <length and label>` starts a timer (`10m tea`, `1h30`, `focus`,
//! `stopwatch`), the same text the island's timer field takes, for scripts and agents; with
//! nothing after it, it opens that field. `reui-bridge stats` opens the system stats card.
//! Both exit 1 if ReUI isn't running.

mod install;
mod parents;

use std::io::{Read, Write};
use std::time::Duration;

use reui_protocol::{pipe_name, read_frame, write_frame, ClientMsg, HubMsg, SurfaceKind, PROTOCOL_VERSION};
use tokio::net::windows::named_pipe::ClientOptions;
use tokio::time::timeout;

const MAX_PAYLOAD: u64 = 4 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
/// How long to wait for the hub's answer, per kind: longer than the hub's own wait, shorter
/// than the hook timeout we install, so the agent always gets a clean "no decision".
/// Permissions: hub 100 s < 110 s < hook 120 s.
const PERMISSION_WAIT: Duration = Duration::from_secs(110);
/// Questions, plans and MCP forms: hub 280 s < 290 s < hook 300 s.
const ASK_WAIT: Duration = Duration::from_secs(290);
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
        Some("test") => match test(args.get(1).map(String::as_str)) {
            Ok(message) => {
                println!("{message}");
                0
            }
            Err(e) => {
                eprintln!("reui-bridge: {e}");
                1
            }
        },
        Some("stats") => match command("stats", "open") {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("reui-bridge: {e}");
                1
            }
        },
        Some("timer") => match timer(&args[1..].join(" ")) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("reui-bridge: {e}");
                1
            }
        },
        Some(agent) => {
            hook(agent, args.get(1).is_some_and(|a| a == install::ASK));
            0
        }
        None => {
            eprintln!("usage: reui-bridge <agent>  |  reui-bridge install|uninstall <agent> [--scope user|project|local] [--project DIR]  |  reui-bridge test <agent>  |  reui-bridge timer <length> [label]  |  reui-bridge stats");
            0
        }
    };
    std::process::exit(code);
}

/// Forward one hook event. Never fails: every problem degrades to "no decision".
fn hook(agent: &str, ask: bool) {
    let mut payload = String::new();
    if std::io::stdin().take(MAX_PAYLOAD).read_to_string(&mut payload).is_err() {
        return;
    }
    let mut event = serde_json::from_str::<serde_json::Value>(&payload).ok();
    let wait = event.as_ref().and_then(|event| reply_wait(event, ask));
    // Where the agent runs, for the island's "Open terminal" (`parents.rs`).
    if let Some(serde_json::Value::Object(fields)) = event.as_mut() {
        fields.insert("reui_parents".into(), parents::own().into());
        fields.insert("reui_console".into(), parents::console().into());
        payload = serde_json::Value::Object(std::mem::take(fields)).to_string();
    }

    let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
    let output = runtime.block_on(forward(agent, payload, wait));
    if let Some(output) = output {
        let mut stdout = std::io::stdout().lock();
        _ = stdout.write_all(output.as_bytes());
        _ = stdout.flush();
    }
}

/// Whether this event waits for an answer, and how long. `ask`: run from the blocking
/// `PreToolUse` entry, which only Claude's AskUserQuestion and ExitPlanMode match (the
/// tool is checked too, so a hand-edited matcher can't make every tool wait).
fn reply_wait(event: &serde_json::Value, ask: bool) -> Option<Duration> {
    let field = |k: &str| event.get(k).and_then(|v| v.as_str()).unwrap_or_default();
    match field("hook_event_name") {
        "PermissionRequest" => Some(PERMISSION_WAIT),
        "Elicitation" => Some(ASK_WAIT),
        // OpenCode's question (our plugin's event), when the plugin can post an answer.
        "Question" if event.get("answerable").and_then(|a| a.as_bool()) == Some(true) => Some(ASK_WAIT),
        "PreToolUse" if ask && matches!(field("tool_name"), "AskUserQuestion" | "ExitPlanMode") => Some(ASK_WAIT),
        _ => None,
    }
}

/// The event `test` sends; the hub's agent module (`modules/agent`) knows it by this name.
const TEST_EVENT: &str = "ReUITest";

fn test_payload() -> String {
    serde_json::json!({ "hook_event_name": TEST_EVENT, "session_id": "reui-test" }).to_string()
}

/// `reui-bridge test <agent>`: the normal hook path, with a synthetic event.
fn test(agent: Option<&str>) -> Result<String, String> {
    let agent = match agent {
        Some(a @ ("claude" | "codex" | "opencode")) => a,
        other => return Err(format!("unsupported agent {other:?} (supported: claude, codex, opencode)")),
    };
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let pipe = timeout(CONNECT_TIMEOUT, connect()).await.ok().flatten().ok_or("ReUI isn't running")?;
        let (mut reader, mut writer) = tokio::io::split(pipe);
        write_frame(&mut writer, &ClientMsg::Hello { version: PROTOCOL_VERSION, surface: SurfaceKind::Bridge }).await.map_err(|e| e.to_string())?;
        match timeout(CONNECT_TIMEOUT, read_frame::<_, HubMsg>(&mut reader)).await {
            Ok(Ok(HubMsg::Welcome { .. })) => {}
            Ok(Ok(HubMsg::Rejected { reason })) => return Err(format!("ReUI refused the test: {reason}")),
            _ => return Err("ReUI didn't answer".into()),
        }
        write_frame(&mut writer, &ClientMsg::Hook { agent: agent.to_string(), payload: test_payload(), wants_reply: false }).await.map_err(|e| e.to_string())?;
        // Let the hub read it before the pipe closes.
        tokio::time::sleep(Duration::from_millis(100)).await;
        Ok("Sent a test event to ReUI".to_string())
    })
}

/// `reui-bridge timer <text>`: hands the text to the Timers module, as Settings sends
/// commands; with no text it opens the island's timer field instead.
fn timer(text: &str) -> Result<(), String> {
    let action = if text.trim().is_empty() { "launcher".to_string() } else { format!("start:{}", text.trim()) };
    command("timers", &action)
}

/// Sends one module command, as the Settings window does.
fn command(module: &str, action: &str) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let pipe = timeout(CONNECT_TIMEOUT, connect()).await.ok().flatten().ok_or("ReUI isn't running")?;
        let (mut reader, mut writer) = tokio::io::split(pipe);
        write_frame(&mut writer, &ClientMsg::Hello { version: PROTOCOL_VERSION, surface: SurfaceKind::Settings }).await.map_err(|e| e.to_string())?;
        match timeout(CONNECT_TIMEOUT, read_frame::<_, HubMsg>(&mut reader)).await {
            Ok(Ok(HubMsg::Welcome { .. })) => {}
            Ok(Ok(HubMsg::Rejected { reason })) => return Err(format!("ReUI refused: {reason}")),
            _ => return Err("ReUI didn't answer".into()),
        }
        write_frame(&mut writer, &ClientMsg::Command { module: module.into(), action: action.into() }).await.map_err(|e| e.to_string())?;
        tokio::time::sleep(Duration::from_millis(100)).await;
        Ok(())
    })
}

async fn forward(agent: &str, payload: String, wait: Option<Duration>) -> Option<String> {
    let pipe = timeout(CONNECT_TIMEOUT, connect()).await.ok()??;
    let (mut reader, mut writer) = tokio::io::split(pipe);
    write_frame(&mut writer, &ClientMsg::Hello { version: PROTOCOL_VERSION, surface: SurfaceKind::Bridge }).await.ok()?;
    match timeout(CONNECT_TIMEOUT, read_frame::<_, HubMsg>(&mut reader)).await.ok()?.ok()? {
        HubMsg::Welcome { .. } => {}
        _ => return None,
    }
    write_frame(&mut writer, &ClientMsg::Hook { agent: agent.to_string(), payload, wants_reply: wait.is_some() }).await.ok()?;
    let wait = wait?;
    match timeout(wait, read_frame::<_, HubMsg>(&mut reader)).await.ok()?.ok()? {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_event_is_named_and_never_waits() {
        let payload: serde_json::Value = serde_json::from_str(&test_payload()).unwrap();
        assert_eq!(payload["hook_event_name"], TEST_EVENT);
        assert!(payload["session_id"].is_string());
        assert!(test(Some("vim")).is_err());
        assert!(test(None).is_err());
    }

    #[test]
    fn only_what_needs_you_waits_and_never_past_its_hook() {
        use serde_json::json;
        let event = |name: &str, tool: &str| json!({ "hook_event_name": name, "tool_name": tool });
        assert_eq!(reply_wait(&event("PermissionRequest", "Bash"), false), Some(PERMISSION_WAIT));
        assert_eq!(reply_wait(&event("Elicitation", ""), false), Some(ASK_WAIT));
        assert_eq!(reply_wait(&event("PreToolUse", "AskUserQuestion"), true), Some(ASK_WAIT));
        assert_eq!(reply_wait(&event("PreToolUse", "ExitPlanMode"), true), Some(ASK_WAIT));
        // The async entry fires for the same tool and must not wait.
        assert_eq!(reply_wait(&event("PreToolUse", "AskUserQuestion"), false), None);
        assert_eq!(reply_wait(&event("PreToolUse", "Bash"), true), None);
        assert_eq!(reply_wait(&event("Stop", ""), true), None);
        assert_eq!(reply_wait(&json!({ "hook_event_name": "Question", "answerable": true }), false), Some(ASK_WAIT));
        assert_eq!(reply_wait(&json!({ "hook_event_name": "Question", "answerable": false }), false), None, "shown read-only");
        // The hook timeouts the installer writes: 120 s and 300 s.
        assert!(PERMISSION_WAIT < Duration::from_secs(120) && ASK_WAIT < Duration::from_secs(300));
    }
}
