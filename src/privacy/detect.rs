//! Password / secret detection heuristics. Pure functions, unit-tested.

use crate::model::item::{SensitiveReason, Sensitivity};
use crate::settings::{Privacy, SensitiveFormats};
use crate::system::process::name_in_list;

/// Decide sensitivity for a capture. Returns (sensitivity, skip_entirely).
pub fn evaluate(text: Option<&str>, source_exe: Option<&str>, format_sensitive: bool, p: &Privacy) -> (Sensitivity, bool) {
    if format_sensitive {
        match p.formats {
            SensitiveFormats::Skip => return (Sensitivity::Auto(SensitiveReason::Format), true),
            SensitiveFormats::MarkSensitive => return (Sensitivity::Auto(SensitiveReason::Format), false),
            SensitiveFormats::Ignore => {}
        }
    }
    if p.process_detection {
        if let Some(exe) = source_exe {
            if name_in_list(exe, &p.process_list) {
                return (Sensitivity::Auto(SensitiveReason::SourceProcess), false);
            }
        }
    }
    if p.heuristic {
        if let Some(t) = text {
            if let Some(score) = password_score(t, p.keys_are_sensitive) {
                if score >= p.threshold {
                    return (Sensitivity::Auto(SensitiveReason::Heuristic { score }), false);
                }
            }
        }
    }
    (Sensitivity::None, false)
}

fn shannon_entropy(s: &str) -> f32 {
    let bytes = s.as_bytes();
    let mut counts = [0u32; 256];
    for &b in bytes {
        counts[b as usize] += 1;
    }
    let n = bytes.len() as f32;
    let mut h = 0f32;
    for &c in &counts {
        if c > 0 {
            let p = c as f32 / n;
            h -= p * p.log2();
        }
    }
    h
}

fn is_hex(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_uuid(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == 5 && parts.iter().map(|p| p.len()).collect::<Vec<_>>() == [8, 4, 4, 4, 12] && parts.iter().all(|p| is_hex(p))
}

fn is_email(s: &str) -> bool {
    let Some(at) = s.find('@') else { return false };
    let (local, domain) = (&s[..at], &s[at + 1..]);
    !local.is_empty() && domain.contains('.') && !domain.ends_with('.') && domain.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

fn looks_like_base64_key(s: &str) -> bool {
    s.len() >= 40
        && s.len() % 4 == 0
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
        && (s.ends_with('=') || s.chars().any(|c| c.is_ascii_uppercase()) && s.chars().any(|c| c.is_ascii_digit()))
}

fn looks_like_jwt(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 3 && parts[0].starts_with("eyJ") && parts.iter().all(|p| p.len() > 8)
}

fn known_token_prefix(s: &str) -> bool {
    const PREFIXES: [&str; 12] = ["sk-", "sk_live_", "sk_test_", "ghp_", "gho_", "github_pat_", "xoxb-", "xoxp-", "AKIA", "AIza", "glpat-", "pk_live_"];
    PREFIXES.iter().any(|p| s.starts_with(p))
}

/// Score 0–100 that a single token is a password/secret. `None` = definitely not.
pub fn password_score(s: &str, keys_are_sensitive: bool) -> Option<u8> {
    let s = s.trim();
    // Known API-token shapes are secrets regardless of length.
    if keys_are_sensitive && (looks_like_jwt(s) || (known_token_prefix(s) && s.len() >= 16 && !s.contains(char::is_whitespace))) {
        return Some(100);
    }
    let n = s.chars().count();
    if !(8..=64).contains(&n) {
        return None;
    }
    if s.contains(char::is_whitespace) {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("ftp://") || lower.starts_with("www.") || lower.starts_with("file:") || lower.starts_with("mailto:") || s.starts_with("\\\\") {
        return None;
    }
    if s.len() >= 3 && s.as_bytes()[1] == b':' && (s.as_bytes()[2] == b'\\' || s.as_bytes()[2] == b'/') && s.as_bytes()[0].is_ascii_alphabetic() {
        return None; // X:\path
    }
    if s.matches('/').count() + s.matches('\\').count() > 1 {
        return None;
    }
    if is_email(s) || is_uuid(s) {
        return None;
    }
    // Dates, times, versions, IPs: mostly digits with separators.
    let digits = s.chars().filter(|c| c.is_ascii_digit()).count();
    if digits * 10 >= n * 6 && s.chars().all(|c| c.is_ascii_digit() || matches!(c, '-' | ':' | 'T' | 'Z' | '.' | '+' | '/' | ' ')) {
        return None;
    }
    if is_hex(s) && matches!(n, 7 | 8 | 12 | 16 | 32 | 40 | 64 | 128) {
        return None; // commit shas, md5/sha1/sha256/sha512
    }
    if s.starts_with('#') && is_hex(&s[1..]) && (n == 7 || n == 9) {
        return None; // hex color
    }
    let classes_lower = s.chars().any(|c| c.is_lowercase());
    let classes_upper = s.chars().any(|c| c.is_uppercase());
    let classes_digit = s.chars().any(|c| c.is_ascii_digit());
    // '-', '_' and '.' are common in identifiers/versions; only stronger symbols count as a class.
    let classes_symbol = s.chars().any(|c| !c.is_alphanumeric() && !matches!(c, '-' | '_' | '.'));
    if s.chars().all(|c| c.is_ascii_digit()) || s.chars().all(|c| c.is_ascii_digit() || c == '+' || c == '-' || c == '.' || c == '(' || c == ')') {
        return None; // numbers, phone numbers, IPs
    }
    if s.chars().all(|c| c.is_alphabetic()) && !(classes_lower && classes_upper && n >= 12) {
        return None; // plain word
    }
    if looks_like_base64_key(s) {
        return if keys_are_sensitive { Some(90) } else { None };
    }
    let classes = [classes_lower, classes_upper, classes_digit, classes_symbol].iter().filter(|&&b| b).count() as i32;
    if classes < 3 {
        return None;
    }
    let h = shannon_entropy(s);
    if h < 2.8 {
        return None;
    }
    let mut score: f32 = 25.0 * (classes - 2) as f32;
    score += ((h - 2.5) * 30.0).clamp(0.0, 40.0);
    score += ((n as f32 - 8.0) * 2.0).clamp(0.0, 20.0);
    // penalties
    let chars: Vec<char> = s.chars().collect();
    let mut run = 1;
    let mut max_repeat = 1;
    for i in 1..chars.len() {
        if chars[i] == chars[i - 1] {
            run += 1;
            max_repeat = max_repeat.max(run);
        } else {
            run = 1;
        }
    }
    if max_repeat >= 4 {
        score -= 20.0;
    }
    let class_of = |c: char| -> u8 {
        if c.is_lowercase() {
            0
        } else if c.is_uppercase() {
            1
        } else if c.is_ascii_digit() {
            2
        } else {
            3
        }
    };
    let mut crun = 1;
    let mut max_crun = 1;
    for i in 1..chars.len() {
        if class_of(chars[i]) == class_of(chars[i - 1]) {
            crun += 1;
            max_crun = max_crun.max(crun);
        } else {
            crun = 1;
        }
    }
    if max_crun >= 6 {
        score -= 15.0;
    }
    Some(score.clamp(0.0, 100.0) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sens(s: &str) -> bool {
        password_score(s, true).is_some_and(|x| x >= 55)
    }

    #[test]
    fn passwords_detected() {
        for p in ["Tr0ub4dor&3", "xK9#mP2$vL5!qR8", "Password123!", "c0rrect-H0rse-B4ttery", "Zq7!pL2@wE9#", "MyS3cret!Pass", "aB3$dE6&gH9*jK1", "g7Hj!2kLm9@Qz"] {
            assert!(sens(p), "should be sensitive: {p}");
        }
    }

    #[test]
    fn not_passwords() {
        for p in [
            "hello world",
            "https://example.com/path?x=1",
            "C:\\Users\\jakub\\file.txt",
            "\\\\server\\share",
            "user@example.com",
            "550e8400-e29b-41d4-a716-446655440000",
            "3f2504e04f8911d39a0c0305e82c3301",
            "a94a8fe5ccb19ba61c4c0873d391e987982fbbd3",
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
            "#ff8800",
            "192.168.0.1",
            "+48 123 456 789",
            "123456789012",
            "Hello",
            "documentation",
            "fn main() {}",
            "some_variable_name",
            "2024-01-15T10:30:00Z",
            "v1.2.3-beta.1",
        ] {
            assert!(!sens(p), "should NOT be sensitive: {p}");
        }
    }

    #[test]
    fn tokens_and_keys() {
        assert_eq!(password_score("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U", true), Some(100));
        assert_eq!(password_score("sk-proj-abcdefghijklmnopqrstuvwxyz0123456789", true), Some(100));
        assert_eq!(password_score("ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef012345", true), Some(100));
        assert!(password_score("dGhpcyBpcyBhIHRlc3Qga2V5IGZvciB0aGUgdW5pdCB0ZXN0cw==", true).is_some_and(|s| s >= 55));
        assert!(password_score("dGhpcyBpcyBhIHRlc3Qga2V5IGZvciB0aGUgdW5pdCB0ZXN0cw==", false).is_none());
    }

    #[test]
    fn evaluate_priority() {
        let mut p = Privacy::default();
        let (s, skip) = evaluate(Some("hello"), None, true, &p);
        assert!(matches!(s, Sensitivity::Auto(SensitiveReason::Format)) && !skip);
        p.formats = SensitiveFormats::Skip;
        assert!(evaluate(Some("hello"), None, true, &p).1);
        let (s, _) = evaluate(Some("hello"), Some("keepassxc.exe"), false, &p);
        assert!(matches!(s, Sensitivity::Auto(SensitiveReason::SourceProcess)));
        let (s, _) = evaluate(Some("hello"), Some("notepad.exe"), false, &p);
        assert_eq!(s, Sensitivity::None);
    }
}
