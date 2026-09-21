//! Synthesize Ctrl+V into the foreground window.

use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_KEYBOARD, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_RMENU,
    VK_RSHIFT, VK_RWIN,
};

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    let mut i = INPUT { r#type: INPUT_KEYBOARD, ..Default::default() };
    i.Anonymous.ki.wVk = vk;
    if up {
        i.Anonymous.ki.dwFlags = KEYEVENTF_KEYUP;
    }
    i
}

fn is_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { (GetAsyncKeyState(vk.0 as i32) as u16) & 0x8000 != 0 }
}

/// Release physically held modifiers (except Ctrl) so the target only sees Ctrl+V, then send it.
pub fn send_ctrl_v() {
    let mut seq: Vec<INPUT> = Vec::with_capacity(12);
    for vk in [VK_LWIN, VK_RWIN, VK_LMENU, VK_RMENU, VK_LSHIFT, VK_RSHIFT] {
        if is_down(vk) {
            seq.push(key(vk, true));
        }
    }
    let ctrl_was_down = is_down(VK_CONTROL);
    if !ctrl_was_down {
        seq.push(key(VK_CONTROL, false));
    }
    seq.push(key(VIRTUAL_KEY(0x56), false)); // V
    seq.push(key(VIRTUAL_KEY(0x56), true));
    if !ctrl_was_down {
        seq.push(key(VK_CONTROL, true));
    }
    unsafe {
        SendInput(&seq, std::mem::size_of::<INPUT>() as i32);
    }
}
