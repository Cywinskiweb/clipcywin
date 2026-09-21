//! Low-level keyboard hook on a dedicated thread. The callback touches only atomics.

use crate::msg::{WM_APP_CAPTURE_KEY, WM_APP_CHORD, WM_APP_HOTKEY, WM_APP_TOGGLE_BAR};
use crate::settings::{Chord, MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_WIN};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU8, Ordering};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PostMessageW, PostThreadMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
    HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

// bits 0-3: required modifiers; bit 4: hotkeys enabled; bit 5: capture-key detection;
// bits 8-15: toggle chord modifiers (0 = disabled); bits 16-23: toggle chord vk
static CFG: AtomicU32 = AtomicU32::new(0);
static MODS_DOWN: AtomicU8 = AtomicU8::new(0);
static CHORD_HELD: AtomicBool = AtomicBool::new(false);
static SWALLOWED_VK: AtomicU32 = AtomicU32::new(0);
static TARGET: AtomicIsize = AtomicIsize::new(0);
static HOOK_TID: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy, Debug)]
pub struct HookConfig {
    pub modifiers: u8,
    pub hotkeys_enabled: bool,
    pub capture_keys: bool,
    pub toggle: Option<Chord>,
}

impl HookConfig {
    pub fn from_settings(s: &crate::settings::Settings) -> HookConfig {
        HookConfig {
            modifiers: s.hotkeys.modifiers & 0xf,
            hotkeys_enabled: s.hotkeys.enabled,
            capture_keys: s.hide.on_capture_keys,
            toggle: if s.hotkeys.toggle_chord_enabled { Some(s.hotkeys.toggle_chord) } else { None },
        }
    }
}

pub fn configure(c: &HookConfig) {
    let mut v = (c.modifiers & 0xf) as u32;
    if c.hotkeys_enabled {
        v |= 1 << 4;
    }
    if c.capture_keys {
        v |= 1 << 5;
    }
    if let Some(t) = c.toggle {
        v |= ((t.modifiers & 0xff) as u32) << 8;
        v |= (t.vk as u32) << 16;
    }
    CFG.store(v, Ordering::Release);
}

const VK_LSHIFT: u32 = 0xA0;
const VK_RSHIFT: u32 = 0xA1;
const VK_LCONTROL: u32 = 0xA2;
const VK_RCONTROL: u32 = 0xA3;
const VK_LMENU: u32 = 0xA4;
const VK_RMENU: u32 = 0xA5;
const VK_LWIN: u32 = 0x5B;
const VK_RWIN: u32 = 0x5C;
const VK_SNAPSHOT: u32 = 0x2C;

fn modifier_bit(vk: u32) -> u8 {
    match vk {
        VK_LWIN | VK_RWIN => MOD_WIN,
        VK_LCONTROL | VK_RCONTROL => MOD_CTRL,
        VK_LMENU | VK_RMENU => MOD_ALT,
        VK_LSHIFT | VK_RSHIFT => MOD_SHIFT,
        _ => 0,
    }
}

fn digit_slot(vk: u32) -> Option<u32> {
    match vk {
        0x30..=0x39 => Some(vk - 0x30),
        0x60..=0x69 => Some(vk - 0x60),
        _ => None,
    }
}

fn post(msg: u32, w: usize) {
    let t = TARGET.load(Ordering::Acquire);
    if t != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(t as *mut _)), msg, WPARAM(w), LPARAM(0));
        }
    }
}

/// Inject a no-op key-up so the shell does not treat a lone Win press as "open Start".
fn mask_win_key() {
    let mut inp = INPUT { r#type: INPUT_KEYBOARD, ..Default::default() };
    inp.Anonymous.ki.wVk = VIRTUAL_KEY(0xFF);
    inp.Anonymous.ki.dwFlags = KEYEVENTF_KEYUP;
    unsafe {
        SendInput(&[inp], std::mem::size_of::<INPUT>() as i32);
    }
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(None, code, wparam, lparam);
    }
    let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
    if kb.flags.0 & LLKHF_INJECTED.0 != 0 {
        return CallNextHookEx(None, code, wparam, lparam);
    }
    let msg = wparam.0 as u32;
    let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
    let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
    let vk = kb.vkCode;
    let cfg = CFG.load(Ordering::Acquire);
    let required = (cfg & 0xf) as u8;
    let hotkeys_on = cfg & (1 << 4) != 0;
    let capture_on = cfg & (1 << 5) != 0;
    let toggle_mods = ((cfg >> 8) & 0xff) as u8;
    let toggle_vk = (cfg >> 16) & 0xff;

    let mbit = modifier_bit(vk);
    if mbit != 0 {
        let prev = MODS_DOWN.load(Ordering::Relaxed);
        let now = if down { prev | mbit } else if up { prev & !mbit } else { prev };
        if now != prev {
            MODS_DOWN.store(now, Ordering::Relaxed);
            let was = prev & required == required && required != 0;
            let is = now & required == required && required != 0;
            if was != is {
                CHORD_HELD.store(is, Ordering::Release);
                post(WM_APP_CHORD, is as usize);
            }
        }
        return CallNextHookEx(None, code, wparam, lparam);
    }

    let mods = MODS_DOWN.load(Ordering::Relaxed);

    // Swallow the key-up of a digit we consumed.
    if up && SWALLOWED_VK.load(Ordering::Relaxed) == vk {
        SWALLOWED_VK.store(0, Ordering::Relaxed);
        return LRESULT(1);
    }

    if down {
        if hotkeys_on && required != 0 && mods == required {
            if let Some(slot) = digit_slot(vk) {
                SWALLOWED_VK.store(vk, Ordering::Relaxed);
                if required & MOD_WIN != 0 {
                    mask_win_key();
                }
                post(WM_APP_HOTKEY, slot as usize);
                return LRESULT(1);
            }
        }
        if toggle_mods != 0 && mods == toggle_mods && vk == toggle_vk {
            SWALLOWED_VK.store(vk, Ordering::Relaxed);
            if toggle_mods & MOD_WIN != 0 {
                mask_win_key();
            }
            post(WM_APP_TOGGLE_BAR, 0);
            return LRESULT(1);
        }
        if capture_on {
            let win = mods & MOD_WIN != 0;
            let shift = mods & MOD_SHIFT != 0;
            let alt = mods & MOD_ALT != 0;
            let is_capture = vk == VK_SNAPSHOT
                || (win && shift && vk == 0x53) // Win+Shift+S
                || (win && alt && vk == 0x52) // Win+Alt+R
                || (win && vk == 0x47); // Win+G
            if is_capture {
                post(WM_APP_CAPTURE_KEY, 0);
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

pub fn start(target: HWND, cfg: &HookConfig) {
    configure(cfg);
    TARGET.store(target.0 as isize, Ordering::Release);
    if HOOK_TID.load(Ordering::Acquire) != 0 {
        return;
    }
    let _ = std::thread::Builder::new().name("clipcywin-hook".into()).spawn(|| unsafe {
        let hook: HHOOK = match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0) {
            Ok(h) => h,
            Err(e) => {
                log::error!("SetWindowsHookExW: {e}");
                return;
            }
        };
        HOOK_TID.store(windows::Win32::System::Threading::GetCurrentThreadId(), Ordering::Release);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = UnhookWindowsHookEx(hook);
        HOOK_TID.store(0, Ordering::Release);
    });
}

pub fn stop() {
    let tid = HOOK_TID.load(Ordering::Acquire);
    if tid != 0 {
        unsafe {
            let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
}

#[allow(dead_code)]
fn _flags(_: KEYBD_EVENT_FLAGS) {}
