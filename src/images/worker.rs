//! Image worker: WIC decode / scale / PNG encode on a background MTA thread.

use crate::clipboard::dib;
use crate::images::fit_within;
use crate::msg::{BgraBuf, RawImage, RawImageData, ToImg, ToUi, UiWaker};
use crate::system::com;
use crate::util::wide::WStr;
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender};
use windows::core::{Interface, GUID};
use windows::Win32::Foundation::GENERIC_WRITE;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat24bppBGR, GUID_WICPixelFormat32bppBGRA,
    GUID_WICPixelFormat32bppPBGRA, IWICBitmapEncoder, IWICBitmapFrameEncode, IWICBitmapSource, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapEncoderNoCache, WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom,
    WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Shell::SHCreateMemStream;

pub fn spawn(rx: Receiver<ToImg>, tx: Sender<ToUi>, waker: UiWaker) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("clipcywin-img".into())
        .spawn(move || {
            let _com = com::init_mta();
            let factory: IWICImagingFactory = match unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) } {
                Ok(f) => f,
                Err(e) => {
                    log::error!("WIC factory: {e}");
                    return;
                }
            };
            let w = Worker { factory, tx, waker };
            while let Ok(m) = rx.recv() {
                if matches!(m, ToImg::Shutdown) {
                    break;
                }
                w.handle(m);
            }
        })
        .expect("spawn image thread")
}

struct Worker {
    factory: IWICImagingFactory,
    tx: Sender<ToUi>,
    waker: UiWaker,
}

impl Worker {
    fn send(&self, m: ToUi) {
        let _ = self.tx.send(m);
        self.waker.wake();
    }

    fn handle(&self, m: ToImg) {
        match m {
            ToImg::Ingest { id, image, path, thumb_px } => match self.ingest(&image, &path, thumb_px) {
                Ok((bytes, has_alpha, thumb)) => self.send(ToUi::ImageIngested { id, path, width: image.width, height: image.height, bytes, has_alpha, thumb }),
                Err(e) => {
                    log::warn!("ingest {id}: {e}");
                    self.send(ToUi::ImageIngestFailed { id });
                }
            },
            ToImg::Thumb { id, path, thumb_px } => match self.decode_file_scaled(&path, thumb_px, true) {
                Ok(t) => self.send(ToUi::ThumbReady { id, thumb: t }),
                Err(e) => {
                    log::warn!("thumb {id}: {e}");
                    self.send(ToUi::ThumbFailed { id });
                }
            },
            ToImg::Preview { id, path, max_px } => match self.decode_file_scaled(&path, max_px, true) {
                Ok(t) => self.send(ToUi::PreviewReady { id, image: t }),
                Err(e) => log::warn!("preview {id}: {e}"),
            },
            ToImg::LoadPng { id, path, reason } => match self.load_png_and_dib(&path) {
                Ok((png, dib)) => self.send(ToUi::PngReady { id, png, dib, reason }),
                Err(e) => log::warn!("load png {id}: {e}"),
            },
            ToImg::DeleteFile(p) => {
                let _ = std::fs::remove_file(p);
            }
            ToImg::SweepOrphans { dir, keep } => {
                if let Ok(rd) = std::fs::read_dir(&dir) {
                    for e in rd.flatten() {
                        let p = e.path();
                        if p.extension().is_some_and(|x| x == "png") && !keep.iter().any(|k| k == &p) {
                            let _ = std::fs::remove_file(&p);
                        }
                    }
                }
            }
            ToImg::Shutdown => {}
        }
    }

    /// Store the original as PNG on disk; return (file bytes, has_alpha, thumbnail).
    fn ingest(&self, image: &RawImage, path: &Path, thumb_px: (u32, u32)) -> windows::core::Result<(u64, bool, BgraBuf)> {
        match &image.data {
            RawImageData::Png(bytes) => {
                std::fs::write(path, bytes).map_err(|e| windows::core::Error::new(windows::core::HRESULT(0x80070000u32 as i32 | e.raw_os_error().unwrap_or(1)), "write png"))?;
                let src = self.decode_bytes(bytes)?;
                let thumb = self.scaled(&src, image.width, image.height, thumb_px, true)?;
                // Detect alpha cheaply from the thumbnail.
                let has_alpha = thumb.pixels.chunks_exact(4).any(|p| p[3] != 255);
                Ok((bytes.len() as u64, has_alpha, thumb))
            }
            RawImageData::Bgra { pixels, has_alpha } => unsafe {
                let stride = image.width * 4;
                let bmp = self.factory.CreateBitmapFromMemory(image.width, image.height, &GUID_WICPixelFormat32bppBGRA, stride, pixels)?;
                let src: IWICBitmapSource = bmp.cast()?;
                self.encode_png(&src, path, image.width, image.height, *has_alpha)?;
                let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                let thumb = self.scaled(&src, image.width, image.height, thumb_px, true)?;
                Ok((bytes, *has_alpha, thumb))
            },
        }
    }

    unsafe fn encode_png(&self, src: &IWICBitmapSource, path: &Path, w: u32, h: u32, has_alpha: bool) -> windows::core::Result<()> {
        let encoder: IWICBitmapEncoder = self.factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        let stream = self.factory.CreateStream()?;
        stream.InitializeFromFilename(WStr::new(&path.to_string_lossy()).pcwstr(), GENERIC_WRITE.0)?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame: Option<IWICBitmapFrameEncode> = None;
        encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
        let frame = frame.ok_or_else(|| windows::core::Error::from_hresult(windows::core::HRESULT(-1)))?;
        frame.Initialize(None)?;
        frame.SetSize(w, h)?;
        let mut fmt: GUID = if has_alpha { GUID_WICPixelFormat32bppBGRA } else { GUID_WICPixelFormat24bppBGR };
        frame.SetPixelFormat(&mut fmt)?;
        frame.WriteSource(src, std::ptr::null())?;
        frame.Commit()?;
        encoder.Commit()?;
        Ok(())
    }

    fn decode_bytes(&self, bytes: &[u8]) -> windows::core::Result<IWICBitmapSource> {
        unsafe {
            let stream = SHCreateMemStream(Some(bytes)).ok_or_else(|| windows::core::Error::from_hresult(windows::core::HRESULT(-1)))?;
            let dec = self.factory.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
            let frame = dec.GetFrame(0)?;
            frame.cast()
        }
    }

    fn decode_file(&self, path: &Path) -> windows::core::Result<(IWICBitmapSource, u32, u32)> {
        unsafe {
            let dec = self.factory.CreateDecoderFromFilename(
                WStr::new(&path.to_string_lossy()).pcwstr(),
                None,
                windows::Win32::Foundation::GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )?;
            let frame = dec.GetFrame(0)?;
            let (mut w, mut h) = (0u32, 0u32);
            frame.GetSize(&mut w, &mut h)?;
            Ok((frame.cast()?, w, h))
        }
    }

    fn decode_file_scaled(&self, path: &Path, max_px: (u32, u32), premultiplied: bool) -> windows::core::Result<BgraBuf> {
        let (src, w, h) = self.decode_file(path)?;
        self.scaled(&src, w, h, max_px, premultiplied)
    }

    /// Scale to fit `max_px` and convert to (P)BGRA.
    fn scaled(&self, src: &IWICBitmapSource, w: u32, h: u32, max_px: (u32, u32), premultiplied: bool) -> windows::core::Result<BgraBuf> {
        unsafe {
            let (tw, th) = fit_within(w, h, max_px.0.max(1), max_px.1.max(1));
            let source: IWICBitmapSource = if (tw, th) != (w, h) {
                let scaler = self.factory.CreateBitmapScaler()?;
                scaler.Initialize(src, tw, th, WICBitmapInterpolationModeFant)?;
                scaler.cast()?
            } else {
                src.clone()
            };
            let conv = self.factory.CreateFormatConverter()?;
            let fmt = if premultiplied { &GUID_WICPixelFormat32bppPBGRA } else { &GUID_WICPixelFormat32bppBGRA };
            conv.Initialize(&source, fmt, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
            let stride = tw * 4;
            let mut pixels = vec![0u8; (stride * th) as usize];
            conv.CopyPixels(std::ptr::null(), stride, &mut pixels)?;
            Ok(BgraBuf { width: tw, height: th, pixels })
        }
    }

    /// Read PNG bytes and build a CF_DIBV5 for clipboard write-back.
    fn load_png_and_dib(&self, path: &Path) -> windows::core::Result<(Vec<u8>, Vec<u8>)> {
        let png = std::fs::read(path).map_err(|_| windows::core::Error::from_hresult(windows::core::HRESULT(0x80070002u32 as i32)))?;
        let (src, w, h) = self.decode_file(path)?;
        let straight = self.scaled(&src, w, h, (w, h), false)?;
        let dibv5 = dib::encode_v5(straight.width, straight.height, &straight.pixels);
        Ok((png, dibv5))
    }
}
