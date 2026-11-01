//! The working folder a session's card names (docs/PRD-agent-folder.md): the agent's `cwd`
//! as a path you can tell apart from another session's, `~` for the home folder and,
//! when long, shortened from the middle so the drive and the project itself stay. Two
//! sessions in `D:\work\api` and `D:\old\api` read differently, and a command shows with
//! where it will run. Pure.

use std::path::Path;

/// Longest path the strip carries, in characters. The strip also holds what you asked, so
/// the path takes no more than about half of a roomy card. (A count, not a width: the
/// island cuts what still doesn't fit.)
pub(super) const FOLDER_CHARS: usize = 36;

const ELLIPSIS: &str = "\u{2026}";

/// `cwd` for the strip, or `None` when there's no path. `home` is the user's home folder.
pub(super) fn shown_folder(cwd: &str, home: &Path) -> Option<String> {
    let path = clean(cwd);
    if path.is_empty() {
        return None;
    }
    let home = clean(&home.to_string_lossy());
    let path = match strip_prefix_ci(&path, &home) {
        Some("") => "~".to_string(),
        Some(rest) if rest.starts_with('\\') => format!("~{rest}"),
        _ => path,
    };
    Some(shorten(&path, FOLDER_CHARS))
}

/// Backslashes, no `\\?\` (Codex), no trailing separator (but `D:\` stays a root).
fn clean(path: &str) -> String {
    let path = path.trim().trim_start_matches(r"\\?\").trim_start_matches("//?/").replace('/', "\\");
    let trimmed = path.trim_end_matches('\\');
    match trimmed.len() {
        0 if !path.is_empty() => "\\".into(),
        2 if trimmed.ends_with(':') => format!("{trimmed}\\"),
        _ => trimmed.to_string(),
    }
}

/// Windows paths are case-insensitive: `c:\users\me` is the home folder too.
fn strip_prefix_ci<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_empty() || prefix.len() > path.len() || !path.is_char_boundary(prefix.len()) {
        return None;
    }
    let (head, rest) = path.split_at(prefix.len());
    head.eq_ignore_ascii_case(prefix).then_some(rest)
}

/// Keeps the first part (drive, share or `~`) and as many of the last folders as fit,
/// the last one always whole; what's dropped becomes one `…`.
fn shorten(path: &str, max: usize) -> String {
    let len = |s: &str| s.chars().count();
    if len(path) <= max {
        return path.to_string();
    }
    let parts: Vec<&str> = path.split('\\').collect();
    // A UNC path (`\\server\share\…`) starts with two empty parts: keep `\\server`.
    let head_parts = if path.starts_with("\\\\") { 3 } else { 1 };
    if parts.len() <= head_parts + 1 {
        return path.to_string();
    }
    let head = parts[..head_parts].join("\\");
    let last = parts[parts.len() - 1];
    let mut tail = vec![last];
    for part in parts[head_parts..parts.len() - 1].iter().rev() {
        let candidate = len(&head) + 1 + len(ELLIPSIS) + 1 + len(part) + 1 + tail.iter().map(|p| len(p) + 1).sum::<usize>() - 1;
        if candidate > max {
            break;
        }
        tail.insert(0, part);
    }
    if tail.len() == parts.len() - head_parts {
        return path.to_string();
    }
    format!("{head}\\{ELLIPSIS}\\{}", tail.join("\\"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> &'static Path {
        Path::new(r"C:\Users\Rohit")
    }

    #[test]
    fn home_is_a_tilde() {
        assert_eq!(shown_folder(r"C:\Users\Rohit\code\api", home()).as_deref(), Some(r"~\code\api"));
        assert_eq!(shown_folder(r"c:\users\rohit\code\api", home()).as_deref(), Some(r"~\code\api"));
        assert_eq!(shown_folder(r"C:\Users\Rohit", home()).as_deref(), Some("~"));
        assert_eq!(shown_folder(r"C:\Users\Rohit\", home()).as_deref(), Some("~"));
        // A sibling whose name starts the same isn't home.
        assert_eq!(shown_folder(r"C:\Users\Rohitk\api", home()).as_deref(), Some(r"C:\Users\Rohitk\api"));
    }

    #[test]
    fn agents_paths_are_cleaned() {
        assert_eq!(shown_folder(r"\\?\D:\ocopus", home()).as_deref(), Some(r"D:\ocopus"));
        assert_eq!(shown_folder("D:/fusion-proxy/", home()).as_deref(), Some(r"D:\fusion-proxy"));
        assert_eq!(shown_folder(r"D:\", home()).as_deref(), Some(r"D:\"));
        assert_eq!(shown_folder("  ", home()), None);
        assert_eq!(shown_folder("", home()), None);
    }

    #[test]
    fn long_paths_lose_their_middle() {
        let long = r"D:\work\clients\northwind\2026\backend\services\billing-api";
        let shown = shown_folder(long, home()).unwrap();
        assert!(shown.chars().count() <= FOLDER_CHARS, "{shown}");
        assert_eq!(shown, r"D:\…\backend\services\billing-api");
        let under_home = shown_folder(r"C:\Users\Rohit\source\repos\company\platform\frontend\web-app", home()).unwrap();
        assert_eq!(under_home, r"~\…\platform\frontend\web-app");
    }

    #[test]
    fn the_project_folder_stays_whole() {
        let name = "a-really-long-project-folder-name-that-is-past-the-cap";
        assert_eq!(shown_folder(&format!(r"D:\work\{name}"), home()).unwrap(), format!(r"D:\…\{name}"));
        // Nothing to drop: as it is, and the island cuts it.
        assert_eq!(shown_folder(&format!(r"D:\{name}"), home()).unwrap(), format!(r"D:\{name}"));
    }

    #[test]
    fn shares_keep_their_server() {
        let shown = shown_folder(r"\\build-01\src\teams\payments\gateway\service", home()).unwrap();
        assert_eq!(shown, r"\\build-01\…\gateway\service");
        assert_eq!(shown_folder(r"\\nas\code", home()).as_deref(), Some(r"\\nas\code"));
    }
}
