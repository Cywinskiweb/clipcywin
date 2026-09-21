//! Per-window composition surface: DXGI swap chain with premultiplied alpha bound through DirectComposition.

use super::device::Gfx;
use super::theme::Color;
use windows::core::{Interface, Result};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT};
use windows::Win32::Graphics::Direct2D::{
    ID2D1DeviceContext, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
};
use windows::Win32::Graphics::DirectComposition::{IDCompositionTarget, IDCompositionVisual};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    IDXGISurface, IDXGISwapChain1, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
    DXGI_USAGE_RENDER_TARGET_OUTPUT,
};

pub struct Surface {
    swapchain: IDXGISwapChain1,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    pub width_px: u32,
    pub height_px: u32,
    pub dpi: u32,
}

impl Surface {
    pub fn new(gfx: &Gfx, hwnd: HWND, width_px: u32, height_px: u32, dpi: u32) -> Result<Surface> {
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width_px.max(1),
            Height: height_px.max(1),
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: false.into(),
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            Flags: 0,
        };
        unsafe {
            let swapchain = gfx.dxgi_factory.CreateSwapChainForComposition(&gfx.d3d, &desc, None)?;
            let target = gfx.dcomp.CreateTargetForHwnd(hwnd, true)?;
            let visual = gfx.dcomp.CreateVisual()?;
            visual.SetContent(&swapchain)?;
            target.SetRoot(&visual)?;
            gfx.dcomp.Commit()?;
            Ok(Surface { swapchain, _target: target, _visual: visual, width_px: width_px.max(1), height_px: height_px.max(1), dpi })
        }
    }

    pub fn resize(&mut self, width_px: u32, height_px: u32, dpi: u32) -> Result<()> {
        let (w, h) = (width_px.max(1), height_px.max(1));
        self.dpi = dpi;
        if w == self.width_px && h == self.height_px {
            return Ok(());
        }
        unsafe { self.swapchain.ResizeBuffers(0, w, h, DXGI_FORMAT_UNKNOWN, windows::Win32::Graphics::Dxgi::DXGI_SWAP_CHAIN_FLAG(0))? };
        self.width_px = w;
        self.height_px = h;
        Ok(())
    }

    /// Render one frame. `draw` receives the device context with DPI set and layout in DIPs.
    pub fn render(&self, gfx: &Gfx, draw: impl FnOnce(&ID2D1DeviceContext)) -> Result<()> {
        unsafe {
            let surface: IDXGISurface = self.swapchain.GetBuffer(0)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: self.dpi as f32,
                dpiY: self.dpi as f32,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                colorContext: std::mem::ManuallyDrop::new(None),
            };
            let bitmap = gfx.dc.CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
            gfx.dc.SetTarget(&bitmap);
            gfx.dc.SetDpi(self.dpi as f32, self.dpi as f32);
            gfx.dc.BeginDraw();
            gfx.dc.Clear(Some(&Color::rgba(0.0, 0.0, 0.0, 0.0).d2d()));
            draw(&gfx.dc);
            let end = gfx.dc.EndDraw(None, None);
            gfx.dc.SetTarget(None);
            drop(bitmap);
            end?;
            self.swapchain.Present(1, windows::Win32::Graphics::Dxgi::DXGI_PRESENT(0)).ok()?;
            Ok(())
        }
    }

    pub fn size_dip(&self) -> (f32, f32) {
        (self.width_px as f32 * 96.0 / self.dpi as f32, self.height_px as f32 * 96.0 / self.dpi as f32)
    }
}

#[allow(dead_code)]
fn _iface_check(s: &IDXGISwapChain1) -> *mut core::ffi::c_void {
    s.as_raw()
}
