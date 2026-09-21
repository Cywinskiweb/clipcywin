//! Custom window messages, timer ids and inter-thread message enums.

use windows::Win32::UI::WindowsAndMessaging::WM_APP;

pub const WM_APP_UI_WAKE: u32 = WM_APP + 1; // drain channels
pub const WM_APP_HOTKEY: u32 = WM_APP + 2; // wParam = slot 0..9
pub const WM_APP_CHORD: u32 = WM_APP + 3; // wParam = 1 held / 0 released
pub const WM_APP_CAPTURE_KEY: u32 = WM_APP + 4; // screenshot/record key seen
pub const WM_APP_CAPTURE_PROC: u32 = WM_APP + 5; // wParam = 1 capture process running / 0 none
pub const WM_APP_TRAY: u32 = WM_APP + 6; // tray icon callback
pub const WM_APP_APPBAR: u32 = WM_APP + 7; // ABM_NEW callback
pub const WM_APP_TOGGLE_BAR: u32 = WM_APP + 8; // toggle chord pressed
pub const WM_APP_FOREGROUND: u32 = WM_APP + 9; // foreground window changed (WinEvent), wParam = hwnd
pub const WM_APP_SETTINGS_MSG: u32 = WM_APP + 12; // settings window posted a JSON message (drained from queue)
pub const WM_APP_SETTINGS_CLOSED: u32 = WM_APP + 13;

pub const TIMER_ANIM: usize = 1;
pub const TIMER_EXPIRY: usize = 2;
pub const TIMER_AUTOHIDE: usize = 3;
pub const TIMER_REVEAL: usize = 4;
pub const TIMER_PASTE: usize = 5;
pub const TIMER_CAPTURE_HIDE: usize = 6;
pub const TIMER_ISLAND_HOLD: usize = 7;
pub const TIMER_LIST_CLOSE: usize = 8;
pub const TIMER_SHELF_CLOSE: usize = 11;
pub const TIMER_FS_CHECK: usize = 12;

use crate::model::item::{ItemId, Sensitivity};
use std::path::PathBuf;

/// Raw image data as captured.
pub struct RawImage {
    pub width: u32,
    pub height: u32,
    pub data: RawImageData,
}

pub enum RawImageData {
    /// PNG bytes as provided by the source.
    Png(Vec<u8>),
    /// 32-bpp top-down BGRA (straight alpha).
    Bgra { pixels: Vec<u8>, has_alpha: bool },
}

/// Result of a clipboard capture on the worker.
pub struct RawCapture {
    pub content: RawContent,
    pub source_exe: Option<String>,
    pub sensitivity: Sensitivity,
    /// The clipboard carried our own marker format (we set it ourselves).
    pub self_set: bool,
    /// Sensitive clipboard formats said "do not record".
    pub skip: bool,
}

pub enum RawContent {
    Text(String),
    Image(RawImage),
    Files(Vec<PathBuf>),
}

/// Premultiplied BGRA pixel buffer, stride = width*4.
pub struct BgraBuf {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Messages to the UI thread.
pub enum ToUi {
    Captured(RawCapture),
    CaptureFailed,
    ImageIngested { id: ItemId, path: PathBuf, width: u32, height: u32, bytes: u64, has_alpha: bool, thumb: BgraBuf },
    ImageIngestFailed { id: ItemId },
    ThumbReady { id: ItemId, thumb: BgraBuf },
    ThumbFailed { id: ItemId },
    PreviewReady { id: ItemId, image: BgraBuf },
    PngReady { id: ItemId, png: Vec<u8>, dib: Vec<u8>, reason: PngReason },
    Loaded(Vec<crate::db::Row>),
    ShelvesLoaded { shelves: Vec<crate::db::ShelfRow>, items: Vec<crate::db::ShelfItemRow> },
    DbError(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PngReason {
    Paste,
    CopyOnly,
}

/// Messages to the image worker.
pub enum ToImg {
    Ingest { id: ItemId, image: RawImage, path: PathBuf, thumb_px: (u32, u32) },
    Thumb { id: ItemId, path: PathBuf, thumb_px: (u32, u32) },
    Preview { id: ItemId, path: PathBuf, max_px: (u32, u32) },
    LoadPng { id: ItemId, path: PathBuf, reason: PngReason },
    DeleteFile(PathBuf),
    SweepOrphans { dir: PathBuf, keep: Vec<PathBuf> },
    Shutdown,
}

/// Messages to the DB worker.
pub enum ToDb {
    Insert(crate::db::Row),
    SetPinned { id: ItemId, pinned: bool },
    Touch { id: ItemId, created_at: i64 },
    Delete(ItemId),
    Clear { keep_pinned: bool },
    LoadAll { limit: usize },
    Trim { max: usize },
    ShelfUpsert(crate::db::ShelfRow),
    ShelfDelete(i64),
    ShelfItemInsert(crate::db::ShelfItemRow),
    ShelfItemDelete(i64),
    ShelfClear(i64),
    LoadShelves,
    Shutdown,
}

/// Messages to the clipboard capture worker.
pub enum ToClip {
    Capture { seq: u32 },
    Shutdown,
}

/// Send-able handle that pokes the UI thread's message loop.
#[derive(Clone, Copy)]
pub struct UiWaker {
    hwnd: isize,
}

impl UiWaker {
    pub fn new(hwnd: windows::Win32::Foundation::HWND) -> UiWaker {
        UiWaker { hwnd: hwnd.0 as isize }
    }
    #[cfg(test)]
    pub fn none() -> UiWaker {
        UiWaker { hwnd: 0 }
    }
    pub fn from_raw(hwnd: isize) -> UiWaker {
        UiWaker { hwnd }
    }
    pub fn wake(&self) {
        self.post(WM_APP_UI_WAKE, 0, 0);
    }
    pub fn post(&self, msg: u32, wparam: usize, lparam: isize) {
        if self.hwnd == 0 {
            return;
        }
        unsafe {
            use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
            let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                Some(HWND(self.hwnd as *mut _)),
                msg,
                WPARAM(wparam),
                LPARAM(lparam),
            );
        }
    }
}
