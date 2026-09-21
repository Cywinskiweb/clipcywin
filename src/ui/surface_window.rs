//! A borderless popup window backed by a DirectComposition surface.

use crate::gfx::device::Gfx;
use crate::gfx::surface::Surface;
use crate::settings::Backdrop;
use crate::ui::win::{hinstance, CLASS_SURFACE};
use crate::util::wide::WStr;
use windows::core::Result;
use windows::core::BOOL;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMSBT_MAINWINDOW, DWMSBT_NONE, DWMWA_SYSTEMBACKDROP_TYPE,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWCP_ROUND, DWM_WINDOW_CORNER_PREFERENCE,
};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, IsWindowVisible, SetWindowDisplayAffinity, SetWindowPos, HWND_NOTOPMOST, HWND_TOPMOST,
    SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, WDA_EXCLUDEFROMCAPTURE,
    WDA_NONE, WINDOW_EX_STYLE, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corners {
    Square,
    #[allow(dead_code)]
    Round,
}

pub struct SurfaceWindow {
    pub hwnd: HWND,
    pub surface: Surface,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub dpi: u32,
    pub topmost: bool,
    /// Rounded region applied to the window (x, y, w, h, radius) in px; re-applied on resize.
    region: Option<(i32, i32, i32, i32, i32)>,
}

#[repr(C)]
struct AccentPolicy {
    state: u32,
    flags: u32,
    gradient: u32,
    animation_id: u32,
}

#[repr(C)]
struct WindowCompositionAttribData {
    attrib: u32,
    data: *mut core::ffi::c_void,
    size: usize,
}

const ACCENT_DISABLED: u32 = 0;
const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;
const WCA_ACCENT_POLICY: u32 = 19;

type SetWindowCompositionAttributeFn = unsafe extern "system" fn(HWND, *mut WindowCompositionAttribData) -> BOOL;

fn set_window_composition_attribute() -> Option<SetWindowCompositionAttributeFn> {
    unsafe {
        let user32 = windows::Win32::System::LibraryLoader::GetModuleHandleW(windows::core::w!("user32.dll")).ok()?;
        let p = windows::Win32::System::LibraryLoader::GetProcAddress(user32, windows::core::s!("SetWindowCompositionAttribute"))?;
        Some(std::mem::transmute::<_, SetWindowCompositionAttributeFn>(p))
    }
}

/// Acrylic blur-behind with a tint (0xAABBGGRR). `enabled=false` removes it.
fn set_accent(hwnd: HWND, enabled: bool, tint_abgr: u32) {
    let Some(f) = set_window_composition_attribute() else { return };
    let mut policy = AccentPolicy { state: if enabled { ACCENT_ENABLE_ACRYLICBLURBEHIND } else { ACCENT_DISABLED }, flags: 2, gradient: tint_abgr, animation_id: 0 };
    let mut data = WindowCompositionAttribData { attrib: WCA_ACCENT_POLICY, data: &mut policy as *mut _ as *mut _, size: std::mem::size_of::<AccentPolicy>() };
    unsafe {
        let _ = f(hwnd, &mut data);
    }
}

impl SurfaceWindow {
    pub fn create(gfx: &Gfx, topmost: bool, transparent: bool, x: i32, y: i32, w: i32, h: i32, dpi: u32) -> Result<SurfaceWindow> {
        let mut ex = WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP;
        if topmost {
            ex |= WS_EX_TOPMOST;
        }
        if transparent {
            ex |= WS_EX_TRANSPARENT;
        }
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(ex.0),
                WStr::new(CLASS_SURFACE).pcwstr(),
                WStr::new("Clipcywin").pcwstr(),
                WS_POPUP,
                x,
                y,
                w.max(1),
                h.max(1),
                None,
                None,
                Some(hinstance()),
                None,
            )?
        };
        let surface = Surface::new(gfx, hwnd, w.max(1) as u32, h.max(1) as u32, dpi)?;
        Ok(SurfaceWindow { hwnd, surface, x, y, w: w.max(1), h: h.max(1), dpi, topmost, region: None })
    }

    pub fn set_bounds(&mut self, x: i32, y: i32, w: i32, h: i32, dpi: u32) {
        let (w, h) = (w.max(1), h.max(1));
        let moved = x != self.x || y != self.y || w != self.w || h != self.h;
        self.x = x;
        self.y = y;
        self.w = w;
        self.h = h;
        self.dpi = dpi;
        if moved {
            unsafe {
                let _ = SetWindowPos(self.hwnd, None, x, y, w, h, SWP_NOACTIVATE | SWP_NOZORDER);
            }
        }
        if let Err(e) = self.surface.resize(w as u32, h as u32, dpi) {
            log::warn!("surface resize: {e}");
        }
        if moved {
            if let Some((rx, ry, rw, rh, rad)) = self.region {
                // A window-sized region follows the window; an inset one keeps its explicit rect.
                if rw >= self.w.max(rw) - 1 && rx == 0 && ry == 0 {
                    self.set_region(Some((0, 0, self.w, self.h, rad)));
                } else {
                    self.set_region(Some((rx, ry, rw, rh, rad)));
                }
            }
        }
    }

    pub fn region(&self) -> Option<(i32, i32, i32, i32, i32)> {
        self.region
    }

    /// Clip the window (and any blur behind it) to a rounded rectangle given in client px.
    pub fn set_region(&mut self, region: Option<(i32, i32, i32, i32, i32)>) {
        self.region = region;
        unsafe {
            match region {
                Some((x, y, w, h, radius)) => {
                    let d = (radius * 2).max(0);
                    let rgn = windows::Win32::Graphics::Gdi::CreateRoundRectRgn(x, y, x + w + 1, y + h + 1, d, d);
                    let _ = windows::Win32::Graphics::Gdi::SetWindowRgn(self.hwnd, Some(rgn), true);
                }
                None => {
                    let _ = windows::Win32::Graphics::Gdi::SetWindowRgn(self.hwnd, None, true);
                }
            }
        }
    }

    /// Sync cached position after the system moved the window (e.g. caption drag).
    pub fn sync_position(&mut self) {
        let mut r = windows::Win32::Foundation::RECT::default();
        if unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowRect(self.hwnd, &mut r) }.is_ok() {
            self.x = r.left;
            self.y = r.top;
        }
    }

    pub fn show(&self) {
        unsafe {
            let _ = SetWindowPos(self.hwnd, None, 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
        }
    }

    pub fn hide(&self) {
        unsafe {
            let _ = SetWindowPos(self.hwnd, None, 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOMOVE | SWP_NOSIZE | SWP_HIDEWINDOW);
        }
    }

    pub fn is_visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd).as_bool() }
    }

    pub fn set_topmost(&mut self, topmost: bool) {
        self.topmost = topmost;
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(if topmost { HWND_TOPMOST } else { HWND_NOTOPMOST }), 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE);
        }
    }

    /// Bring to the top of its z-band without activating.
    pub fn raise(&self) {
        unsafe {
            let after = if self.topmost { HWND_TOPMOST } else { windows::Win32::UI::WindowsAndMessaging::HWND_TOP };
            let _ = SetWindowPos(self.hwnd, Some(after), 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE);
        }
    }

    pub fn set_capture_excluded(&self, excluded: bool) {
        unsafe {
            if let Err(e) = SetWindowDisplayAffinity(self.hwnd, if excluded { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE }) {
                log::warn!("SetWindowDisplayAffinity: {e}");
            }
        }
    }

    /// `tint` is the acrylic tint as (r, g, b, a) in 0..=1; only used for `Backdrop::Acrylic`.
    pub fn set_backdrop(&self, backdrop: Backdrop, dark: bool, corners: Corners, tint: (f32, f32, f32, f32)) {
        unsafe {
            let dark_v: BOOL = BOOL(dark as i32);
            let _ = DwmSetWindowAttribute(self.hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark_v as *const _ as *const _, 4);
            let corner: DWM_WINDOW_CORNER_PREFERENCE = match corners {
                Corners::Square => DWMWCP_DONOTROUND,
                Corners::Round => DWMWCP_ROUND,
            };
            let _ = DwmSetWindowAttribute(self.hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &corner as *const _ as *const _, 4);
            // Mica goes through the documented DWM backdrop; acrylic uses the accent policy, which also
            // blurs behind windows that are never activated (DWM's transient backdrop falls back to a flat color there).
            let bd = match backdrop {
                Backdrop::Mica => DWMSBT_MAINWINDOW,
                _ => DWMSBT_NONE,
            };
            let m = if backdrop == Backdrop::Mica {
                MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 }
            } else {
                MARGINS { cxLeftWidth: 0, cxRightWidth: 0, cyTopHeight: 0, cyBottomHeight: 0 }
            };
            let _ = DwmExtendFrameIntoClientArea(self.hwnd, &m);
            let _ = DwmSetWindowAttribute(self.hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &bd as *const _ as *const _, 4);
            let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
            let abgr = (to8(tint.3) << 24) | (to8(tint.2) << 16) | (to8(tint.1) << 8) | to8(tint.0);
            set_accent(self.hwnd, backdrop == Backdrop::Acrylic, abgr);
        }
    }

    pub fn recreate_surface(&mut self, gfx: &Gfx) -> Result<()> {
        self.surface = Surface::new(gfx, self.hwnd, self.w as u32, self.h as u32, self.dpi)?;
        Ok(())
    }

    pub fn contains_screen_point(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    pub fn destroy(self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
