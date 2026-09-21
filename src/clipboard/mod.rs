//! Clipboard capture and write-back.

pub mod dib;
pub mod formats;
pub mod reader;
pub mod writer;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::DataExchange::{CloseClipboard, OpenClipboard};

/// RAII guard for an open clipboard.
pub struct ClipboardGuard(());

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

/// Open the clipboard with exponential backoff (other listeners race us right after an update).
pub fn open_with_retry(owner: Option<HWND>, attempts: u32) -> Option<ClipboardGuard> {
    let mut delay_ms = 5u64;
    for i in 0..attempts {
        if unsafe { OpenClipboard(owner) }.is_ok() {
            return Some(ClipboardGuard(()));
        }
        if i + 1 < attempts {
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            delay_ms = (delay_ms * 2).min(200);
        }
    }
    None
}
