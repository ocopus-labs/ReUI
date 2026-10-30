//! What the launcher's field means: `10` (minutes), `10m`, `90s`, `1h30`, `1h 30m`,
//! `1:30:00`, `5:00`, `10 min`, `2 hours`, `½h`, with anything else as the label
//! (`10m tea`, `tea 10m`); `focus` / `pomodoro` and `sw` / `stopwatch` start those instead.

pub const MAX_MS: u64 = 24 * 3_600_000;

#[derive(Debug, Clone, PartialEq)]
pub enum Parsed {
    /// Nothing typed yet.
    Empty,
    Timer { ms: u64, label: Option<String> },
    Focus,
    Stopwatch,
    /// A length over [`MAX_MS`].
    TooLong,
    /// Not a length we understand (or zero).
    Unknown,
}

pub fn parse(input: &str) -> Parsed {
    let words: Vec<&str> = input.split_whitespace().collect();
    if words.is_empty() {
        return Parsed::Empty;
    }
    let lower = input.trim().to_lowercase();
    match lower.as_str() {
        "focus" | "pomodoro" | "pomo" => return Parsed::Focus,
        "sw" | "stopwatch" => return Parsed::Stopwatch,
        _ => {}
    }
    // The longest run of words at the start that reads as a length, else at the end.
    let n = words.len();
    let found = (1..=n).rev().find_map(|k| length(&words[..k]).map(|ms| (ms, &words[k..])))
        .or_else(|| (1..n).find_map(|k| length(&words[k..]).map(|ms| (ms, &words[..k]))));
    match found {
        Some((0, _)) | None => Parsed::Unknown,
        Some((ms, _)) if ms > MAX_MS => Parsed::TooLong,
        Some((ms, rest)) => Parsed::Timer { ms, label: label(rest) },
    }
}

/// The rest as a label, first letter capitalised ("tea" → "Tea").
fn label(words: &[&str]) -> Option<String> {
    let text = words.join(" ");
    let mut chars = text.chars();
    let first = chars.next()?;
    Some(first.to_uppercase().chain(chars).collect())
}

/// Words that together are one length ("1h", "30m" / "10", "min").
pub(super) fn length(words: &[&str]) -> Option<u64> {
    let joined: String = words.concat().to_lowercase();
    if joined.contains(':') {
        return colon(&joined);
    }
    let mut rest = joined.as_str();
    let mut total = 0f64;
    // The unit a bare number takes: minutes alone, the next unit down after a unit.
    let mut implied = Some(60_000f64);
    let mut parts = 0;
    while !rest.is_empty() {
        let (number, after) = number(rest)?;
        let letters = after.len() - after.trim_start_matches(|c: char| c.is_ascii_alphabetic()).len();
        let (unit, after) = after.split_at(letters);
        let unit_ms = match unit {
            "" => implied?,
            "h" | "hr" | "hrs" | "hour" | "hours" => 3_600_000.0,
            "m" | "min" | "mins" | "minute" | "minutes" => 60_000.0,
            "s" | "sec" | "secs" | "second" | "seconds" => 1_000.0,
            _ => return None,
        };
        if unit.is_empty() && parts > 0 && !after.is_empty() {
            return None;
        }
        total += number * unit_ms;
        parts += 1;
        implied = match unit_ms as u64 {
            3_600_000 => Some(60_000.0),
            60_000 => Some(1_000.0),
            _ => None,
        };
        if unit.is_empty() {
            implied = None;
        }
        rest = after;
    }
    (parts > 0).then_some(total.round() as u64)
}

/// A leading number: digits with an optional fraction, or ½.
fn number(s: &str) -> Option<(f64, &str)> {
    if let Some(after) = s.strip_prefix('½') {
        return Some((0.5, after));
    }
    let end = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
    let value = s[..end].parse::<f64>().ok().filter(|v| v.is_finite())?;
    Some((value, &s[end..]))
}

/// `m:ss` or `h:mm:ss`.
fn colon(s: &str) -> Option<u64> {
    let parts: Vec<u64> = s.split(':').map(|p| if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) { None } else { p.parse().ok() }).collect::<Option<_>>()?;
    let secs = match parts.as_slice() {
        [m, s] if *s < 60 => m * 60 + s,
        [h, m, s] if *m < 60 && *s < 60 => h * 3600 + m * 60 + s,
        _ => return None,
    };
    Some(secs * 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(input: &str) -> Option<u64> {
        match parse(input) {
            Parsed::Timer { ms, .. } => Some(ms),
            _ => None,
        }
    }

    fn label_of(input: &str) -> Option<String> {
        match parse(input) {
            Parsed::Timer { label, .. } => label,
            _ => None,
        }
    }

    const MIN: u64 = 60_000;

    #[test]
    fn lengths() {
        assert_eq!(ms("10"), Some(10 * MIN));
        assert_eq!(ms("10m"), Some(10 * MIN));
        assert_eq!(ms("90s"), Some(90_000));
        assert_eq!(ms("1h30"), Some(90 * MIN));
        assert_eq!(ms("1h 30m"), Some(90 * MIN));
        assert_eq!(ms("1h30m15s"), Some(90 * MIN + 15_000));
        assert_eq!(ms("2m30"), Some(150_000));
        assert_eq!(ms("1:30:00"), Some(90 * MIN));
        assert_eq!(ms("5:00"), Some(5 * MIN));
        assert_eq!(ms("10 min"), Some(10 * MIN));
        assert_eq!(ms("2 hours"), Some(120 * MIN));
        assert_eq!(ms("½h"), Some(30 * MIN));
        assert_eq!(ms("1.5h"), Some(90 * MIN));
        assert_eq!(ms("25M"), Some(25 * MIN));
    }

    #[test]
    fn labels_before_or_after() {
        assert_eq!(ms("10m tea"), Some(10 * MIN));
        assert_eq!(label_of("10m tea"), Some("Tea".into()));
        assert_eq!(ms("tea 4m"), Some(4 * MIN));
        assert_eq!(label_of("tea 4m"), Some("Tea".into()));
        assert_eq!(label_of("10 min pasta water"), Some("Pasta water".into()));
        assert_eq!(label_of("10m"), None);
    }

    #[test]
    fn words_and_limits() {
        assert_eq!(parse("focus"), Parsed::Focus);
        assert_eq!(parse(" Pomodoro "), Parsed::Focus);
        assert_eq!(parse("sw"), Parsed::Stopwatch);
        assert_eq!(parse("stopwatch"), Parsed::Stopwatch);
        assert_eq!(parse("   "), Parsed::Empty);
        assert_eq!(parse("0"), Parsed::Unknown);
        assert_eq!(parse("0s"), Parsed::Unknown);
        assert_eq!(parse("-5"), Parsed::Unknown);
        assert_eq!(parse("tea"), Parsed::Unknown);
        assert_eq!(parse("5:75"), Parsed::Unknown);
        assert_eq!(parse("10x"), Parsed::Unknown);
        assert_eq!(parse("25h"), Parsed::TooLong);
        assert_eq!(ms("24h"), Some(24 * 60 * MIN));
    }
}
