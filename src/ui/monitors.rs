//! Monitor enumeration, DPI and taskbar geometry.

use crate::settings::MonitorChoice;
use crate::util::wide::from_wide;
use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

#[derive(Clone, Debug, PartialEq)]
pub struct MonitorInfo {
    pub handle: isize,
    pub index: u32,
    pub device: String,
    pub rect: RECT,
    pub work: RECT,
    pub primary: bool,
    pub dpi: u32,
}

impl MonitorInfo {
    pub fn width(&self) -> i32 {
        self.rect.right - self.rect.left
    }
    pub fn height(&self) -> i32 {
        self.rect.bottom - self.rect.top
    }
}

unsafe extern "system" fn enum_proc(hmon: HMONITOR, _hdc: HDC, _rc: *mut RECT, lp: LPARAM) -> BOOL {
    let list = &mut *(lp.0 as *mut Vec<MonitorInfo>);
    let mut mi = MONITORINFOEXW::default();
    mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(hmon, &mut mi as *mut _ as *mut MONITORINFO).as_bool() {
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        list.push(MonitorInfo {
            handle: hmon.0 as isize,
            index: list.len() as u32,
            device: from_wide(&mi.szDevice),
            rect: mi.monitorInfo.rcMonitor,
            work: mi.monitorInfo.rcWork,
            primary: (mi.monitorInfo.dwFlags & 1) != 0,
            dpi: dx.max(48),
        });
    }
    BOOL(1)
}

pub fn enumerate() -> Vec<MonitorInfo> {
    let mut list: Vec<MonitorInfo> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(enum_proc), LPARAM(&mut list as *mut _ as isize));
    }
    if list.is_empty() {
        list.push(MonitorInfo {
            handle: 0,
            index: 0,
            device: "\\\\.\\DISPLAY1".into(),
            rect: RECT { left: 0, top: 0, right: 1920, bottom: 1080 },
            work: RECT { left: 0, top: 0, right: 1920, bottom: 1040 },
            primary: true,
            dpi: 96,
        });
    }
    list
}

pub fn choose(list: &[MonitorInfo], choice: &MonitorChoice) -> MonitorInfo {
    let primary = || list.iter().find(|m| m.primary).or(list.first()).cloned().unwrap();
    match choice {
        MonitorChoice::Primary => primary(),
        MonitorChoice::Index(i) => list.get(*i as usize).cloned().unwrap_or_else(primary),
        MonitorChoice::Device(d) => list.iter().find(|m| m.device.eq_ignore_ascii_case(d)).cloned().unwrap_or_else(primary),
    }
}

pub fn rect_w(r: &RECT) -> i32 {
    r.right - r.left
}
pub fn rect_h(r: &RECT) -> i32 {
    r.bottom - r.top
}
