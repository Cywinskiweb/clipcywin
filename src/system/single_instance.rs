//! Named mutex guard; second instances signal the first and exit.

use crate::util::wide::WStr;
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{RegisterWindowMessageW, SendNotifyMessageW, HWND_BROADCAST};

pub struct InstanceGuard(HANDLE);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub fn activate_message() -> u32 {
    unsafe { RegisterWindowMessageW(WStr::new("Clipcywin.Activate").pcwstr()) }
}

pub fn settings_message() -> u32 {
    unsafe { RegisterWindowMessageW(WStr::new("Clipcywin.OpenSettings").pcwstr()) }
}

pub fn toggle_message() -> u32 {
    unsafe { RegisterWindowMessageW(WStr::new("Clipcywin.Toggle").pcwstr()) }
}

pub fn reload_message() -> u32 {
    unsafe { RegisterWindowMessageW(WStr::new("Clipcywin.Reload").pcwstr()) }
}

/// Returns `None` if another instance already runs (after notifying it).
pub fn acquire() -> Option<InstanceGuard> {
    unsafe {
        let h = CreateMutexW(None, true, WStr::new("Local\\Clipcywin.SingleInstance").pcwstr()).ok()?;
        if windows::Win32::Foundation::GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(h);
            let args: Vec<String> = std::env::args().collect();
            let msg = if args.iter().any(|a| a == "--settings") {
                settings_message()
            } else if args.iter().any(|a| a == "--toggle") {
                toggle_message()
            } else if args.iter().any(|a| a == "--reload") {
                reload_message()
            } else {
                activate_message()
            };
            let _ = SendNotifyMessageW(HWND_BROADCAST, msg, WPARAM(0), LPARAM(0));
            return None;
        }
        Some(InstanceGuard(h))
    }
}

#[allow(dead_code)]
fn _hwnd(_: HWND) {}
