//! Registered clipboard format ids (resolved once).

use crate::util::wide::WStr;
use std::sync::OnceLock;
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;

#[derive(Clone, Copy, Debug)]
pub struct Formats {
    pub png: u32,
    pub exclude_from_monitor: u32,
    pub can_include_in_history: u32,
    pub can_upload_to_cloud: u32,
    pub self_set: u32,
}

static FORMATS: OnceLock<Formats> = OnceLock::new();

pub fn formats() -> &'static Formats {
    FORMATS.get_or_init(|| unsafe {
        let reg = |s: &str| RegisterClipboardFormatW(WStr::new(s).pcwstr());
        Formats {
            png: reg("PNG"),
            exclude_from_monitor: reg("ExcludeClipboardContentFromMonitorProcessing"),
            can_include_in_history: reg("CanIncludeInClipboardHistory"),
            can_upload_to_cloud: reg("CanUploadToCloudClipboard"),
            self_set: reg("Clipcywin.SelfSet"),
        }
    })
}
