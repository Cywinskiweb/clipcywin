//! Hide-on-capture helpers: foreground WinEvent hook, capture-process poller, fullscreen detection.

use crate::msg::{UiWaker, WM_APP_CAPTURE_PROC, WM_APP_FOREGROUND};
use crate::util::wide::from_wide;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN};
use windows::Win32::UI::WindowsAndMessaging::{EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS};

static FG_TARGET: AtomicIsize = AtomicIsize::new(0);

unsafe extern "system" fn fg_proc(_hook: HWINEVENTHOOK, _event: u32, hwnd: HWND, _id_object: i32, _id_child: i32, _thread: u32, _time: u32) {
    let t = FG_TARGET.load(Ordering::Acquire);
    if t != 0 {
        UiWaker::from_raw(t).post(WM_APP_FOREGROUND, hwnd.0 as usize, 0);
    }
}

/// Must be called on a thread with a message loop (the UI thread).
pub fn start_foreground_hook(target: HWND) -> HWINEVENTHOOK {
    FG_TARGET.store(target.0 as isize, Ordering::Release);
    unsafe { SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(fg_proc), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS) }
}

pub fn stop_foreground_hook(h: HWINEVENTHOOK) {
    if !h.is_invalid() {
        unsafe {
            let _ = UnhookWinEvent(h);
        }
    }
}

/// True when the foreground app runs full screen (game, video, presentation).
pub fn is_fullscreen_foreground() -> bool {
    match unsafe { SHQueryUserNotificationState() } {
        Ok(s) => s == QUNS_BUSY || s == QUNS_RUNNING_D3D_FULL_SCREEN || s == QUNS_PRESENTATION_MODE,
        Err(_) => false,
    }
}

/// Snapshot running process names and test membership in `list`.
pub fn any_process_running(list: &[String]) -> bool {
    if list.is_empty() {
        return false;
    }
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return false };
        let mut pe = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut found = false;
        if Process32FirstW(snap, &mut pe).is_ok() {
            loop {
                let name = from_wide(&pe.szExeFile);
                if list.iter().any(|l| l.eq_ignore_ascii_case(&name)) {
                    found = true;
                    break;
                }
                if Process32NextW(snap, &mut pe).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
        found
    }
}

pub struct Poller {
    pub enabled: Arc<AtomicBool>,
    pub list: Arc<Mutex<Vec<String>>>,
}

/// Background poller (2 s) that posts WM_APP_CAPTURE_PROC on state changes while enabled.
pub fn spawn_poller(waker: UiWaker, enabled: bool, list: Vec<String>) -> Poller {
    let p = Poller { enabled: Arc::new(AtomicBool::new(enabled)), list: Arc::new(Mutex::new(list)) };
    let en = p.enabled.clone();
    let li = p.list.clone();
    let _ = std::thread::Builder::new().name("clipcywin-poll".into()).spawn(move || {
        let mut last = false;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(2000));
            if !en.load(Ordering::Relaxed) {
                if last {
                    last = false;
                    waker.post(WM_APP_CAPTURE_PROC, 0, 0);
                }
                continue;
            }
            let list = li.lock().map(|l| l.clone()).unwrap_or_default();
            let running = any_process_running(&list);
            if running != last {
                last = running;
                waker.post(WM_APP_CAPTURE_PROC, running as usize, 0);
            }
        }
    });
    p
}
