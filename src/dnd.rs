//! OLE drag & drop: drop targets for our windows and a drag source for shelf/bar items.

use crate::util::wide::WStr;
use std::path::PathBuf;
use std::sync::OnceLock;
use windows::core::{implement, Ref, Result, BOOL, HRESULT};
use windows::Win32::Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, HGLOBAL, HWND, POINTL, S_OK};
use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{
    DoDragDrop, IDropSource, IDropSource_Impl, IDropTarget, IDropTarget_Impl, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop, CF_DIBV5,
    CF_HDROP, CF_UNICODETEXT, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{DragQueryFileW, SHCreateDataObject, HDROP};

const MK_LBUTTON: u32 = 0x0001;

/// Which of our windows a drag is over.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Zone {
    Bar,
    Shelf,
}

/// Data carried by a drag, in the forms we understand.
#[derive(Default, Debug)]
pub struct DragPayload {
    /// History item ids (negative = shelf entry ids) when the drag started inside Clipcywin.
    pub internal: Vec<i64>,
    pub files: Vec<PathBuf>,
    pub text: Option<String>,
    pub png: Option<Vec<u8>>,
    pub dib: Option<Vec<u8>>,
}

impl DragPayload {
    pub fn is_empty(&self) -> bool {
        self.internal.is_empty() && self.files.is_empty() && self.text.is_none() && self.png.is_none() && self.dib.is_none()
    }
}

static INTERNAL_FORMAT: OnceLock<u16> = OnceLock::new();
static PNG_FORMAT: OnceLock<u16> = OnceLock::new();

fn internal_format() -> u16 {
    *INTERNAL_FORMAT.get_or_init(|| unsafe { RegisterClipboardFormatW(WStr::new("Clipcywin.Items").pcwstr()) as u16 })
}
fn png_format() -> u16 {
    *PNG_FORMAT.get_or_init(|| unsafe { RegisterClipboardFormatW(WStr::new("PNG").pcwstr()) as u16 })
}

fn fmt(cf: u16) -> FORMATETC {
    FORMATETC { cfFormat: cf, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 }
}

unsafe fn read_hglobal(h: HGLOBAL, max: usize) -> Option<Vec<u8>> {
    let size = GlobalSize(h);
    if size == 0 || size > max {
        return None;
    }
    let p = GlobalLock(h);
    if p.is_null() {
        return None;
    }
    let v = std::slice::from_raw_parts(p as *const u8, size).to_vec();
    let _ = GlobalUnlock(h);
    Some(v)
}

fn get_bytes(obj: &IDataObject, cf: u16, max: usize) -> Option<Vec<u8>> {
    unsafe {
        let f = fmt(cf);
        let mut med = obj.GetData(&f).ok()?;
        let out = if med.tymed == TYMED_HGLOBAL.0 as u32 { read_hglobal(med.u.hGlobal, max) } else { None };
        ReleaseStgMedium(&mut med);
        out
    }
}

fn has(obj: &IDataObject, cf: u16) -> bool {
    unsafe { obj.QueryGetData(&fmt(cf)).is_ok() }
}

/// True when the data object carries anything we can put on a shelf.
pub fn accepts(obj: &IDataObject) -> bool {
    has(obj, internal_format()) || has(obj, CF_HDROP.0) || has(obj, CF_UNICODETEXT.0) || has(obj, png_format()) || has(obj, CF_DIBV5.0)
}

/// Extract everything useful from a dropped data object.
pub fn payload_from(obj: &IDataObject) -> DragPayload {
    let mut p = DragPayload::default();
    if let Some(b) = get_bytes(obj, internal_format(), 1 << 20) {
        p.internal = b.chunks_exact(8).map(|c| i64::from_le_bytes(c.try_into().unwrap())).collect();
        return p; // internal drags carry everything else by reference
    }
    if has(obj, CF_HDROP.0) {
        unsafe {
            let f = fmt(CF_HDROP.0);
            if let Ok(mut med) = obj.GetData(&f) {
                if med.tymed == TYMED_HGLOBAL.0 as u32 {
                    let hdrop = HDROP(med.u.hGlobal.0);
                    let n = DragQueryFileW(hdrop, u32::MAX, None);
                    let mut buf = vec![0u16; 32768];
                    for i in 0..n {
                        let len = DragQueryFileW(hdrop, i, Some(&mut buf));
                        if len > 0 {
                            p.files.push(PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])));
                        }
                    }
                }
                ReleaseStgMedium(&mut med);
            }
        }
    }
    if p.files.is_empty() {
        if let Some(b) = get_bytes(obj, CF_UNICODETEXT.0, 16 << 20) {
            let u: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            let end = u.iter().position(|&c| c == 0).unwrap_or(u.len());
            let s = String::from_utf16_lossy(&u[..end]);
            if !s.is_empty() {
                p.text = Some(s);
            }
        }
        if p.text.is_none() {
            if let Some(b) = get_bytes(obj, png_format(), 256 << 20) {
                p.png = Some(b);
            } else if let Some(b) = get_bytes(obj, CF_DIBV5.0, 256 << 20) {
                p.dib = Some(b);
            }
        }
    }
    p
}

// ------------------------------------------------------------------ drop target

#[implement(IDropTarget)]
pub struct DropTarget {
    zone: Zone,
}

impl DropTarget {
    pub fn register(hwnd: HWND, zone: Zone) -> Result<IDropTarget> {
        let t: IDropTarget = DropTarget { zone }.into();
        unsafe { RegisterDragDrop(hwnd, &t)? };
        Ok(t)
    }
}

pub fn revoke(hwnd: HWND) {
    unsafe {
        let _ = RevokeDragDrop(hwnd);
    }
}

impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(&self, pdataobj: Ref<'_, IDataObject>, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, pdweffect: *mut DROPEFFECT) -> Result<()> {
        let ok = pdataobj.as_ref().is_some_and(accepts);
        unsafe { *pdweffect = if ok { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
        let zone = self.zone;
        crate::app::with_app(|a| a.on_drag_enter(zone, ok, pt.x, pt.y));
        Ok(())
    }
    fn DragOver(&self, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, pdweffect: *mut DROPEFFECT) -> Result<()> {
        let zone = self.zone;
        let ok = crate::app::with_app(|a| a.on_drag_over(zone, pt.x, pt.y)).unwrap_or(false);
        unsafe { *pdweffect = if ok { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        let zone = self.zone;
        crate::app::with_app(|a| a.on_drag_leave(zone));
        Ok(())
    }
    fn Drop(&self, pdataobj: Ref<'_, IDataObject>, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, pdweffect: *mut DROPEFFECT) -> Result<()> {
        let payload = pdataobj.as_ref().map(payload_from).unwrap_or_default();
        let zone = self.zone;
        let ok = crate::app::with_app(|a| a.on_drop(zone, payload, pt.x, pt.y)).unwrap_or(false);
        unsafe { *pdweffect = if ok { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
        Ok(())
    }
}

// ------------------------------------------------------------------ drag source

#[implement(IDropSource)]
struct DropSource;

impl IDropSource_Impl for DropSource_Impl {
    fn QueryContinueDrag(&self, fescapepressed: BOOL, grfkeystate: MODIFIERKEYS_FLAGS) -> HRESULT {
        if fescapepressed.as_bool() {
            DRAGDROP_S_CANCEL
        } else if grfkeystate.0 & MK_LBUTTON == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }
    fn GiveFeedback(&self, _dweffect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// What a drag started from Clipcywin offers to other applications.
#[derive(Default)]
pub struct DragData {
    pub internal: Vec<i64>,
    pub text: Option<String>,
    pub files: Vec<PathBuf>,
    pub png: Option<Vec<u8>>,
    pub dib: Option<Vec<u8>>,
}

unsafe fn hglobal_from(bytes: &[u8]) -> Option<HGLOBAL> {
    let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)).ok()?;
    let p = GlobalLock(h);
    if p.is_null() {
        return None;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), p as *mut u8, bytes.len());
    let _ = GlobalUnlock(h);
    Some(h)
}

unsafe fn set_bytes(obj: &IDataObject, cf: u16, bytes: &[u8]) {
    if let Some(h) = hglobal_from(bytes) {
        let f = fmt(cf);
        let med = STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, u: STGMEDIUM_0 { hGlobal: h }, pUnkForRelease: std::mem::ManuallyDrop::new(None) };
        let _ = obj.SetData(&f, &med, true);
    }
}

fn hdrop_bytes(paths: &[PathBuf]) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(&20u32.to_le_bytes());
    buf.extend_from_slice(&[0u8; 8]);
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&1u32.to_le_bytes());
    for p in paths {
        for c in p.to_string_lossy().encode_utf16() {
            buf.extend_from_slice(&c.to_le_bytes());
        }
        buf.extend_from_slice(&[0, 0]);
    }
    buf.extend_from_slice(&[0, 0]);
    buf
}

/// Run a modal OLE drag. Must be called on the UI thread while the app is not borrowed.
pub fn start_drag(data: DragData) -> bool {
    unsafe {
        let obj: IDataObject = match SHCreateDataObject(None, None, None) {
            Ok(o) => o,
            Err(e) => {
                log::warn!("SHCreateDataObject: {e}");
                return false;
            }
        };
        if !data.internal.is_empty() {
            let mut b = Vec::with_capacity(data.internal.len() * 8);
            for id in &data.internal {
                b.extend_from_slice(&id.to_le_bytes());
            }
            set_bytes(&obj, internal_format(), &b);
        }
        if !data.files.is_empty() {
            set_bytes(&obj, CF_HDROP.0, &hdrop_bytes(&data.files));
        }
        if let Some(t) = &data.text {
            let mut w: Vec<u16> = t.encode_utf16().collect();
            w.push(0);
            let bytes = std::slice::from_raw_parts(w.as_ptr() as *const u8, w.len() * 2);
            set_bytes(&obj, CF_UNICODETEXT.0, bytes);
        }
        if let Some(p) = &data.png {
            set_bytes(&obj, png_format(), p);
        }
        if let Some(d) = &data.dib {
            set_bytes(&obj, CF_DIBV5.0, d);
        }
        let src: IDropSource = DropSource.into();
        let mut effect = DROPEFFECT_NONE;
        let hr = DoDragDrop(&obj, &src, DROPEFFECT_COPY, &mut effect);
        hr == DRAGDROP_S_DROP && effect != DROPEFFECT_NONE
    }
}
