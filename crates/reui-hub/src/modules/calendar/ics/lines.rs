//! RFC 5545 content lines (§3.1): a feed is unfolded into properties (`NAME;PARAM=v:value`)
//! grouped into components (`BEGIN:VEVENT` … `END:VEVENT`). Only the syntax: what a
//! property means is `ics/mod.rs`'s. Lenient on purpose: feeds in the wild use bare LF,
//! lower-case names and stray blank lines, and one bad line must never lose the rest.

/// One property: its name (upper-cased), parameters and raw value (still escaped).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub name: String,
    pub params: Vec<(String, String)>,
    pub value: String,
}

impl Property {
    /// A parameter's value (names compare case-insensitively).
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// The value as text: `\n`, `\,`, `\;` and `\\` unescaped (§3.3.11).
    pub fn text(&self) -> String {
        unescape(&self.value)
    }
}

/// A component and what's in it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Component {
    /// "VCALENDAR", "VEVENT", "VTIMEZONE", "STANDARD", ...
    pub kind: String,
    pub props: Vec<Property>,
    pub children: Vec<Component>,
}

impl Component {
    pub fn prop(&self, name: &str) -> Option<&Property> {
        self.props.iter().find(|p| p.name == name)
    }

    pub fn props<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Property> + 'a {
        self.props.iter().filter(move |p| p.name == name)
    }

    pub fn text(&self, name: &str) -> Option<String> {
        self.prop(name).map(Property::text).filter(|t| !t.trim().is_empty())
    }
}

/// The top-level components of a feed (normally one VCALENDAR). Unbalanced `END`s are
/// ignored and components left open at the end are closed, so a truncated download still
/// yields the events it got to.
pub fn parse(text: &str) -> Vec<Component> {
    let mut stack: Vec<Component> = Vec::new();
    let mut top = Vec::new();
    for line in unfold(text) {
        let Some(prop) = property(&line) else { continue };
        match prop.name.as_str() {
            "BEGIN" => stack.push(Component { kind: prop.value.trim().to_ascii_uppercase(), ..Component::default() }),
            "END" => {
                let kind = prop.value.trim().to_ascii_uppercase();
                if !stack.iter().any(|c| c.kind == kind) {
                    continue;
                }
                while let Some(done) = stack.pop() {
                    let matched = done.kind == kind;
                    close(&mut stack, &mut top, done);
                    if matched {
                        break;
                    }
                }
            }
            _ => {
                if let Some(open) = stack.last_mut() {
                    open.props.push(prop);
                }
            }
        }
    }
    while let Some(done) = stack.pop() {
        close(&mut stack, &mut top, done);
    }
    top
}

fn close(stack: &mut [Component], top: &mut Vec<Component>, done: Component) {
    match stack.last_mut() {
        Some(parent) => parent.children.push(done),
        None => top.push(done),
    }
}

/// Lines with their folds joined: a line starting with a space or tab continues the one
/// before, minus that one character (§3.1).
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.trim_start_matches('\u{feff}').split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        match (raw.chars().next(), lines.last_mut()) {
            (Some(' ' | '\t'), Some(last)) => last.push_str(&raw[1..]),
            _ if raw.is_empty() => {}
            _ => lines.push(raw.to_string()),
        }
    }
    lines
}

/// `NAME;P1=a;P2="b:c":value`. `None` for a line without a colon or a name.
fn property(line: &str) -> Option<Property> {
    let mut params = Vec::new();
    let (name_end, mut rest) = match line.find([';', ':']) {
        Some(i) => (i, &line[i..]),
        None => return None,
    };
    let name = line[..name_end].trim().to_ascii_uppercase();
    if name.is_empty() {
        return None;
    }
    loop {
        if let Some(value) = rest.strip_prefix(':') {
            return Some(Property { name, params, value: value.to_string() });
        }
        rest = rest.strip_prefix(';')?;
        let eq = rest.find('=')?;
        let pname = rest[..eq].trim().to_ascii_uppercase();
        rest = &rest[eq + 1..];
        // Values up to the next ';' or ':' outside quotes; several are comma-separated.
        let mut value = String::new();
        let mut quoted = false;
        let mut end = rest.len();
        for (i, c) in rest.char_indices() {
            match c {
                '"' => quoted = !quoted,
                ';' | ':' if !quoted => {
                    end = i;
                    break;
                }
                c => value.push(c),
            }
        }
        params.push((pname, value));
        rest = &rest[end..];
        if rest.is_empty() {
            return None;
        }
    }
}

/// TEXT unescaping (§3.3.11); an unknown escape keeps its character.
pub fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folded_lines_and_quoted_params_parse() {
        let feed = "BEGIN:VCALENDAR\r\nX-WR-CALNAME:Work\r\nBEGIN:VEVENT\r\nSUMMARY:Design \r\n review\\, part 2\r\nATTENDEE;CN=\"Sam: PM\";PARTSTAT=DECLINED:mailto:sam@example.com\r\nDESCRIPTION:Line one\\nLine two\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let top = parse(feed);
        assert_eq!(top.len(), 1);
        let cal = &top[0];
        assert_eq!(cal.text("X-WR-CALNAME").as_deref(), Some("Work"));
        let ev = &cal.children[0];
        assert_eq!(ev.kind, "VEVENT");
        assert_eq!(ev.text("SUMMARY").as_deref(), Some("Design review, part 2"));
        let att = ev.prop("ATTENDEE").unwrap();
        assert_eq!(att.param("cn"), Some("Sam: PM"));
        assert_eq!(att.param("PARTSTAT"), Some("DECLINED"));
        assert_eq!(att.value, "mailto:sam@example.com");
        assert_eq!(ev.text("DESCRIPTION").as_deref(), Some("Line one\nLine two"));
    }

    #[test]
    fn bare_newlines_lowercase_and_truncation_are_tolerated() {
        let feed = "begin:vcalendar\nbegin:vevent\nsummary:One\nend:vevent\nbegin:vevent\nsummary:Cut off";
        let top = parse(feed);
        assert_eq!(top[0].children.len(), 2, "the truncated event is still there");
        assert_eq!(top[0].children[1].text("SUMMARY").as_deref(), Some("Cut off"));
        assert!(parse("garbage\nEND:VEVENT\n").is_empty());
    }
}
