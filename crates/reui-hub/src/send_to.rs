//! `reui-hub --shelf <paths…>`: files sent to the shelf from outside the island
//! (docs/PRD-send-to-shelf.md). Explorer's Send to → ReUI shelf runs it with the selected
//! files after the flag. The hub has no console (unlike `reui-bridge`), so nothing flashes.
//! With ReUI running, the paths go to its shelf over the pipe and this process exits;
//! otherwise this process becomes the hub and its shelf holds them once loaded.

use std::path::Path;
use std::time::Duration;

use reui_protocol::{pipe_name, read_frame, write_frame, ClientMsg, HubMsg, SurfaceKind, PROTOCOL_VERSION};
use tokio::net::windows::named_pipe::ClientOptions;
use tokio::time::{sleep, timeout, Instant};

pub const FLAG: &str = "--shelf";

/// How long to keep trying the running hub's pipe: it may be starting (Send to while
/// Windows signs in), or busy with another surface.
const CONNECT_FOR: Duration = Duration::from_secs(5);
const ANSWER_WAIT: Duration = Duration::from_millis(1000);

/// The flags before `--shelf`, and the paths after it (`None` without it). Everything after
/// the flag is a path, whatever it looks like: a file may be called `--demo`.
pub fn split(args: &[String]) -> (&[String], Option<Vec<String>>) {
    match args.iter().position(|a| a == FLAG) {
        Some(at) => (&args[..at], Some(args[at + 1..].to_vec())),
        None => (args, None),
    }
}

/// Full paths of what exists, in order, each once. Explorer sends full paths; a script's
/// relative ones are taken from the current folder.
pub fn existing(paths: &[String], exists: impl Fn(&Path) -> bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for path in paths.iter().map(|p| p.trim().trim_matches('"')).filter(|p| !p.is_empty()) {
        let Ok(full) = std::path::absolute(path) else { continue };
        let full = full.to_string_lossy().into_owned();
        if exists(Path::new(&full)) && !out.iter().any(|o| o.eq_ignore_ascii_case(&full)) {
            out.push(full);
        }
    }
    out
}

/// Hands the paths to the running hub's shelf. `false` if no hub answered in time.
pub async fn forward(paths: &[String]) -> bool {
    let action = reui_protocol::shelf_add(paths);
    let deadline = Instant::now() + CONNECT_FOR;
    loop {
        if send(&action).await.is_some() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        sleep(Duration::from_millis(100)).await;
    }
}

async fn send(action: &str) -> Option<()> {
    let pipe = ClientOptions::new().open(pipe_name()).ok()?;
    let (mut reader, mut writer) = tokio::io::split(pipe);
    write_frame(&mut writer, &ClientMsg::Hello { version: PROTOCOL_VERSION, surface: SurfaceKind::Settings }).await.ok()?;
    match timeout(ANSWER_WAIT, read_frame::<_, HubMsg>(&mut reader)).await.ok()?.ok()? {
        HubMsg::Welcome { .. } => {}
        _ => return None,
    }
    write_frame(&mut writer, &ClientMsg::Command { module: "shelf".into(), action: action.into() }).await.ok()?;
    // Let the hub read it before the pipe closes.
    sleep(Duration::from_millis(100)).await;
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn everything_after_the_flag_is_a_path() {
        let args = strings(&["--no-updates", "--shelf", r"C:\a.txt", "--demo"]);
        let (flags, paths) = split(&args);
        assert_eq!(flags, &strings(&["--no-updates"])[..]);
        assert_eq!(paths, Some(strings(&[r"C:\a.txt", "--demo"])));
        let plain = strings(&["--demo"]);
        assert_eq!(split(&plain), (&plain[..], None));
    }

    #[test]
    fn only_files_that_exist_once_each() {
        let there = |p: &Path| !p.to_string_lossy().contains("gone");
        let paths = strings(&[r"C:\Work\a.pdf", r"c:\work\A.pdf", r"C:\gone.txt", "  ", r#""C:\Work\b c.txt""#]);
        assert_eq!(existing(&paths, there), strings(&[r"C:\Work\a.pdf", r"C:\Work\b c.txt"]));
        let relative = existing(&strings(&["notes.txt"]), |_| true);
        assert!(Path::new(&relative[0]).is_absolute() && relative[0].ends_with("notes.txt"));
    }
}
