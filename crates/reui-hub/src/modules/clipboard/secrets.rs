//! Does copied text look like a secret? Detected secrets are never stored (PRD D5).
//!
//! Two kinds of evidence: well-known credential shapes (private key blocks, cloud and
//! service tokens with fixed prefixes, JWTs), found anywhere in the text; and a copy that
//! is one long random-looking token. The second is where false positives live, so it
//! stays narrow: URLs, paths, emails, hex (commit hashes, digests), UUIDs and anything
//! made of words are never "random".

/// True when `text` contains a credential, or is one.
pub fn looks_secret(text: &str) -> bool {
    let text = text.trim();
    private_key(text) || tokens(text).any(known_token) || random_token(text)
}

/// A PEM / OpenSSH / PGP private key header followed by key material.
fn private_key(text: &str) -> bool {
    let Some(start) = text.find("-----BEGIN ") else { return false };
    let rest = &text[start..];
    let Some((header, body)) = rest.split_once('\n') else { return false };
    let header = header.trim();
    let is_key = header.ends_with("PRIVATE KEY-----") || header == "-----BEGIN PGP PRIVATE KEY BLOCK-----";
    // Just the header (in docs, or a grep result) isn't a key; some base64 under it is.
    is_key && body.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=')).count() >= 64
        || rest.len() > header.len() && private_key(&rest[header.len()..])
}

/// Candidate tokens: runs of `[A-Za-z0-9_.-]`, without trailing sentence punctuation.
fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))).map(|t| t.trim_matches(['.', '-'])).filter(|t| t.len() >= 20)
}

fn known_token(t: &str) -> bool {
    let all = |s: &str, f: fn(u8) -> bool| s.bytes().all(f);
    let alnum = |b: u8| b.is_ascii_alphanumeric();
    // AWS access key ids: base32 (A-Z, 2-7); a digit tells them apart from shouted words.
    if t.len() == 20 && (t.starts_with("AKIA") || t.starts_with("ASIA")) {
        let id = &t[4..];
        return all(id, |b| b.is_ascii_uppercase() || (b'2'..=b'7').contains(&b)) && id.bytes().any(|b| b.is_ascii_digit());
    }
    for prefix in ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"] {
        if let Some(rest) = t.strip_prefix(prefix) {
            return (30..=255).contains(&rest.len()) && all(rest, alnum);
        }
    }
    if let Some(rest) = t.strip_prefix("github_pat_") {
        return rest.len() >= 40 && all(rest, |b| b.is_ascii_alphanumeric() || b == b'_');
    }
    if t.len() > 5 && t.starts_with("xox") && b"abprs".contains(&t.as_bytes()[3]) && t.as_bytes()[4] == b'-' {
        let rest = &t[5..];
        return rest.len() >= 10 && all(rest, |b| b.is_ascii_alphanumeric() || b == b'-') && rest.bytes().any(|b| b.is_ascii_digit());
    }
    for prefix in ["sk_live_", "rk_live_"] {
        if let Some(rest) = t.strip_prefix(prefix) {
            return rest.len() >= 16 && all(rest, alnum);
        }
    }
    if let Some(rest) = t.strip_prefix("AIza") {
        return rest.len() == 35 && all(rest, |b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    }
    jwt(t)
}

/// `header.payload.signature`, base64url, where the header is JSON naming an `alg`.
fn jwt(t: &str) -> bool {
    let parts: Vec<&str> = t.split('.').collect();
    if parts.len() != 3 || !parts[0].starts_with("eyJ") || parts[1].len() < 4 {
        return false;
    }
    if !parts.iter().all(|p| p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')) {
        return false;
    }
    base64url(parts[0]).and_then(|h| String::from_utf8(h).ok()).is_some_and(|h| h.trim_start().starts_with('{') && h.contains("\"alg\""))
}

fn base64url(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for b in s.bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// The whole copy is one long token that looks machine-made: 32+ chars, no spaces,
/// lower + upper + digits, ≥ 4 bits of entropy per char, and not mostly words.
fn random_token(t: &str) -> bool {
    let n = t.len();
    if !(32..=512).contains(&n) || !t.is_ascii() || t.contains(char::is_whitespace) {
        return false;
    }
    let path = t.contains('\\') || t.starts_with(['/', '~', '.']) || t.as_bytes().get(1) == Some(&b':');
    // Dots mean file names, domains and versions; dotted credentials (JWTs) are matched by shape.
    if path || t.contains(['.', '@']) || t.contains("://") || t.starts_with('#') || uuid(t) {
        return false;
    }
    let hex = t.strip_prefix("0x").unwrap_or(t);
    if hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    let has = |f: fn(&u8) -> bool| t.as_bytes().iter().any(f);
    if !(has(u8::is_ascii_lowercase) && has(u8::is_ascii_uppercase) && has(u8::is_ascii_digit)) {
        return false;
    }
    // Identifiers are mostly words ("getAccountSettings": runs of 3+ lower-case letters,
    // with their capital); in random base64 such runs are short and rare.
    let bytes = t.as_bytes();
    let (mut wordy, mut i) = (0, 0);
    while i < n {
        let start = i;
        while i < n && bytes[i].is_ascii_lowercase() {
            i += 1;
        }
        let run = i - start;
        if run >= 3 {
            wordy += run + usize::from(start > 0 && bytes[start - 1].is_ascii_uppercase());
        }
        i += usize::from(run == 0);
    }
    if wordy * 2 >= n {
        return false;
    }
    entropy(t) >= 4.0
}

fn uuid(t: &str) -> bool {
    let t = t.trim_matches(['{', '}']);
    let groups: Vec<&str> = t.split('-').collect();
    groups.iter().map(|g| g.len()).eq([8, 4, 4, 4, 12]) && groups.iter().all(|g| g.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Shannon entropy in bits per character.
fn entropy(t: &str) -> f64 {
    let mut counts = [0u32; 256];
    for b in t.bytes() {
        counts[b as usize] += 1;
    }
    let n = t.len() as f64;
    counts.iter().filter(|&&c| c > 0).map(|&c| f64::from(c) / n).map(|p| -p * p.log2()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_are_detected() {
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA3Tz2mr7SZiAMfQyuvBjM9Oi7m2uR6vOFcVQhXZ1bXtq8+K4l\nWQ8xUkMjB0xZHjE=\n-----END RSA PRIVATE KEY-----";
        let secrets = [
            pem.to_string(),
            "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\n".into(),
            "AKIAIOSFODNN7EXAMPLE".into(),
            "aws_access_key_id = ASIAY34FZKBOKMUTVV7A".into(),
            "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".into(),
            "token: gho_16C7e42F292c6912E7710c838347Ae178B4a.".into(),
            "github_pat_11ABCDEFG0123456789_abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ012345".into(),
            "xoxb-2048-123456789012-AbCdEfGhIjKlMnOpQrStUvWx".into(),
            "sk_live_4eC39HqLyjWDarjtT1zdp7dc".into(),
            "rk_live_51H8bXYZabc123DEF456ghi".into(),
            "AIzaSyD-9tSrke72PouQMnMX-a7eZSW0jkFMBWY".into(),
            "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4ifQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c".into(),
            "Zx8Qk2LmN4pR7sT9vW1yB3dF6hJ0aC5e".into(),
            "q7Hc9Kd2Lp/Xw4Rt+Vb8Nm3Js6Fg1Za0".into(),
        ];
        for s in &secrets {
            assert!(looks_secret(s), "missed: {s}");
        }
    }

    #[test]
    fn ordinary_copies_are_not_secrets() {
        let ordinary = [
            "hello world",
            "https://www.figma.com/design/AbC123xYz987/My-File?node-id=12-34&t=Zx8Qk2LmN4pR7sT9",
            "https://example.com/reset?token=abc",
            r"C:\Users\Rohit\AppData\Roaming\ReUI\clipboard.db",
            "/usr/local/lib/python3.12/site-packages/requests/__init__.py",
            "crates/reui-hub/src/modules/clipboard_store.rs",
            "#0A84FF",
            "#ff00aa80",
            "550e8400-e29b-41d4-a716-446655440000",
            "{550E8400-E29B-41D4-A716-446655440000}",
            "3f786850e387550fdab836ed7e6dc881de23001b",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
            "Supercalifragilisticexpialidocious",
            "getUserAccountSettingsForTenant2",
            "ThisIsAVeryLongIdentifierNameForTesting123",
            "rohit.sharma+newsletter@example.com",
            "my-very-long-file-name-for-the-report-final-v2.docx",
            "AKIA is the prefix of AWS keys",
            "ASIAPACIFICCOMPANIES",
            "-----BEGIN RSA PRIVATE KEY-----",
            "eyJhbGciOiJIUzI1NiJ9 is a JWT header",
            "eyJub3QiOiJhIGp3dCJ9.eyJzdWIiOiIxIn0.c2ln",
            "Settings_Version_2_Final_Draft_Copy_3",
            "AppleWebKit/537.36 (KHTML, like Gecko)",
            "fn main() {\n    println!(\"hi\");\n}",
            "The quick brown fox jumps over the lazy dog 1234567890",
            "ghp_short",
            "xoxb-not-a-token",
            "SELECT id, name FROM users WHERE created_at > now() - interval '7 days';",
        ];
        for s in ordinary {
            assert!(!looks_secret(s), "false positive: {s}");
        }
    }

    #[test]
    fn entropy_and_base64_helpers() {
        assert!(entropy("aaaa") < 0.01);
        assert!((entropy("abcd") - 2.0).abs() < 1e-9);
        assert_eq!(base64url("eyJhbGciOiJIUzI1NiJ9").as_deref(), Some(&b"{\"alg\":\"HS256\"}"[..]));
        assert!(uuid("550e8400-e29b-41d4-a716-446655440000"));
    }
}
