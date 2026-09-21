//! SHAppBarMessage wrapper for the docked bar mode.

use crate::msg::WM_APP_APPBAR;
use crate::settings::Edge;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::Shell::{
    SHAppBarMessage, ABE_BOTTOM, ABE_TOP, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, ABM_WINDOWPOSCHANGED, APPBARDATA,
};

pub struct AppBar {
    hwnd: HWND,
    registered: bool,
    pub rect: RECT,
}

fn abd(hwnd: HWND) -> APPBARDATA {
    APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, hWnd: hwnd, uCallbackMessage: WM_APP_APPBAR, ..Default::default() }
}

/// Full-width strip of `thick` px at `edge` of `mon`, as the initial ABM_QUERYPOS request.
pub fn strip_rect(mon: &RECT, edge: Edge, thick: i32) -> RECT {
    match edge {
        Edge::Top => RECT { left: mon.left, top: mon.top, right: mon.right, bottom: mon.top + thick },
        Edge::Bottom => RECT { left: mon.left, top: mon.bottom - thick, right: mon.right, bottom: mon.bottom },
    }
}

/// Re-normalize a rect the shell adjusted so it keeps exactly `thick` px.
pub fn normalize(mut rc: RECT, edge: Edge, thick: i32) -> RECT {
    match edge {
        Edge::Top => rc.bottom = rc.top + thick,
        Edge::Bottom => rc.top = rc.bottom - thick,
    }
    rc
}

impl AppBar {
    pub fn new(hwnd: HWND) -> AppBar {
        AppBar { hwnd, registered: false, rect: RECT::default() }
    }

    pub fn register(&mut self) -> bool {
        if self.registered {
            return true;
        }
        let mut d = abd(self.hwnd);
        let ok = unsafe { SHAppBarMessage(ABM_NEW, &mut d) } != 0;
        self.registered = ok;
        ok
    }

    /// Ask the shell for a non-overlapping position and reserve it. Returns the final rect.
    pub fn set_pos(&mut self, mon: &RECT, edge: Edge, thick: i32) -> RECT {
        if !self.registered && !self.register() {
            return strip_rect(mon, edge, thick);
        }
        let mut d = abd(self.hwnd);
        d.uEdge = match edge {
            Edge::Top => ABE_TOP,
            Edge::Bottom => ABE_BOTTOM,
        };
        d.rc = strip_rect(mon, edge, thick);
        unsafe {
            SHAppBarMessage(ABM_QUERYPOS, &mut d);
        }
        d.rc = normalize(d.rc, edge, thick);
        // Keep horizontal extent on our monitor even if the shell shifted it.
        d.rc.left = mon.left;
        d.rc.right = mon.right;
        unsafe {
            SHAppBarMessage(ABM_SETPOS, &mut d);
        }
        self.rect = d.rc;
        d.rc
    }

    pub fn window_pos_changed(&self) {
        if self.registered {
            let mut d = abd(self.hwnd);
            unsafe {
                SHAppBarMessage(ABM_WINDOWPOSCHANGED, &mut d);
            }
        }
    }

    pub fn remove(&mut self) {
        if self.registered {
            let mut d = abd(self.hwnd);
            unsafe {
                SHAppBarMessage(ABM_REMOVE, &mut d);
            }
            self.registered = false;
        }
    }
}

impl Drop for AppBar {
    fn drop(&mut self) {
        self.remove();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_and_normalize() {
        let mon = RECT { left: 0, top: 0, right: 2560, bottom: 1440 };
        let r = strip_rect(&mon, Edge::Bottom, 56);
        assert_eq!((r.top, r.bottom), (1384, 1440));
        // shell pushed it up above the taskbar (76 px): bottom becomes 1364
        let shifted = RECT { left: 0, top: 1364 - 56 - 10, right: 2560, bottom: 1364 };
        let n = normalize(shifted, Edge::Bottom, 56);
        assert_eq!((n.top, n.bottom), (1308, 1364));
        let t = normalize(RECT { left: 0, top: 0, right: 100, bottom: 200 }, Edge::Top, 40);
        assert_eq!((t.top, t.bottom), (0, 40));
    }
}
