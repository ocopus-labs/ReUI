//! "Paste as" transforms for the clipboard window's `mode`. Everything except `normal`
//! pastes as plain text, so each transform is text in, text out; `markdown` also reads the
//! item's CF_HTML when it has one.

/// A paste mode from the window (`"mode"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// As copied, with its rich formats.
    #[default]
    Normal,
    Plain,
    Upper,
    Lower,
    Title,
    Trim,
    SingleLine,
    Json,
    Markdown,
}

impl Mode {
    /// Unknown modes are `Normal`: pasting as is is never wrong.
    pub fn parse(name: &str) -> Mode {
        match name {
            "plain" => Mode::Plain,
            "upper" => Mode::Upper,
            "lower" => Mode::Lower,
            "title" => Mode::Title,
            "trim" => Mode::Trim,
            "single-line" => Mode::SingleLine,
            "json" => Mode::Json,
            "markdown" => Mode::Markdown,
            _ => Mode::Normal,
        }
    }

    /// `text` transformed; `html` is the item's CF_HTML payload, used by `markdown`.
    pub fn apply(self, text: &str, html: Option<&str>) -> String {
        match self {
            Mode::Normal | Mode::Plain => text.to_string(),
            Mode::Upper => text.to_uppercase(),
            Mode::Lower => text.to_lowercase(),
            Mode::Title => title_case(text),
            Mode::Trim => text.trim().to_string(),
            Mode::SingleLine => text.split_whitespace().collect::<Vec<_>>().join(" "),
            Mode::Json => pretty_json(text).unwrap_or_else(|| text.to_string()),
            Mode::Markdown => html.and_then(html_to_markdown).filter(|md| !md.trim().is_empty()).unwrap_or_else(|| text.to_string()),
        }
    }
}

/// The first letter of every word upper case, the rest lower case.
fn title_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut start = true;
    for c in text.chars() {
        if c.is_alphanumeric() || c == '\'' || c == '’' {
            if start {
                out.extend(c.to_uppercase());
            } else {
                out.extend(c.to_lowercase());
            }
            start = false;
        } else {
            out.push(c);
            start = true;
        }
    }
    out
}

/// Valid JSON re-indented with two spaces. Keys keep their order and numbers their exact
/// spelling (a parse-and-print round trip would sort keys and could reformat numbers).
fn pretty_json(text: &str) -> Option<String> {
    serde_json::from_str::<serde::de::IgnoredAny>(text).ok()?;
    let mut out = String::with_capacity(text.len() * 2);
    let mut depth = 0usize;
    let mut chars = text.trim().chars().peekable();
    let newline = |out: &mut String, depth: usize| {
        out.push('\n');
        out.extend(std::iter::repeat_n("  ", depth));
    };
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                out.push('"');
                while let Some(c) = chars.next() {
                    out.push(c);
                    match c {
                        '\\' => out.extend(chars.next()),
                        '"' => break,
                        _ => {}
                    }
                }
            }
            '{' | '[' => {
                out.push(c);
                // Empty containers stay on one line.
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    chars.next();
                }
                if matches!(chars.peek(), Some('}' | ']')) {
                    out.extend(chars.next());
                } else {
                    depth += 1;
                    newline(&mut out, depth);
                }
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(c);
            }
            ',' => {
                out.push(',');
                newline(&mut out, depth);
            }
            ':' => out.push_str(": "),
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    Some(out)
}

/// The fragment of a CF_HTML payload (between the fragment markers), or the whole thing
/// when it's plain HTML.
fn html_fragment(html: &str) -> &str {
    let start = html.find("<!--StartFragment-->").map(|i| i + "<!--StartFragment-->".len());
    let end = html.find("<!--EndFragment-->");
    match (start, end) {
        (Some(s), Some(e)) if s <= e => &html[s..e],
        _ => html.find('<').map_or(html, |i| &html[i..]),
    }
}

/// Basic HTML → Markdown: headings, bold, italic, code, pre, links, images, lists, line
/// breaks, paragraphs; other tags are dropped and entities decoded. `None` when there is
/// no HTML in it.
pub fn html_to_markdown(html: &str) -> Option<String> {
    let html = html_fragment(html);
    if !html.contains('<') {
        return None;
    }
    let mut out = String::new();
    // Open lists: ordered (with the next number) or not.
    let mut lists: Vec<Option<u32>> = Vec::new();
    let mut links: Vec<Option<String>> = Vec::new();
    let mut pre = 0usize;
    let mut skip = 0usize; // inside <script>/<style>/<head>
    let mut rest = html;
    while !rest.is_empty() {
        let Some(open) = rest.find('<') else {
            if skip == 0 {
                push_text(&mut out, rest, pre > 0);
            }
            break;
        };
        if skip == 0 {
            push_text(&mut out, &rest[..open], pre > 0);
        }
        rest = &rest[open..];
        if rest.starts_with("<!--") {
            rest = rest.find("-->").map_or("", |e| &rest[e + 3..]);
            continue;
        }
        let Some(close) = rest.find('>') else { break };
        let tag = &rest[1..close];
        rest = &rest[close + 1..];
        let closing = tag.starts_with('/');
        let body = tag.trim_start_matches('/').trim_end_matches('/');
        let name = body.split(|c: char| c.is_whitespace()).next().unwrap_or("").to_ascii_lowercase();
        if matches!(name.as_str(), "script" | "style" | "head") {
            skip = if closing { skip.saturating_sub(1) } else { skip + 1 };
            continue;
        }
        if skip > 0 {
            continue;
        }
        match (name.as_str(), closing) {
            ("h1" | "h2" | "h3" | "h4" | "h5" | "h6", false) => {
                block(&mut out);
                let level = name[1..].parse().unwrap_or(1);
                out.extend(std::iter::repeat_n('#', level));
                out.push(' ');
            }
            ("h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "div" | "table" | "blockquote", true) => block(&mut out),
            ("p" | "div" | "table" | "blockquote", false) => block(&mut out),
            ("tr", true) => line(&mut out),
            ("td" | "th", true) => out.push(' '),
            ("br", _) => out.push('\n'),
            ("hr", _) => {
                block(&mut out);
                out.push_str("---");
                block(&mut out);
            }
            ("b" | "strong", _) => out.push_str("**"),
            ("i" | "em", _) => out.push('*'),
            ("s" | "del" | "strike", _) => out.push_str("~~"),
            ("code", _) if pre == 0 => out.push('`'),
            ("pre", false) => {
                block(&mut out);
                out.push_str("```\n");
                pre += 1;
            }
            ("pre", true) => {
                line(&mut out);
                out.push_str("```");
                pre = pre.saturating_sub(1);
                block(&mut out);
            }
            ("ul", false) => {
                line(&mut out);
                lists.push(None);
            }
            ("ol", false) => {
                line(&mut out);
                lists.push(Some(1));
            }
            ("ul" | "ol", true) => {
                lists.pop();
                if lists.is_empty() {
                    block(&mut out);
                }
            }
            ("li", false) => {
                line(&mut out);
                out.extend(std::iter::repeat_n("  ", lists.len().saturating_sub(1)));
                match lists.last_mut() {
                    Some(Some(n)) => {
                        out.push_str(&format!("{n}. "));
                        *n += 1;
                    }
                    _ => out.push_str("- "),
                }
            }
            ("a", false) => {
                let href = attribute(body, "href").filter(|h| !h.is_empty() && !h.starts_with('#'));
                if href.is_some() {
                    out.push('[');
                }
                links.push(href);
            }
            ("a", true) => {
                if let Some(Some(href)) = links.pop() {
                    out.push_str(&format!("]({href})"));
                }
            }
            ("img", _) => {
                if let Some(src) = attribute(body, "src") {
                    out.push_str(&format!("![{}]({src})", attribute(body, "alt").unwrap_or_default()));
                }
            }
            _ => {}
        }
    }
    // Tidy: no trailing spaces, at most one blank line in a row.
    let mut tidy = String::with_capacity(out.len());
    let mut blank = 0;
    for l in out.lines().map(str::trim_end) {
        if l.trim().is_empty() {
            blank += 1;
            if blank > 1 || tidy.is_empty() {
                continue;
            }
        } else {
            blank = 0;
        }
        tidy.push_str(l);
        tidy.push('\n');
    }
    Some(tidy.trim_end().to_string())
}

/// Ends the current line, then leaves one blank line (a paragraph break).
fn block(out: &mut String) {
    if out.is_empty() {
        return;
    }
    line(out);
    if !out.ends_with("\n\n") {
        out.push('\n');
    }
}

fn line(out: &mut String) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
}

/// Text between tags: whitespace collapses (as a browser shows it) except inside `<pre>`.
fn push_text(out: &mut String, text: &str, pre: bool) {
    let text = decode_entities(text);
    if pre {
        out.push_str(&text);
        return;
    }
    for (i, word) in text.split_whitespace().enumerate() {
        let at_line_start = out.is_empty() || out.ends_with('\n') || out.ends_with(' ');
        if (i > 0 || text.starts_with(char::is_whitespace)) && !at_line_start {
            out.push(' ');
        }
        out.push_str(word);
    }
    if text.ends_with(char::is_whitespace) && !text.trim().is_empty() {
        out.push(' ');
    }
}

/// An attribute's value from a tag body (`a href="x" class=y`).
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let at = from + i;
        from = at + name.len();
        let before_ok = at > 0 && lower.as_bytes()[at - 1].is_ascii_whitespace();
        let rest = tag[from..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let value = rest[1..].trim_start();
        let raw = match value.chars().next() {
            Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or(""),
            _ => value.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or(""),
        };
        return Some(decode_entities(raw));
    }
    None
}

fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find(';').filter(|&e| e <= 10);
        let decoded = end.and_then(|e| {
            let name = &rest[1..e];
            let c = match name {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                "nbsp" => ' ',
                _ => {
                    let n = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")).map(|h| u32::from_str_radix(h, 16));
                    let n = n.or_else(|| name.strip_prefix('#').map(str::parse));
                    char::from_u32(n?.ok()?)?
                }
            };
            Some((c, e))
        });
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_parse() {
        assert_eq!(Mode::parse("single-line"), Mode::SingleLine);
        assert_eq!(Mode::parse("markdown"), Mode::Markdown);
        assert_eq!(Mode::parse("normal"), Mode::Normal);
        assert_eq!(Mode::parse("bogus"), Mode::Normal);
    }

    #[test]
    fn text_transforms() {
        assert_eq!(Mode::Plain.apply("a\tb", Some("<b>x</b>")), "a\tb");
        assert_eq!(Mode::Upper.apply("straße", None), "STRASSE");
        assert_eq!(Mode::Lower.apply("ÉCOLE Ok", None), "école ok");
        assert_eq!(Mode::Title.apply("hello wORLD, it's o'neil-smith", None), "Hello World, It's O'neil-Smith");
        assert_eq!(Mode::Trim.apply("  \n hi there \t\n", None), "hi there");
        assert_eq!(Mode::SingleLine.apply("  one\r\n  two\t\tthree \n", None), "one two three");
    }

    #[test]
    fn json_is_pretty_printed_only_when_valid() {
        assert_eq!(Mode::Json.apply(r#"{"b":1,"a":[1,2.50,{}],"s":"x,{y}:\"z\""}"#, None), "{\n  \"b\": 1,\n  \"a\": [\n    1,\n    2.50,\n    {}\n  ],\n  \"s\": \"x,{y}:\\\"z\\\"\"\n}");
        assert_eq!(Mode::Json.apply("[]", None), "[]");
        assert_eq!(Mode::Json.apply("not {json", None), "not {json");
    }

    #[test]
    fn markdown_from_cf_html() {
        let cf = "Version:0.9\r\nStartHTML:0000000105\r\n<html><body>\r\n<!--StartFragment--><h2>Title</h2><p>Some <b>bold</b> and <em>soft</em> text with a <a href=\"https://x.com/?a=1&amp;b=2\">link</a>.</p><ul><li>one</li><li>two <code>x()</code></li></ul><ol><li>first</li><li>second</li></ol><pre>let a = 1;\n  b</pre><!--EndFragment-->\r\n</body></html>";
        let md = Mode::Markdown.apply("plain", Some(cf));
        assert_eq!(md, "## Title\n\nSome **bold** and *soft* text with a [link](https://x.com/?a=1&b=2).\n\n- one\n- two `x()`\n\n1. first\n2. second\n\n```\nlet a = 1;\n  b\n```");
        // No HTML: the plain text.
        assert_eq!(Mode::Markdown.apply("plain", None), "plain");
        assert_eq!(html_to_markdown("<p>caf&eacute; &#233; &#xE9;<br>x</p>").unwrap(), "caf&eacute; é é\nx");
        assert_eq!(html_to_markdown("<img alt=\"logo\" src=\"a.png\">").unwrap(), "![logo](a.png)");
    }
}
