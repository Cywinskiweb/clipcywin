//! JSON protocol between the settings page and the app.

use crate::app::App;
use crate::gfx::theme;
use crate::settings::Settings;
use crate::ui::monitors;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Incoming {
    GetSettings,
    SetSettings { data: Settings },
    Reset,
    Revert,
    OpenDataFolder,
    OpenLogs,
    ClearHistory { keep_pinned: bool },
    GetStats,
    Close,
    ShelfCreate { name: String, color: u32 },
    ShelfUpdate { id: i64, name: Option<String>, color: Option<u32> },
    ShelfDelete { id: i64 },
    ShelfActivate { id: i64 },
    OpenUrl { url: String },
}

#[derive(Serialize)]
struct ShelfMeta {
    id: i64,
    name: String,
    color: String,
    count: usize,
    active: bool,
}

#[derive(Serialize)]
struct MonitorMeta {
    index: u32,
    device: String,
    primary: bool,
    width: i32,
    height: i32,
    dpi: u32,
}

#[derive(Serialize)]
struct Meta {
    monitors: Vec<MonitorMeta>,
    accent: String,
    version: &'static str,
    webview_version: String,
    is_elevated: bool,
    dark: bool,
    shelves: Vec<ShelfMeta>,
    support_url: &'static str,
    repo_url: &'static str,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Outgoing<'a> {
    Settings { data: &'a Settings, meta: Meta },
    Applied { ok: bool, error: Option<String> },
    Stats { items: usize, pinned: usize, sensitive: usize, images: usize, thumb_bytes: usize, db_bytes: u64, image_bytes: u64 },
}

fn meta(app: &App) -> Meta {
    let mons = monitors::enumerate()
        .into_iter()
        .map(|m| MonitorMeta { index: m.index, device: m.device.clone(), primary: m.primary, width: m.width(), height: m.height(), dpi: m.dpi })
        .collect();
    Meta {
        monitors: mons,
        accent: format!("#{:06x}", theme::system_accent()),
        version: env!("CARGO_PKG_VERSION"),
        webview_version: crate::settings_ui::window::runtime_version().unwrap_or_default(),
        is_elevated: crate::system::process::is_self_elevated(),
        dark: app.theme.dark,
        support_url: crate::app::SUPPORT_URL,
        repo_url: crate::app::REPO_URL,
        shelves: app
            .shelves
            .shelves
            .iter()
            .map(|s| ShelfMeta { id: s.id, name: s.name.clone(), color: format!("#{:06x}", s.color), count: s.entries.len(), active: s.id == app.shelves.active })
            .collect(),
    }
}

fn send(app: &App, msg: &Outgoing) {
    if let Some(w) = &app.settings_win {
        if let Ok(json) = serde_json::to_string(msg) {
            w.post_json(json);
        }
    }
}

pub fn send_settings(app: &App) {
    send(app, &Outgoing::Settings { data: &app.settings, meta: meta(app) });
}

fn dir_size(p: &std::path::Path) -> u64 {
    std::fs::read_dir(p).map(|rd| rd.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum()).unwrap_or(0)
}

pub fn send_stats(app: &App) {
    let items = app.history.len();
    let pinned = app.history.iter().filter(|i| i.pinned).count();
    let sensitive = app.history.iter().filter(|i| i.sensitivity.is_sensitive()).count();
    let images = app.history.iter().filter(|i| matches!(i.content, crate::model::item::ClipContent::Image { .. })).count();
    let db_bytes = std::fs::metadata(&app.paths.db).map(|m| m.len()).unwrap_or(0);
    send(app, &Outgoing::Stats { items, pinned, sensitive, images, thumb_bytes: app.thumbs.total_bytes(), db_bytes, image_bytes: dir_size(&app.paths.images) });
}

pub fn handle_message(app: &mut App, json: &str) {
    let msg: Incoming = match serde_json::from_str(json) {
        Ok(m) => m,
        Err(e) => {
            log::warn!("settings message: {e}");
            return;
        }
    };
    match msg {
        Incoming::GetSettings => {
            if app.settings_snapshot.is_none() {
                app.settings_snapshot = Some(app.settings.clone());
            }
            send_settings(app);
            send_stats(app);
        }
        Incoming::SetSettings { mut data } => {
            data.validate();
            app.apply_settings(data);
            send(app, &Outgoing::Applied { ok: true, error: None });
        }
        Incoming::Reset => {
            app.apply_settings(Settings::default());
            send_settings(app);
        }
        Incoming::Revert => {
            if let Some(s) = app.settings_snapshot.clone() {
                app.apply_settings(s);
            }
            send_settings(app);
        }
        Incoming::OpenDataFolder => {
            crate::app::shell_open(&app.paths.root.to_string_lossy());
        }
        Incoming::OpenLogs => {
            crate::app::shell_open(&app.paths.logs.to_string_lossy());
        }
        Incoming::ClearHistory { keep_pinned } => {
            app.clear_history(keep_pinned);
            send_stats(app);
        }
        Incoming::GetStats => send_stats(app),
        Incoming::Close => app.close_settings(),
        Incoming::ShelfCreate { name, color } => {
            let name = if name.trim().is_empty() { format!("Shelf {}", app.shelves.shelves.len() + 1) } else { name };
            app.new_shelf(name, color);
            send_settings(app);
        }
        Incoming::ShelfUpdate { id, name, color } => {
            app.update_shelf(id, name, color);
            send_settings(app);
        }
        Incoming::ShelfDelete { id } => {
            app.delete_shelf(id);
            send_settings(app);
        }
        Incoming::OpenUrl { url } => {
            if url.starts_with("https://") {
                crate::app::shell_open(&url);
            }
        }
        Incoming::ShelfActivate { id } => {
            if app.shelves.get(id).is_some() {
                app.shelves.active = id;
                app.invalidate_public();
            }
            send_settings(app);
        }
    }
}
