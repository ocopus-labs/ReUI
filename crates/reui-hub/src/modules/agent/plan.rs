//! A plan (ExitPlanMode's Markdown) read for its card: the title from its first heading,
//! how many steps it lists, and the rest as quiet prose (PRD agent moments §5.5: list
//! markers kept, no other Markdown). Pure.

/// The first heading's text, without its `#`s.
pub(super) fn title(plan: &str) -> Option<String> {
    plan.lines().map(str::trim).find_map(heading).map(|h| h.to_string())
}

/// Top-level list items (`1.`, `-`, `*`), which is how plans list their steps.
pub(super) fn steps(plan: &str) -> usize {
    plan.lines().filter(|l| !l.starts_with([' ', '\t']) && item(l).is_some()).count()
}

/// The plan without its title (the hero shows it), headings as plain lines, `**` / `__`
/// emphasis and backticks dropped, blank runs folded to one.
pub(super) fn prose(plan: &str) -> String {
    let title_line = plan.lines().position(|l| heading(l.trim()).is_some());
    let mut out: Vec<String> = Vec::new();
    for (i, line) in plan.lines().enumerate() {
        if Some(i) == title_line {
            continue;
        }
        let line = line.trim_end();
        let text = heading(line.trim()).unwrap_or(line);
        let text = text.replace("**", "").replace("__", "").replace('`', "");
        if text.trim().is_empty() && out.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        out.push(if text.trim().is_empty() { String::new() } else { text });
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out.join("\n")
}

fn heading(line: &str) -> Option<&str> {
    let rest = line.trim_start_matches('#');
    (rest.len() < line.len() && rest.starts_with(' ')).then(|| rest.trim()).filter(|h| !h.is_empty())
}

fn item(line: &str) -> Option<&str> {
    if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
        return Some(rest);
    }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    (digits > 0).then(|| &line[digits..]).and_then(|r| r.strip_prefix(". ").or_else(|| r.strip_prefix(") ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAN: &str = "# Refactor auth\n\nMove the token logic out of `main.rs`.\n\n## Steps\n1. Extract **TokenStore**\n   - keep the old API\n2. Add tests\n3. Update the docs\n\n\n- Ship it\n";

    #[test]
    fn plans_read_as_title_steps_and_prose() {
        assert_eq!(title(PLAN).as_deref(), Some("Refactor auth"));
        assert_eq!(title("Just do it\n1. now"), None);
        assert_eq!(title("#hashtag\n## Real"), Some("Real".into()));
        assert_eq!(steps(PLAN), 4, "nested items aren't steps");
        assert_eq!(prose(PLAN), "Move the token logic out of main.rs.\n\nSteps\n1. Extract TokenStore\n   - keep the old API\n2. Add tests\n3. Update the docs\n\n- Ship it");
    }
}
