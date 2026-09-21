//! HKCU\Software\Microsoft\Windows\CurrentVersion\Run entry.

use crate::util::wide::WStr;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE,
    REG_SZ,
};

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE: &str = "Clipcywin";

fn open(access: windows::Win32::System::Registry::REG_SAM_FLAGS) -> Option<HKEY> {
    let mut key = HKEY::default();
    let ok = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, WStr::new(RUN_KEY).pcwstr(), None, access, &mut key) };
    if ok.is_ok() {
        Some(key)
    } else {
        None
    }
}

pub fn is_enabled() -> bool {
    let Some(key) = open(KEY_READ) else { return false };
    let mut len = 0u32;
    let r = unsafe { RegQueryValueExW(key, WStr::new(VALUE).pcwstr(), None, None, None, Some(&mut len)) };
    unsafe {
        let _ = RegCloseKey(key);
    }
    r.is_ok() && len > 0
}

pub fn set_enabled(enabled: bool) -> bool {
    let Some(key) = open(KEY_WRITE) else { return false };
    let r = unsafe {
        if enabled {
            let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            let cmd = WStr::new(&format!("\"{exe}\" --autostart"));
            let bytes = std::slice::from_raw_parts(cmd.as_slice().as_ptr() as *const u8, cmd.as_slice().len() * 2);
            RegSetValueExW(key, WStr::new(VALUE).pcwstr(), None, REG_SZ, Some(bytes)).is_ok()
        } else {
            let r = RegDeleteValueW(key, WStr::new(VALUE).pcwstr());
            r.is_ok() || r == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND
        }
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    r
}
