//! Window classes, wndproc trampoline and the deferred-operation queue.

use crate::util::wide::WStr;
use std::cell::RefCell;
use std::collections::VecDeque;
use windows::core::Result;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, LoadCursorW, LoadIconW, RegisterClassExW, CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, HICON, IDC_ARROW, WNDCLASSEXW,
};

pub const CLASS_SURFACE: &str = "ClipcywinSurface";
pub const CLASS_CORE: &str = "ClipcywinCore";
pub const CLASS_SETTINGS: &str = "ClipcywinSettings";

thread_local! {
    static DEFERRED: RefCell<VecDeque<Box<dyn FnOnce()>>> = RefCell::new(VecDeque::new());
}

/// Queue an operation to run once the current message handler has released the app borrow.
pub fn defer(f: impl FnOnce() + 'static) {
    DEFERRED.with(|d| d.borrow_mut().push_back(Box::new(f)));
}

/// Run queued operations. Safe to call re-entrantly: ops queued while running are picked up.
pub fn run_deferred() {
    loop {
        let op = DEFERRED.with(|d| d.borrow_mut().pop_front());
        match op {
            Some(f) => f(),
            None => break,
        }
    }
}

pub fn hinstance() -> HINSTANCE {
    unsafe { GetModuleHandleW(None).map(|h| HINSTANCE(h.0)).unwrap_or_default() }
}

pub fn app_icon() -> HICON {
    unsafe { LoadIconW(Some(hinstance()), windows::core::PCWSTR(1 as *const u16)).unwrap_or_default() }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let handled = crate::app::dispatch(hwnd, msg, wparam, lparam);
    run_deferred();
    match handled {
        Some(r) => r,
        None => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn register(name: &str, style: windows::Win32::UI::WindowsAndMessaging::WNDCLASS_STYLES, icon: HICON) -> Result<()> {
    let cls = WStr::new(name);
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style,
        lpfnWndProc: Some(wndproc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance(),
        hIcon: icon,
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_default() },
        hbrBackground: HBRUSH::default(),
        lpszMenuName: windows::core::PCWSTR::null(),
        lpszClassName: cls.pcwstr(),
        hIconSm: icon,
    };
    let atom = unsafe { RegisterClassExW(&wc) };
    if atom == 0 {
        return Err(windows::core::Error::from_win32());
    }
    Ok(())
}

pub fn register_classes() -> Result<()> {
    let icon = app_icon();
    register(CLASS_SURFACE, CS_DBLCLKS, icon)?;
    register(CLASS_CORE, CS_HREDRAW | CS_VREDRAW, icon)?;
    register(CLASS_SETTINGS, CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS, icon)?;
    Ok(())
}

/// Extract signed x/y from an lParam mouse coordinate pair.
pub fn lparam_xy(l: LPARAM) -> (i32, i32) {
    let v = l.0 as u32;
    ((v & 0xffff) as i16 as i32, (v >> 16) as i16 as i32)
}

pub fn wheel_delta(w: WPARAM) -> i32 {
    ((w.0 as u32 >> 16) as u16) as i16 as i32
}
