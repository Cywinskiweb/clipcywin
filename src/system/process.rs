//! Process helpers: exe name lookup, elevation checks.

use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

struct HandleGuard(HANDLE);
impl Drop for HandleGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub fn pid_for_hwnd(hwnd: HWND) -> Option<u32> {
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if pid == 0 {
        None
    } else {
        Some(pid)
    }
}

/// Full image path of a process.
pub fn exe_path_for_pid(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let _g = HandleGuard(h);
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// File name (e.g. `Code.exe`) of a process.
pub fn exe_name_for_pid(pid: u32) -> Option<String> {
    let p = exe_path_for_pid(pid)?;
    Some(p.rsplit(['\\', '/']).next().unwrap_or(&p).to_string())
}

pub fn exe_name_for_hwnd(hwnd: HWND) -> Option<String> {
    exe_name_for_pid(pid_for_hwnd(hwnd)?)
}

fn token_elevated(h: HANDLE) -> Option<bool> {
    unsafe {
        let mut tok = HANDLE::default();
        OpenProcessToken(h, TOKEN_QUERY, &mut tok).ok()?;
        let _g = HandleGuard(tok);
        let mut te = TOKEN_ELEVATION::default();
        let mut ret = 0u32;
        GetTokenInformation(tok, TokenElevation, Some(&mut te as *mut _ as *mut _), std::mem::size_of::<TOKEN_ELEVATION>() as u32, &mut ret).ok()?;
        Some(te.TokenIsElevated != 0)
    }
}

pub fn is_pid_elevated(pid: u32) -> Option<bool> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let _g = HandleGuard(h);
        token_elevated(h)
    }
}

pub fn is_hwnd_elevated(hwnd: HWND) -> Option<bool> {
    is_pid_elevated(pid_for_hwnd(hwnd)?)
}

pub fn is_self_elevated() -> bool {
    unsafe { token_elevated(GetCurrentProcess()).unwrap_or(false) }
}

/// Case-insensitive membership test of an exe name in a list.
pub fn name_in_list(name: &str, list: &[String]) -> bool {
    list.iter().any(|l| l.eq_ignore_ascii_case(name))
}
