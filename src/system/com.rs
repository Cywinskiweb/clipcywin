//! Per-thread COM initialization guard.

use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, COINIT_MULTITHREADED};

pub struct ComGuard(bool);

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() }
        }
    }
}

/// UI thread: OLE (drag & drop) needs OleInitialize, which also initializes COM as STA.
pub fn init_sta() -> ComGuard {
    let ok = unsafe { windows::Win32::System::Ole::OleInitialize(None).is_ok() };
    if !ok {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        return ComGuard(hr.is_ok());
    }
    ComGuard(false)
}

pub fn init_mta() -> ComGuard {
    let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED | COINIT_DISABLE_OLE1DDE) };
    ComGuard(hr.is_ok())
}
