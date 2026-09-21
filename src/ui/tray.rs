//! Notification-area icon.

use crate::msg::WM_APP_TRAY;
use crate::ui::win::app_icon;
use crate::util::wide::copy_to_array;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
};

const TRAY_ID: u32 = 1;

pub struct Tray {
    hwnd: HWND,
    added: bool,
}

impl Tray {
    pub fn new(hwnd: HWND) -> Tray {
        Tray { hwnd, added: false }
    }

    fn base(&self) -> NOTIFYICONDATAW {
        let mut d = NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: self.hwnd, uID: TRAY_ID, ..Default::default() };
        d.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        d
    }

    pub fn add(&mut self, tip: &str) {
        let mut d = self.base();
        d.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
        d.uCallbackMessage = WM_APP_TRAY;
        d.hIcon = app_icon();
        copy_to_array(&mut d.szTip, tip);
        unsafe {
            if self.added {
                let _ = Shell_NotifyIconW(NIM_DELETE, &d);
            }
            self.added = Shell_NotifyIconW(NIM_ADD, &d).as_bool();
            if self.added {
                let _ = Shell_NotifyIconW(NIM_SETVERSION, &d);
            }
        }
    }

    pub fn set_tip(&self, tip: &str) {
        if !self.added {
            return;
        }
        let mut d = self.base();
        d.uFlags = NIF_TIP | NIF_SHOWTIP;
        copy_to_array(&mut d.szTip, tip);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
        }
    }

    pub fn balloon(&self, title: &str, text: &str) {
        if !self.added {
            return;
        }
        let mut d = self.base();
        d.uFlags = NIF_INFO;
        d.dwInfoFlags = NIIF_INFO;
        copy_to_array(&mut d.szInfoTitle, title);
        copy_to_array(&mut d.szInfo, text);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
        }
    }

    pub fn remove(&mut self) {
        if self.added {
            let d = self.base();
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &d);
            }
            self.added = false;
        }
    }
}
