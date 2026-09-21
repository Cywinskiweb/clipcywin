//! UTF-16 helpers for Win32 string APIs.

use windows::core::PCWSTR;

/// Owned, NUL-terminated UTF-16 buffer.
pub struct WStr(Vec<u16>);

impl WStr {
    pub fn new(s: &str) -> Self {
        let mut v: Vec<u16> = s.encode_utf16().collect();
        v.push(0);
        WStr(v)
    }
    pub fn pcwstr(&self) -> PCWSTR {
        PCWSTR(self.0.as_ptr())
    }
    pub fn as_slice(&self) -> &[u16] {
        &self.0
    }
}

/// Convert a NUL-terminated UTF-16 buffer (or the whole slice if no NUL) to String.
pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// Copy a &str into a fixed-size UTF-16 array (NUL-terminated, truncated if needed).
pub fn copy_to_array(dst: &mut [u16], s: &str) {
    let mut n = 0;
    for c in s.encode_utf16() {
        if n + 1 >= dst.len() {
            break;
        }
        dst[n] = c;
        n += 1;
    }
    dst[n] = 0;
}
