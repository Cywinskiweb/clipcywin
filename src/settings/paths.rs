//! Data directory layout under %LOCALAPPDATA%\Clipcywin.

use std::path::PathBuf;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_LocalAppData, SHGetKnownFolderPath, KF_FLAG_CREATE};

#[derive(Clone, Debug)]
pub struct Paths {
    pub root: PathBuf,
    pub db: PathBuf,
    pub settings: PathBuf,
    pub images: PathBuf,
    pub shelves: PathBuf,
    pub webview: PathBuf,
    pub logs: PathBuf,
}

impl Paths {
    pub fn resolve() -> Paths {
        let base = local_app_data().unwrap_or_else(|| {
            std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
        });
        let root = base.join("Clipcywin");
        // One-time migration from the pre-rename data folder.
        let old = base.join("Clipbar");
        if !root.exists() && old.exists() {
            let _ = std::fs::rename(&old, &root);
        }
        let p = Paths {
            db: root.join("history.db"),
            settings: root.join("settings.json"),
            images: root.join("images"),
            shelves: root.join("shelves"),
            webview: root.join("webview"),
            logs: root.join("logs"),
            root,
        };
        let _ = std::fs::create_dir_all(&p.images);
        let _ = std::fs::create_dir_all(&p.shelves);
        let _ = std::fs::create_dir_all(&p.logs);
        p
    }
}

fn local_app_data() -> Option<PathBuf> {
    unsafe {
        let pw = SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_CREATE, None).ok()?;
        let s = pw.to_string().ok();
        CoTaskMemFree(Some(pw.0 as *const _));
        s.map(PathBuf::from)
    }
}
