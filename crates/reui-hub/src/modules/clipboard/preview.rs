//! What copied text is, for its card and its type: a colour, a link, code or plain text.

use reui_protocol::{Preview, Rgb};

/// The first `chars` characters.
pub(super) fn prefix(text: &str, chars: usize) -> &str {
    text.char_indices().nth(chars).map_or(text, |(i, _)| &text[..i])
}

/// What copied text is, for its card: a colour, a link, code, or plain text. Only the
/// start of a long text is looked at.
pub(super) fn classify(text: &str) -> Preview {
    let text = prefix(text, 16 * 1024);
    let trimmed = text.trim();
    if let Some(rgb) = parse_color(trimmed) {
        return Preview::Color(rgb);
    }
    if !trimmed.contains(char::is_whitespace) {
        if let Some(rest) = trimmed.strip_prefix("https://").or_else(|| trimmed.strip_prefix("http://")) {
            let (host, path) = rest.split_once('/').map_or((rest, ""), |(h, p)| (h, p));
            // Never show `user:password@`.
            let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
            return Preview::Link { host: host.trim_start_matches("www.").to_string(), path: format!("/{path}"), title: None, site: None, image: None };
        }
    }
    let lines: Vec<&str> = text.lines().collect();
    let code_marks = lines.iter().filter(|l| l.trim_end().ends_with([';', '{', '}', ')']) || l.starts_with("    ") || l.starts_with('\t')).count();
    if lines.len() >= 2 && code_marks * 2 >= lines.len() {
        return Preview::Code(text.to_string());
    }
    Preview::Text(trimmed.to_string())
}

/// `#RGB`, `#RRGGBB`, `rgb(r, g, b)`.
fn parse_color(text: &str) -> Option<Rgb> {
    if let Some(hex) = text.strip_prefix('#') {
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let hex: Vec<u8> = match hex.len() {
            3 => hex.chars().map(|c| c.to_digit(16).map(|d| (d * 17) as u8)).collect::<Option<_>>()?,
            6 => (0..3).map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()).collect::<Option<_>>()?,
            _ => return None,
        };
        return Some(Rgb(hex[0], hex[1], hex[2]));
    }
    let inner = text.strip_prefix("rgb(")?.strip_suffix(')')?;
    let parts: Vec<u8> = inner.split(',').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
    (parts.len() == 3).then(|| Rgb(parts[0], parts[1], parts[2]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copied_text_is_recognised() {
        assert_eq!(classify("#0A84FF"), Preview::Color(Rgb(10, 132, 255)));
        assert_eq!(classify("#fff"), Preview::Color(Rgb(255, 255, 255)));
        assert_eq!(classify("rgb(48, 209, 88)"), Preview::Color(Rgb(48, 209, 88)));
        assert_eq!(classify("https://www.figma.com/design/abc"), Preview::Link { host: "figma.com".into(), path: "/design/abc".into(), title: None, site: None, image: None });
        assert!(matches!(classify("fn main() {\n    println!(\"hi\");\n}"), Preview::Code(_)));
        assert_eq!(classify("  Minneapolis 55410  "), Preview::Text("Minneapolis 55410".into()));
        assert_eq!(classify("see https://example.com later"), Preview::Text("see https://example.com later".into()));
        assert_eq!(classify("#ab€c"), Preview::Text("#ab€c".into()));
        assert_eq!(classify("#€€"), Preview::Text("#€€".into()));
        assert!(matches!(classify("https://me:hunter2@example.com/x"), Preview::Link { host, .. } if host == "example.com"));
    }

    #[test]
    fn previews_are_cut_on_character_boundaries() {
        assert_eq!(prefix("héllo", 2), "hé");
        assert_eq!(prefix("hi", 10), "hi");
        assert_eq!(prefix(&"€".repeat(5000), 2000).chars().count(), 2000);
    }
}
