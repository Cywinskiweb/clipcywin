//! Clipboard item model.

use crate::model::hash::Hash;
use std::path::PathBuf;
use std::time::Instant;
use zeroize::Zeroizing;

pub type ItemId = i64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Text = 0,
    Image = 1,
    Files = 2,
}

impl ItemKind {
    pub fn from_i64(v: i64) -> Option<ItemKind> {
        match v {
            0 => Some(ItemKind::Text),
            1 => Some(ItemKind::Image),
            2 => Some(ItemKind::Files),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum ClipContent {
    Text {
        text: Zeroizing<String>,
        /// First ~200 chars, whitespace collapsed. Empty for sensitive items.
        snippet: String,
        chars: u32,
        lines: u32,
        looks_like_code: bool,
    },
    Image {
        /// PNG path on disk; `None` while the image worker is still ingesting.
        path: Option<PathBuf>,
        width: u32,
        height: u32,
        bytes: u64,
        has_alpha: bool,
    },
    Files {
        paths: Vec<PathBuf>,
    },
}

impl ClipContent {
    pub fn kind(&self) -> ItemKind {
        match self {
            ClipContent::Text { .. } => ItemKind::Text,
            ClipContent::Image { .. } => ItemKind::Image,
            ClipContent::Files { .. } => ItemKind::Files,
        }
    }

    pub fn text(text: String) -> ClipContent {
        let chars = text.chars().count() as u32;
        let lines = text.lines().count().max(1) as u32;
        let looks_like_code = looks_like_code(&text);
        let snippet = make_snippet(&text, 200);
        ClipContent::Text { text: Zeroizing::new(text), snippet, chars, lines, looks_like_code }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SensitiveReason {
    Format,
    SourceProcess,
    Heuristic { score: u8 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sensitivity {
    None,
    Auto(SensitiveReason),
    Manual,
}

impl Sensitivity {
    pub fn is_sensitive(&self) -> bool {
        !matches!(self, Sensitivity::None)
    }
}

#[derive(Clone, Debug)]
pub struct ClipItem {
    pub id: ItemId,
    pub content: ClipContent,
    pub created_at_ms: i64,
    pub hash: Hash,
    pub source_exe: Option<String>,
    pub sensitivity: Sensitivity,
    pub pinned: bool,
    pub revealed_until: Option<Instant>,
    pub expires_at: Option<Instant>,
    /// Bumped whenever the displayed text of the item changes (cache invalidation).
    pub layout_gen: u32,
}

impl ClipItem {
    pub fn kind(&self) -> ItemKind {
        self.content.kind()
    }

    pub fn is_masked(&self, now: Instant) -> bool {
        self.sensitivity.is_sensitive() && !self.revealed_until.is_some_and(|t| t > now)
    }

    /// Text used for the bar snippet (masked when sensitive).
    pub fn display_snippet(&self, now: Instant) -> Option<&str> {
        match &self.content {
            ClipContent::Text { snippet, text, .. } => {
                if self.is_masked(now) {
                    None
                } else if snippet.is_empty() {
                    Some(text.as_str())
                } else {
                    Some(snippet)
                }
            }
            _ => None,
        }
    }
}

/// Collapse whitespace and truncate to `max` chars.
pub fn make_snippet(text: &str, max: usize) -> String {
    let mut out = String::with_capacity(max.min(text.len()) + 1);
    let mut last_space = true;
    let mut n = 0;
    for c in text.chars() {
        if n >= max {
            out.push('…');
            break;
        }
        if c.is_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
                n += 1;
            }
        } else {
            out.push(c);
            last_space = false;
            n += 1;
        }
    }
    out.trim_end().to_string()
}

/// Cheap heuristic: braces/semicolons/indentation density suggests code.
pub fn looks_like_code(text: &str) -> bool {
    let sample: &str = if text.len() > 4000 { &text[..text.char_indices().nth(4000).map(|(i, _)| i).unwrap_or(text.len())] } else { text };
    let lines: Vec<&str> = sample.lines().collect();
    if lines.len() < 2 {
        return false;
    }
    let mut indented = 0;
    let mut codey = 0;
    for l in &lines {
        if l.starts_with("    ") || l.starts_with('\t') {
            indented += 1;
        }
        let t = l.trim_end();
        if t.ends_with(';') || t.ends_with('{') || t.ends_with('}') || t.contains("=>") || t.contains("->") || t.starts_with("fn ") || t.starts_with("def ") || t.starts_with("import ") || t.starts_with("#include") || t.starts_with("const ") || t.starts_with("let ") || t.starts_with("var ") {
            codey += 1;
        }
    }
    let n = lines.len() as f32;
    (indented as f32 / n) > 0.3 || (codey as f32 / n) > 0.3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_collapses() {
        assert_eq!(make_snippet("  a \n\n b\tc  ", 100), "a b c");
        assert_eq!(make_snippet("abcdef", 3), "abc…");
    }

    #[test]
    fn code_detection() {
        assert!(looks_like_code("fn main() {\n    println!(\"hi\");\n}\n"));
        assert!(!looks_like_code("This is a sentence.\nAnd another one here."));
        assert!(!looks_like_code("single line"));
    }
}
