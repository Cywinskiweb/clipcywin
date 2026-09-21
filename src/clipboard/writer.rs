//! Put items back on the clipboard (UI thread).

use crate::clipboard::formats::formats;
use crate::clipboard::open_with_retry;
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{EmptyClipboard, GetClipboardSequenceNumber, SetClipboardData};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{CF_DIB, CF_DIBV5, CF_HDROP, CF_UNICODETEXT};

unsafe fn set_bytes(format: u32, bytes: &[u8]) -> bool {
    let h = match GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) {
        Ok(h) => h,
        Err(_) => return false,
    };
    let p = GlobalLock(h);
    if p.is_null() {
        let _ = GlobalFree(Some(h));
        return false;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), p as *mut u8, bytes.len());
    let _ = GlobalUnlock(h);
    if SetClipboardData(format, Some(HANDLE(h.0))).is_err() {
        let _ = GlobalFree(Some(h));
        return false;
    }
    true
}

unsafe fn set_markers(skip_history: bool) {
    let f = formats();
    set_bytes(f.self_set, &1u32.to_le_bytes());
    if skip_history {
        set_bytes(f.can_include_in_history, &0u32.to_le_bytes());
        set_bytes(f.can_upload_to_cloud, &0u32.to_le_bytes());
    }
}

/// Returns the clipboard sequence number after the write.
pub fn put_text(owner: HWND, text: &str, skip_history: bool) -> Option<u32> {
    let _g = open_with_retry(Some(owner), 8)?;
    unsafe {
        EmptyClipboard().ok()?;
        let mut w: Vec<u16> = text.encode_utf16().collect();
        w.push(0);
        let bytes = std::slice::from_raw_parts(w.as_ptr() as *const u8, w.len() * 2);
        if !set_bytes(CF_UNICODETEXT.0 as u32, bytes) {
            return None;
        }
        set_markers(skip_history);
        Some(GetClipboardSequenceNumber())
    }
}

/// `dib_v5` is a packed BITMAPV5HEADER DIB (32-bpp, BI_BITFIELDS, bottom-up).
pub fn put_image(owner: HWND, png: &[u8], dib_v5: &[u8], skip_history: bool) -> Option<u32> {
    let _g = open_with_retry(Some(owner), 8)?;
    unsafe {
        EmptyClipboard().ok()?;
        let f = formats();
        let mut ok = false;
        if !png.is_empty() {
            ok |= set_bytes(f.png, png);
        }
        if dib_v5.len() > 124 {
            ok |= set_bytes(CF_DIBV5.0 as u32, dib_v5);
            // CF_DIB: same 32-bpp bottom-up pixels with a 40-byte BI_RGB header.
            let v1 = dib_v1_from_v5(dib_v5);
            ok |= set_bytes(CF_DIB.0 as u32, &v1);
        }
        if !ok {
            return None;
        }
        set_markers(skip_history);
        Some(GetClipboardSequenceNumber())
    }
}

pub fn put_files(owner: HWND, paths: &[impl AsRef<Path>], skip_history: bool) -> Option<u32> {
    let _g = open_with_retry(Some(owner), 8)?;
    unsafe {
        EmptyClipboard().ok()?;
        // DROPFILES { pFiles=20, pt{0,0}, fNC=0, fWide=1 } + double-NUL terminated UTF-16 list
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(&20u32.to_le_bytes());
        buf.extend_from_slice(&[0u8; 8]);
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&1u32.to_le_bytes());
        for p in paths {
            for c in p.as_ref().to_string_lossy().encode_utf16() {
                buf.extend_from_slice(&c.to_le_bytes());
            }
            buf.extend_from_slice(&[0, 0]);
        }
        buf.extend_from_slice(&[0, 0]);
        if !set_bytes(CF_HDROP.0 as u32, &buf) {
            return None;
        }
        set_markers(skip_history);
        Some(GetClipboardSequenceNumber())
    }
}

/// Put text and/or a file list on the clipboard in one go (shelf "copy all").
pub fn put_multi(owner: HWND, text: Option<&str>, files: &[PathBuf], skip_history: bool) -> Option<u32> {
    let _g = open_with_retry(Some(owner), 8)?;
    unsafe {
        EmptyClipboard().ok()?;
        let mut ok = false;
        if let Some(t) = text {
            let mut w: Vec<u16> = t.encode_utf16().collect();
            w.push(0);
            let bytes = std::slice::from_raw_parts(w.as_ptr() as *const u8, w.len() * 2);
            ok |= set_bytes(CF_UNICODETEXT.0 as u32, bytes);
        }
        if !files.is_empty() {
            ok |= set_bytes(CF_HDROP.0 as u32, &hdrop_bytes(files));
        }
        if !ok {
            return None;
        }
        set_markers(skip_history);
        Some(GetClipboardSequenceNumber())
    }
}

fn hdrop_bytes(paths: &[PathBuf]) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(&20u32.to_le_bytes());
    buf.extend_from_slice(&[0u8; 8]);
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&1u32.to_le_bytes());
    for p in paths {
        for c in p.to_string_lossy().encode_utf16() {
            buf.extend_from_slice(&c.to_le_bytes());
        }
        buf.extend_from_slice(&[0, 0]);
    }
    buf.extend_from_slice(&[0, 0]);
    buf
}

pub fn clear(owner: HWND) -> bool {
    let Some(_g) = open_with_retry(Some(owner), 4) else { return false };
    unsafe { EmptyClipboard().is_ok() }
}

fn dib_v1_from_v5(v5: &[u8]) -> Vec<u8> {
    let hdr = u32::from_le_bytes([v5[0], v5[1], v5[2], v5[3]]) as usize;
    let mut out = Vec::with_capacity(40 + v5.len() - hdr);
    let mut h = [0u8; 40];
    h[0..4].copy_from_slice(&40u32.to_le_bytes());
    h[4..12].copy_from_slice(&v5[4..12]); // width, height
    h[12..16].copy_from_slice(&v5[12..16]); // planes, bpp
    // compression BI_RGB (0), size image
    h[20..24].copy_from_slice(&v5[20..24]);
    out.extend_from_slice(&h);
    out.extend_from_slice(&v5[hdr..]);
    out
}

#[allow(dead_code)]
fn _hglobal_type_check(h: HGLOBAL) -> HGLOBAL {
    h
}
