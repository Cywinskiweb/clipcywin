//! Popup menus for items and the tray icon.

use crate::util::wide::WStr;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetForegroundWindow, PostMessageW, SetForegroundWindow, TrackPopupMenuEx, MF_CHECKED, MF_DISABLED,
    MF_GRAYED, MF_SEPARATOR, MF_STRING, TPM_LEFTALIGN, TPM_LEFTBUTTON, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TPM_BOTTOMALIGN, WM_NULL,
};

pub struct MenuItem {
    pub id: u32,
    pub label: String,
    pub checked: bool,
    pub disabled: bool,
    pub separator: bool,
}

impl MenuItem {
    pub fn new(id: u32, label: &str) -> MenuItem {
        MenuItem { id, label: label.into(), checked: false, disabled: false, separator: false }
    }
    pub fn checked(mut self, c: bool) -> MenuItem {
        self.checked = c;
        self
    }
    pub fn disabled(mut self, d: bool) -> MenuItem {
        self.disabled = d;
        self
    }
    pub fn sep() -> MenuItem {
        MenuItem { id: 0, label: String::new(), checked: false, disabled: false, separator: true }
    }
}

/// Show a popup menu at screen coords; returns the chosen id. Restores the previous foreground window.
pub fn show(owner: HWND, items: &[MenuItem], x: i32, y: i32, bottom_align: bool) -> Option<u32> {
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        for it in items {
            if it.separator {
                let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            } else {
                let mut flags = MF_STRING;
                if it.checked {
                    flags |= MF_CHECKED;
                }
                if it.disabled {
                    flags |= MF_DISABLED | MF_GRAYED;
                }
                let label = WStr::new(&it.label);
                let _ = AppendMenuW(menu, flags, it.id as usize, label.pcwstr());
            }
        }
        let prev = GetForegroundWindow();
        let _ = SetForegroundWindow(owner);
        let mut flags = TPM_LEFTALIGN | TPM_LEFTBUTTON | TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY;
        if bottom_align {
            flags |= TPM_BOTTOMALIGN;
        }
        let cmd = TrackPopupMenuEx(menu, flags.0, x, y, owner, None);
        let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        if !prev.is_invalid() && prev != owner {
            let _ = SetForegroundWindow(prev);
        }
        let id = cmd.0 as u32;
        if id == 0 {
            None
        } else {
            Some(id)
        }
    }
}
