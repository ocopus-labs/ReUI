//! `reui-bridge finished --exit N --seconds S --shell NAME`: a shell's hook saying a long
//! command finished (docs/PRD-terminal-commands.md). The command line comes in the
//! `REUI_COMMAND` variable (quoting it as an argument breaks in Windows PowerShell), or
//! after `--`. It goes to the hub's `commands` module as `finished:<json>`, with the
//! bridge's parent chain so the hub can tell whether that terminal is in front and bring it
//! forward. Pure; `main.rs` sends it, and nothing is ever printed.

use serde_json::json;

/// The command the hub gets, or `None` when the arguments don't say enough.
pub fn action(args: &[String], env_command: Option<String>, parents: Vec<u32>, console: isize) -> Option<String> {
    let mut exit: Option<i64> = None;
    let mut seconds: Option<u64> = None;
    let mut shell = String::new();
    let mut after_dashes: Vec<&str> = Vec::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--exit" => exit = rest.next().and_then(|v| v.trim().parse().ok()),
            "--seconds" => seconds = rest.next().and_then(|v| v.trim().parse().ok()),
            "--shell" => shell = rest.next().cloned().unwrap_or_default(),
            "--" => {
                after_dashes = rest.by_ref().map(String::as_str).collect();
                break;
            }
            _ => {}
        }
    }
    let command = env_command.filter(|c| !c.trim().is_empty()).unwrap_or_else(|| after_dashes.join(" "));
    let command = command.trim();
    if command.is_empty() {
        return None;
    }
    let payload = json!({
        "command": command,
        "exit": exit?,
        "seconds": seconds?,
        "shell": shell,
        "reui_parents": parents,
        "reui_console": console,
    });
    Some(format!("finished:{payload}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_command_comes_from_the_variable_or_after_dashes() {
        let args = strings(&["--exit", "2", "--seconds", "75", "--shell", "powershell"]);
        let action = action(&args, Some("cargo build --release".into()), vec![30, 20], 0).unwrap();
        let payload: serde_json::Value = serde_json::from_str(action.strip_prefix("finished:").unwrap()).unwrap();
        assert_eq!(payload["command"], "cargo build --release");
        assert_eq!((payload["exit"].as_i64(), payload["seconds"].as_u64(), payload["shell"].as_str()), (Some(2), Some(75), Some("powershell")));
        assert_eq!(payload["reui_parents"], json!([30, 20]));
        let dashes = strings(&["--exit", "0", "--seconds", "12", "--", "npm", "test"]);
        assert!(super::action(&dashes, None, vec![], 0).unwrap().contains(r#""command":"npm test""#));
    }

    #[test]
    fn too_little_to_say_is_nothing() {
        assert_eq!(action(&strings(&["--exit", "0", "--seconds", "12"]), None, vec![], 0), None, "no command");
        assert_eq!(action(&strings(&["--seconds", "12"]), Some("make".into()), vec![], 0), None, "no exit code");
        assert_eq!(action(&strings(&["--exit", "x", "--seconds", "12"]), Some("make".into()), vec![], 0), None);
        assert_eq!(action(&strings(&["--exit", "0"]), Some("make".into()), vec![], 0), None, "no time");
    }
}
