//! Content hashing for dedupe.

use sha2::{Digest, Sha256};
use std::path::Path;

pub type Hash = [u8; 32];

pub fn hash_text(s: &str) -> Hash {
    let mut h = Sha256::new();
    h.update(b"text\0");
    h.update(s.as_bytes());
    h.finalize().into()
}

pub fn hash_bytes(tag: &[u8], bytes: &[u8]) -> Hash {
    let mut h = Sha256::new();
    h.update(tag);
    h.update(b"\0");
    h.update(bytes);
    h.finalize().into()
}

pub fn hash_files(paths: &[impl AsRef<Path>]) -> Hash {
    let mut h = Sha256::new();
    h.update(b"files\0");
    for p in paths {
        h.update(p.as_ref().to_string_lossy().to_lowercase().as_bytes());
        h.update(b"\n");
    }
    h.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distinct_kinds_differ() {
        assert_ne!(hash_text("abc"), hash_bytes(b"png", b"abc"));
        assert_eq!(hash_text("abc"), hash_text("abc"));
    }
}
