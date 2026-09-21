//! D3D11 + DXGI + Direct2D + DirectWrite + DirectComposition device bundle (UI thread).

use windows::core::{Interface, Result};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Device, ID2D1DeviceContext, ID2D1Factory1, ID2D1SolidColorBrush, D2D1_DEVICE_CONTEXT_OPTIONS_NONE,
    D2D1_FACTORY_TYPE_SINGLE_THREADED,
};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_SINGLETHREADED, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::DirectComposition::{DCompositionCreateDevice, IDCompositionDevice};
use windows::Win32::Graphics::DirectWrite::{DWriteCreateFactory, IDWriteFactory, DWRITE_FACTORY_TYPE_SHARED};
use windows::Win32::Graphics::Dxgi::{IDXGIDevice, IDXGIFactory2};

use super::theme::Color;

pub struct Gfx {
    pub d3d: ID3D11Device,
    _dxgi_device: IDXGIDevice,
    pub dxgi_factory: IDXGIFactory2,
    pub d2d_factory: ID2D1Factory1,
    _d2d_device: ID2D1Device,
    pub dc: ID2D1DeviceContext,
    pub dwrite: IDWriteFactory,
    pub dcomp: IDCompositionDevice,
    /// Scratch brush; set the color before each use.
    pub brush: ID2D1SolidColorBrush,
    /// Bumped on every device rebuild so dependents can drop stale resources.
    pub generation: u32,
    pub warp: bool,
}

impl Gfx {
    pub fn new(prefer_warp: bool) -> Result<Gfx> {
        let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let d2d_factory: ID2D1Factory1 = unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let mut g = Gfx::create_device_parts(&d2d_factory, prefer_warp)?;
        g.dwrite = dwrite;
        g.warp = prefer_warp;
        Ok(g)
    }

    fn create_device_parts(d2d_factory: &ID2D1Factory1, prefer_warp: bool) -> Result<Gfx> {
        let flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_SINGLETHREADED;
        let mut d3d: Option<ID3D11Device> = None;
        let mut r = Ok(());
        if !prefer_warp {
            r = unsafe { D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), flags, None, D3D11_SDK_VERSION, Some(&mut d3d), None, None) };
        }
        if prefer_warp || r.is_err() || d3d.is_none() {
            if !prefer_warp {
                log::warn!("hardware D3D11 unavailable, using WARP");
            }
            r = unsafe { D3D11CreateDevice(None, D3D_DRIVER_TYPE_WARP, HMODULE::default(), flags, None, D3D11_SDK_VERSION, Some(&mut d3d), None, None) };
        }
        r?;
        let d3d = d3d.unwrap();
        let dxgi_device: IDXGIDevice = d3d.cast()?;
        let adapter = unsafe { dxgi_device.GetAdapter()? };
        let dxgi_factory: IDXGIFactory2 = unsafe { adapter.GetParent()? };
        let d2d_device = unsafe { d2d_factory.CreateDevice(&dxgi_device)? };
        let dc = unsafe { d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)? };
        let dcomp: IDCompositionDevice = unsafe { DCompositionCreateDevice(&dxgi_device)? };
        let brush = unsafe { dc.CreateSolidColorBrush(&Color::rgba(1.0, 1.0, 1.0, 1.0).d2d(), None)? };
        // Placeholder dwrite; replaced by caller (DWrite is device independent).
        let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        Ok(Gfx { d3d, _dxgi_device: dxgi_device, dxgi_factory, d2d_factory: d2d_factory.clone(), _d2d_device: d2d_device, dc, dwrite, dcomp, brush, generation: 0, warp: prefer_warp })
    }

    /// Rebuild all device-dependent objects after device loss.
    pub fn recreate(&mut self) -> Result<()> {
        let mut g = Gfx::create_device_parts(&self.d2d_factory, self.warp)?;
        g.dwrite = self.dwrite.clone();
        g.generation = self.generation.wrapping_add(1);
        *self = g;
        Ok(())
    }

    /// Set the scratch brush color and return it for drawing.
    pub fn brush(&self, c: Color) -> &ID2D1SolidColorBrush {
        unsafe { self.brush.SetColor(&c.d2d()) };
        &self.brush
    }
}

/// True when an HRESULT means the graphics device must be rebuilt.
pub fn is_device_lost(hr: windows::core::HRESULT) -> bool {
    const D2DERR_RECREATE_TARGET: i32 = 0x8899000Cu32 as i32;
    const DXGI_ERROR_DEVICE_REMOVED: i32 = 0x887A0005u32 as i32;
    const DXGI_ERROR_DEVICE_RESET: i32 = 0x887A0007u32 as i32;
    matches!(hr.0, D2DERR_RECREATE_TARGET | DXGI_ERROR_DEVICE_REMOVED | DXGI_ERROR_DEVICE_RESET)
}
