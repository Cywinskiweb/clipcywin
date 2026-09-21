//! Clipboard capture worker thread.

use crate::clipboard::dib;
use crate::clipboard::formats::formats;
use crate::clipboard::open_with_retry;
use crate::msg::{RawCapture, RawContent, RawImage, RawImageData, ToClip, ToUi, UiWaker};
use crate::privacy::detect;
use crate::settings::Privacy;
use crate::system::process;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{GetClipboardData, GetClipboardOwner, GetClipboardSequenceNumber, IsClipboardFormatAvailable};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{CF_DIB, CF_DIBV5, CF_HDROP, CF_UNICODETEXT};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

#[derive(Clone, Debug)]
pub struct CaptureConfig {
    pub text: bool,
    pub images: bool,
    pub files: bool,
    pub max_image_bytes: usize,
    pub max_text_bytes: usize,
    pub privacy: Privacy,
}

impl CaptureConfig {
    pub fn from_settings(s: &crate::settings::Settings) -> CaptureConfig {
        CaptureConfig {
            text: s.capture.text,
            images: s.capture.images,
            files: s.capture.files,
            max_image_bytes: s.capture.max_image_mb as usize * 1024 * 1024,
            max_text_bytes: s.capture.max_text_kb as usize * 1024,
            privacy: s.privacy.clone(),
        }
    }
}

pub fn spawn(rx: Receiver<ToClip>, tx: Sender<ToUi>, waker: UiWaker, cfg: Arc<Mutex<CaptureConfig>>) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("clipcywin-clip".into())
        .spawn(move || loop {
            let mut msg = match rx.recv() {
                Ok(m) => m,
                Err(_) => return,
            };
            // Coalesce bursts: only the newest capture request matters.
            loop {
                match rx.try_recv() {
                    Ok(m) => msg = m,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            match msg {
                ToClip::Shutdown => return,
                ToClip::Capture { seq } => {
                    let cfg = cfg.lock().map(|c| c.clone()).unwrap_or_else(|p| p.into_inner().clone());
                    match capture(seq, &cfg) {
                        Some(c) => {
                            let _ = tx.send(ToUi::Captured(c));
                        }
                        None => {
                            let _ = tx.send(ToUi::CaptureFailed);
                        }
                    }
                    waker.wake();
                }
            }
        })
        .expect("spawn clipboard thread")
}

/// Copy the bytes of a global-memory clipboard handle.
unsafe fn read_global(h: HANDLE, max: usize) -> Option<Vec<u8>> {
    if h.is_invalid() {
        return None;
    }
    let hg = HGLOBAL(h.0);
    let size = GlobalSize(hg);
    if size == 0 || size > max {
        return None;
    }
    let p = GlobalLock(hg);
    if p.is_null() {
        return None;
    }
    let v = std::slice::from_raw_parts(p as *const u8, size).to_vec();
    let _ = GlobalUnlock(hg);
    Some(v)
}

fn capture(seq: u32, cfg: &CaptureConfig) -> Option<RawCapture> {
    let f = formats();
    let _guard = open_with_retry(None, 8)?;
    unsafe {
        // Even if the clipboard changed again since we were asked, the newest content is what the user has.
        let _ = (seq, GetClipboardSequenceNumber());

        let self_set = IsClipboardFormatAvailable(f.self_set).is_ok();
        let exclude = IsClipboardFormatAvailable(f.exclude_from_monitor).is_ok();
        let mut history_blocked = false;
        if IsClipboardFormatAvailable(f.can_include_in_history).is_ok() {
            if let Ok(h) = GetClipboardData(f.can_include_in_history) {
                if let Some(b) = read_global(h, 16) {
                    if b.len() >= 4 && u32::from_le_bytes([b[0], b[1], b[2], b[3]]) == 0 {
                        history_blocked = true;
                    }
                }
            }
        }
        let format_sensitive = exclude || history_blocked;

        let source_exe = {
            let owner = GetClipboardOwner().unwrap_or_default();
            if owner.is_invalid() {
                None
            } else {
                process::exe_name_for_hwnd(owner)
            }
        };

        let content = if cfg.files && IsClipboardFormatAvailable(CF_HDROP.0 as u32).is_ok() {
            let h = GetClipboardData(CF_HDROP.0 as u32).ok()?;
            let hdrop = HDROP(h.0);
            let count = DragQueryFileW(hdrop, u32::MAX, None);
            let mut paths = Vec::with_capacity(count as usize);
            let mut buf = vec![0u16; 32768];
            for i in 0..count {
                let n = DragQueryFileW(hdrop, i, Some(&mut buf));
                if n > 0 {
                    paths.push(PathBuf::from(String::from_utf16_lossy(&buf[..n as usize])));
                }
            }
            if paths.is_empty() {
                return None;
            }
            RawContent::Files(paths)
        } else if cfg.text && IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_ok() {
            let h = GetClipboardData(CF_UNICODETEXT.0 as u32).ok()?;
            let bytes = read_global(h, cfg.max_text_bytes * 2 + 2)?;
            let u16s: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            let end = u16s.iter().position(|&c| c == 0).unwrap_or(u16s.len());
            let text = String::from_utf16_lossy(&u16s[..end]);
            if text.is_empty() || text.len() > cfg.max_text_bytes {
                return None;
            }
            RawContent::Text(text)
        } else if cfg.images && IsClipboardFormatAvailable(f.png).is_ok() {
            let h = GetClipboardData(f.png).ok()?;
            let png = read_global(h, cfg.max_image_bytes)?;
            let (w, hgt) = dib::png_dimensions(&png)?;
            RawContent::Image(RawImage { width: w, height: hgt, data: RawImageData::Png(png) })
        } else if cfg.images && (IsClipboardFormatAvailable(CF_DIBV5.0 as u32).is_ok() || IsClipboardFormatAvailable(CF_DIB.0 as u32).is_ok()) {
            let fmt = if IsClipboardFormatAvailable(CF_DIBV5.0 as u32).is_ok() { CF_DIBV5.0 as u32 } else { CF_DIB.0 as u32 };
            let h = GetClipboardData(fmt).ok()?;
            let bytes = read_global(h, cfg.max_image_bytes)?;
            let d = match dib::decode(&bytes) {
                Ok(d) => d,
                Err(e) => {
                    log::warn!("dib decode: {e}");
                    return None;
                }
            };
            RawContent::Image(RawImage { width: d.width, height: d.height, data: RawImageData::Bgra { pixels: d.pixels, has_alpha: d.has_alpha } })
        } else {
            return None;
        };

        let text_ref = match &content {
            RawContent::Text(t) => Some(t.as_str()),
            _ => None,
        };
        let (sensitivity, skip) = detect::evaluate(text_ref, source_exe.as_deref(), format_sensitive, &cfg.privacy);
        Some(RawCapture { content, source_exe, sensitivity, self_set, skip })
    }
}

