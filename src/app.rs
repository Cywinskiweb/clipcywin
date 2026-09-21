//! UI-thread application state and message handling.

use crate::clipboard::reader::CaptureConfig;
use crate::clipboard::{reader, writer};
use crate::db;
use crate::dnd::{self, DragData, DragPayload, Zone};
use crate::gfx::anim::{Anim, Easing};
use crate::gfx::device::{is_device_lost, Gfx};
use crate::gfx::text::{Font, TextEngine};
use crate::gfx::theme::Theme;
use crate::gfx::{dip_to_px, px_to_dip, Rect};
use crate::i18n::t;
use crate::images::cache::BitmapCache;
use crate::images::worker as img_worker;
use crate::input::{hook, paste};
use crate::model::hash;
use crate::model::history::{HistoryStore, PushResult};
use crate::model::shelf::{ShelfEntry, ShelfStore};
use crate::model::item::{ClipContent, ClipItem, ItemId, Sensitivity};
use crate::msg::*;
use crate::privacy::capture_guard;
use crate::settings::paths::Paths;
use crate::settings::{Align, Backdrop, Edge, HotkeyAction, ModeKind, PanelPos, Placement, Settings, SettingsDelta, MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_WIN};
use crate::settings_ui::bridge;
use crate::settings_ui::window::SettingsWindow;
use crate::system::{autostart, process};
use crate::ui::appbar::AppBar;
use crate::ui::bar::{self, BarGeometry};
use crate::ui::context_menu::{self, MenuItem};
use crate::ui::island::{self, Island, IslandGeometry};
use crate::ui::items_view::{self, draw_text_in, fill_rr, stroke_rr, DrawCtx, Hit, ItemsLayout, LayoutParams};
use crate::ui::list_popup::{self, ListState};
use crate::ui::monitors::{self, MonitorInfo};
use crate::ui::preview;
use crate::ui::surface_window::{Corners, SurfaceWindow};
use crate::ui::tray::Tray;
use crate::ui::win::{self, defer, hinstance, lparam_xy, wheel_delta, CLASS_CORE};
use crate::util::wide::WStr;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::Ole::IDropTarget;
use windows::Win32::UI::Input::KeyboardAndMouse::DragDetect;
use windows::Win32::Graphics::Direct2D::D2D1_ELLIPSE;
use windows::Win32::Graphics::Gdi::ValidateRect;
use windows::Win32::System::DataExchange::{AddClipboardFormatListener, GetClipboardSequenceNumber, RemoveClipboardFormatListener};
use windows::Win32::UI::Accessibility::HWINEVENTHOOK;
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_HOVER, TME_LEAVE, TRACKMOUSEEVENT};
const WM_MOUSEHOVER: u32 = 0x02A1;
const WM_MOUSELEAVE: u32 = 0x02A3;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DispatchMessageW, GetCursorPos, GetForegroundWindow, GetMessageW, KillTimer, PostQuitMessage, RegisterWindowMessageW,
    SetTimer, TranslateMessage, HTCLIENT, HTTRANSPARENT, MA_NOACTIVATE, MSG, SW_SHOWNORMAL, WINDOW_EX_STYLE, WM_CLIPBOARDUPDATE, WM_CLOSE,
    WM_CONTEXTMENU, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ERASEBKGND, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_MBUTTONUP, WM_MOUSEACTIVATE,
    WM_EXITSIZEMOVE, WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCHITTEST, WM_PAINT, WM_RBUTTONUP, WM_SETTINGCHANGE, WM_SIZE, WM_THEMECHANGED,
    WM_TIMER, WM_WINDOWPOSCHANGED, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};
use windows_numerics::Vector2;

const SETTINGS_HTML: &str = include_str!("../assets/settings.html");
/// Where the in-app "support" button points.
pub const SUPPORT_URL: &str = "https://buymeacoffee.com/jakub_webdev";
pub const REPO_URL: &str = "https://github.com/Cywinskiweb/clipcywin";

// Menu command ids
const CMD_PASTE: u32 = 1;
const CMD_COPY: u32 = 2;
const CMD_PIN: u32 = 3;
const CMD_SENSITIVE: u32 = 4;
const CMD_REVEAL: u32 = 5;
const CMD_OPEN: u32 = 6;
const CMD_DELETE: u32 = 7;
const CMD_SETTINGS: u32 = 8;
const CMD_TRAY_TOGGLE: u32 = 20;
const CMD_TRAY_PAUSE: u32 = 21;
const CMD_TRAY_ALLOW_CAPTURE: u32 = 22;
const CMD_TRAY_CLEAR: u32 = 23;
const CMD_TRAY_CLEAR_ALL: u32 = 24;
const CMD_TRAY_SETTINGS: u32 = 25;
const CMD_TRAY_FOLDER: u32 = 26;
const CMD_TRAY_EXIT: u32 = 27;
const CMD_TRAY_MODE_BAR: u32 = 28;
const CMD_TRAY_MODE_ISLAND: u32 = 29;
const CMD_TRAY_RELOAD: u32 = 30;
const CMD_ADD_TO_SHELF: u32 = 40;
const CMD_SHELF_REMOVE: u32 = 41;
const CMD_SHELF_COPY_ALL: u32 = 42;
const CMD_SHELF_PASTE_ALL: u32 = 43;
const CMD_SHELF_CLEAR: u32 = 44;
const CMD_SHELF_NEW: u32 = 45;
const CMD_SHELF_DELETE: u32 = 46;
const CMD_SHELF_OPEN: u32 = 47;
const CMD_SHELF_SWITCH_BASE: u32 = 100; // + index

/// Hit regions of the shelf panel, refreshed on every render.
#[derive(Default, Clone)]
struct ShelfUi {
    header: Rect,
    grip: Rect,
    add_btn: Rect,
    copy_btn: Rect,
    clear_btn: Rect,
    close_btn: Rect,
    tabs: Vec<(i64, Rect)>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Borrow the app if it is not already borrowed by an outer handler.
pub fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| {
        let mut b = a.try_borrow_mut().ok()?;
        b.as_mut().map(f)
    })
}

pub fn dispatch(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> Option<LRESULT> {
    with_app(|app| app.handle(hwnd, msg, w, l)).flatten()
}

pub fn shell_open(target: &str) {
    unsafe {
        ShellExecuteW(None, WStr::new("open").pcwstr(), WStr::new(target).pcwstr(), None, None, SW_SHOWNORMAL);
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub struct App {
    pub paths: Paths,
    pub settings: Settings,
    pub settings_snapshot: Option<Settings>,
    pub theme: Theme,
    pub gfx: Gfx,
    pub text: TextEngine,
    pub history: HistoryStore,
    pub thumbs: BitmapCache,
    pub previews: BitmapCache,
    pub core: HWND,
    pub main: Option<SurfaceWindow>,
    pub preview_win: Option<SurfaceWindow>,
    pub list_win: Option<SurfaceWindow>,
    pub sentinel: Option<SurfaceWindow>,
    pub tray: Tray,
    pub settings_win: Option<SettingsWindow>,
    to_img: Sender<ToImg>,
    to_db: Sender<ToDb>,
    to_clip: Sender<ToClip>,
    from_workers: Receiver<ToUi>,
    capture_cfg: Arc<Mutex<CaptureConfig>>,
    poller: capture_guard::Poller,
    monitor: MonitorInfo,
    bar_geom: Option<BarGeometry>,
    island_geom: Option<IslandGeometry>,
    island: Island,
    appbar: Option<AppBar>,
    layout: ItemsLayout,
    list_layout: ItemsLayout,
    list_state: ListState,
    scroll: usize,
    hover: Option<ItemId>,
    hover_close: bool,
    hover_in_list: bool,
    preview_item: Option<ItemId>,
    chord_held: bool,
    self_set_seq: u32,
    paused: bool,
    hidden_for_capture: bool,
    capture_allowed: bool,
    fullscreen_hidden: bool,
    capture_proc_running: bool,
    main_shown: bool,
    flash: Option<(ItemId, Anim)>,
    fg_hook: HWINEVENTHOOK,
    activate_msg: u32,
    settings_msg: u32,
    toggle_msg: u32,
    reload_msg: u32,
    taskbar_created_msg: u32,
    start_time: Instant,
    first_frame_logged: bool,
    loaded: bool,
    image_paths_pending: HashMap<ItemId, Sensitivity>,
    // shelves & drag/drop
    pub shelves: ShelfStore,
    shelf_win: Option<SurfaceWindow>,
    shelf_shown: Option<i64>,
    shelf_layout: ItemsLayout,
    shelf_ui: ShelfUi,
    shelf_scroll: usize,
    shelf_hover: Option<ItemId>,
    shelf_hover_close: bool,
    drop_targets: Vec<IDropTarget>,
    drag_hover: Option<Zone>,
    drag_ok: bool,
    drag_in_progress: bool,
    pending_click: Option<(ItemId, u8)>,
    island_region: Option<(i32, i32, i32, i32, i32)>,
}

pub fn run(paths: Paths, settings: Settings, autostart_flag: bool, start: Instant) {
    let core = unsafe {
        match CreateWindowExW(
            WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0),
            WStr::new(CLASS_CORE).pcwstr(),
            WStr::new("Clipcywin").pcwstr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance()),
            None,
        ) {
            Ok(h) => h,
            Err(e) => {
                log::error!("core window: {e}");
                return;
            }
        }
    };
    let waker = UiWaker::new(core);

    let (to_ui, from_workers) = channel::<ToUi>();
    let (to_img, img_rx) = channel::<ToImg>();
    let (to_db, db_rx) = channel::<ToDb>();
    let (to_clip, clip_rx) = channel::<ToClip>();
    let capture_cfg = Arc::new(Mutex::new(CaptureConfig::from_settings(&settings)));
    img_worker::spawn(img_rx, to_ui.clone(), waker);
    db::spawn(paths.db.clone(), db_rx, to_ui.clone(), waker);
    reader::spawn(clip_rx, to_ui.clone(), waker, capture_cfg.clone());

    let t_gfx = Instant::now();
    let prefer_warp = settings.general.renderer != "hardware";
    let gfx = match Gfx::new(prefer_warp) {
        Ok(g) => g,
        Err(e) => {
            log::error!("graphics init: {e}");
            return;
        }
    };
    log::info!("gfx init ({}) took {:?}", if prefer_warp { "warp" } else { "hardware" }, t_gfx.elapsed());
    let t_text = Instant::now();
    let text = match TextEngine::with_preview(&gfx.dwrite, &settings.appearance, settings.preview.font_size) {
        Ok(t) => t,
        Err(e) => {
            log::error!("text init: {e}");
            return;
        }
    };
    log::info!("text init took {:?}", t_text.elapsed());
    let theme = Theme::resolve(&settings.appearance);
    let mons = monitors::enumerate();
    let monitor = monitors::choose(&mons, &settings.monitor);
    let poller = capture_guard::spawn_poller(waker, settings.hide.on_capture_procs, settings.hide.procs.clone());
    let thumb_cap = settings.general.thumb_cache_mb as usize * 1024 * 1024;
    let history = HistoryStore::new(settings.layout.pinned_first, settings.capture.dedupe);
    let main_shown_initial = !(autostart_flag && settings.bar.placement == Placement::OnDemand);

    let app = App {
        paths,
        settings,
        settings_snapshot: None,
        theme,
        gfx,
        text,
        history,
        thumbs: BitmapCache::new(thumb_cap),
        previews: BitmapCache::new(64 * 1024 * 1024),
        core,
        main: None,
        preview_win: None,
        list_win: None,
        sentinel: None,
        tray: Tray::new(core),
        settings_win: None,
        to_img,
        to_db,
        to_clip,
        from_workers,
        capture_cfg,
        poller,
        monitor,
        bar_geom: None,
        island_geom: None,
        island: Island::new(),
        appbar: None,
        layout: ItemsLayout::default(),
        list_layout: ItemsLayout::default(),
        list_state: ListState::default(),
        scroll: 0,
        hover: None,
        hover_close: false,
        hover_in_list: false,
        preview_item: None,
        chord_held: false,
        self_set_seq: 0,
        paused: false,
        hidden_for_capture: false,
        capture_allowed: false,
        fullscreen_hidden: false,
        capture_proc_running: false,
        main_shown: main_shown_initial,
        flash: None,
        fg_hook: HWINEVENTHOOK::default(),
        activate_msg: crate::system::single_instance::activate_message(),
        settings_msg: crate::system::single_instance::settings_message(),
        toggle_msg: crate::system::single_instance::toggle_message(),
        reload_msg: crate::system::single_instance::reload_message(),
        taskbar_created_msg: unsafe { RegisterWindowMessageW(WStr::new("TaskbarCreated").pcwstr()) },
        start_time: start,
        first_frame_logged: false,
        loaded: false,
        image_paths_pending: HashMap::new(),
        shelves: ShelfStore::new(),
        shelf_win: None,
        shelf_shown: None,
        shelf_layout: ItemsLayout::default(),
        shelf_ui: ShelfUi::default(),
        shelf_scroll: 0,
        shelf_hover: None,
        shelf_hover_close: false,
        drop_targets: Vec::new(),
        drag_hover: None,
        drag_ok: false,
        drag_in_progress: false,
        pending_click: None,
        island_region: None,
    };
    APP.with(|a| *a.borrow_mut() = Some(app));

    with_app(|app| {
        let t = Instant::now();
        app.tray.add("Clipcywin");
        app.rebuild_windows();
        log::info!("windows created after {:?} (took {:?})", app.start_time.elapsed(), t.elapsed());
        unsafe {
            if let Err(e) = AddClipboardFormatListener(app.core) {
                log::error!("AddClipboardFormatListener: {e}");
            }
        }
        hook::start(app.core, &hook::HookConfig::from_settings(&app.settings));
        app.fg_hook = capture_guard::start_foreground_hook(app.core);
        let _ = app.to_db.send(ToDb::LoadAll { limit: app.settings.capture.max_history as usize });
        let _ = app.to_db.send(ToDb::LoadShelves);
        if app.settings.general.autostart != autostart::is_enabled() {
            autostart::set_enabled(app.settings.general.autostart);
        }
        app.render_all();
    });
    win::run_deferred();

    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
            win::run_deferred();
        }
    }

    // Shutdown
    let app = APP.with(|a| a.borrow_mut().take());
    if let Some(mut app) = app {
        hook::stop();
        capture_guard::stop_foreground_hook(app.fg_hook);
        unsafe {
            let _ = RemoveClipboardFormatListener(app.core);
        }
        let _ = app.to_clip.send(ToClip::Shutdown);
        let _ = app.to_img.send(ToImg::Shutdown);
        let _ = app.to_db.send(ToDb::Shutdown);
        app.tray.remove();
        if let Some(mut ab) = app.appbar.take() {
            ab.remove();
        }
        if let Some(w) = app.settings_win.take() {
            w.close();
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

impl App {
    // ---------------------------------------------------------------- helpers

    fn now(&self) -> Instant {
        Instant::now()
    }

    fn animate(&self) -> bool {
        self.settings.appearance.animations
    }

    fn is_island(&self) -> bool {
        self.settings.mode == ModeKind::Island
    }

    fn modifier_hint(&self) -> String {
        let m = self.settings.hotkeys.modifiers;
        let mut parts = Vec::new();
        if m & MOD_WIN != 0 {
            parts.push("Win");
        }
        if m & MOD_CTRL != 0 {
            parts.push("Ctrl");
        }
        if m & MOD_ALT != 0 {
            parts.push("Alt");
        }
        if m & MOD_SHIFT != 0 {
            parts.push("Shift");
        }
        parts.join("+")
    }

    /// Width of the modifier badge: Windows logo + "+Ctrl" (or plain text) plus padding.
    fn hint_len(&self) -> f32 {
        if !self.settings.layout.show_modifier_hint {
            return 0.0;
        }
        let hint = self.modifier_hint();
        let pad = 11.0;
        let w = if let Some(rest) = hint.strip_prefix("Win") {
            12.0 + if rest.is_empty() { 0.0 } else { 3.0 + self.text.measure(rest, Font::Badge).0 }
        } else {
            self.text.measure(&hint, Font::Badge).0
        };
        (w + 2.0 * pad).ceil().max(36.0)
    }

    fn thumb_px(&self) -> (u32, u32) {
        let pad = self.settings.layout.padding_dip as f32;
        let w = self.settings.layout.item_width_dip as f32 * 0.55;
        let h = self.settings.bar.height_dip as f32 - 2.0 * pad - 8.0;
        let dpi = self.monitor.dpi;
        (dip_to_px(w, dpi).max(16) as u32 * 2, dip_to_px(h, dpi).max(16) as u32 * 2)
    }

    fn visible_ids(&self) -> Vec<ItemId> {
        self.history.visible().iter().map(|i| i.id).collect()
    }

    fn set_timer(&self, id: usize, ms: u32) {
        unsafe {
            SetTimer(Some(self.core), id, ms, None);
        }
    }

    fn kill_timer(&self, id: usize) {
        unsafe {
            let _ = KillTimer(Some(self.core), id);
        }
    }

    fn cursor_pos() -> (i32, i32) {
        let mut p = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut p);
        }
        (p.x, p.y)
    }

    // ---------------------------------------------------------------- windows

    pub fn rebuild_windows(&mut self) {
        // Tear down
        if let Some(mut ab) = self.appbar.take() {
            ab.remove();
        }
        self.drop_targets.clear();
        if let Some(w) = self.main.take() {
            dnd::revoke(w.hwnd);
            w.destroy();
        }
        if let Some(w) = self.shelf_win.take() {
            dnd::revoke(w.hwnd);
            w.destroy();
        }
        self.shelf_shown = None;
        if let Some(w) = self.preview_win.take() {
            w.destroy();
        }
        if let Some(w) = self.list_win.take() {
            w.destroy();
        }
        if let Some(w) = self.sentinel.take() {
            w.destroy();
        }
        self.hover = None;
        self.preview_item = None;
        self.island = Island::new();
        self.island_region = None;

        let mons = monitors::enumerate();
        self.monitor = monitors::choose(&mons, &self.settings.monitor);
        let dpi = self.monitor.dpi;
        let topmost = self.is_island() || self.settings.bar.placement != Placement::Docked || self.settings.bar.docked_topmost;
        match SurfaceWindow::create(&self.gfx, topmost, false, self.monitor.rect.left, self.monitor.rect.top, 100, 40, dpi) {
            Ok(w) => self.main = Some(w),
            Err(e) => {
                log::error!("main window: {e}");
                return;
            }
        }
        if !self.is_island() && self.settings.bar.placement == Placement::Docked {
            let mut ab = AppBar::new(self.main.as_ref().unwrap().hwnd);
            ab.register();
            self.appbar = Some(ab);
        }
        match SurfaceWindow::create(&self.gfx, true, true, 0, 0, 10, 10, dpi) {
            Ok(w) => self.preview_win = Some(w),
            Err(e) => log::warn!("preview window: {e}"),
        }
        match SurfaceWindow::create(&self.gfx, true, false, 0, 0, 10, 10, dpi) {
            Ok(w) => self.list_win = Some(w),
            Err(e) => log::warn!("list window: {e}"),
        }
        if self.settings.shelves.enabled {
            match SurfaceWindow::create(&self.gfx, true, false, 0, 0, 10, 10, dpi) {
                Ok(w) => {
                    match dnd::DropTarget::register(w.hwnd, Zone::Shelf) {
                        Ok(t) => self.drop_targets.push(t),
                        Err(e) => log::warn!("RegisterDragDrop(shelf): {e}"),
                    }
                    self.shelf_win = Some(w);
                }
                Err(e) => log::warn!("shelf window: {e}"),
            }
            if let Some(m) = &self.main {
                match dnd::DropTarget::register(m.hwnd, Zone::Bar) {
                    Ok(t) => self.drop_targets.push(t),
                    Err(e) => log::warn!("RegisterDragDrop(bar): {e}"),
                }
            }
        }
        if !self.is_island() && self.settings.bar.placement == Placement::OnDemand && self.settings.capture.edge_trigger {
            let r = self.monitor.rect;
            let (x, y, w, h) = match self.settings.bar.edge {
                Edge::Bottom => (r.left, r.bottom - 1, r.right - r.left, 1),
                Edge::Top => (r.left, r.top, r.right - r.left, 1),
            };
            match SurfaceWindow::create(&self.gfx, true, false, x, y, w, h, dpi) {
                Ok(w) => {
                    w.show();
                    self.sentinel = Some(w);
                }
                Err(e) => log::warn!("sentinel: {e}"),
            }
        }
        self.apply_window_attrs();
        self.relayout();
        self.update_visibility();
    }

    fn island_tint(&self) -> (f32, f32, f32, f32) {
        let c = self.settings.island.color.map(crate::gfx::theme::Color::from_rgb_u32).unwrap_or(self.theme.bg_solid);
        (c.r, c.g, c.b, (self.settings.island.opacity * 0.8).clamp(0.05, 0.95))
    }

    fn preview_tint(&self) -> (f32, f32, f32, f32) {
        let c = self.settings.preview.color.map(crate::gfx::theme::Color::from_rgb_u32).unwrap_or(self.theme.bg_solid);
        (c.r, c.g, c.b, (self.settings.preview.opacity * 0.8).clamp(0.05, 0.95))
    }

    fn preview_style(&self) -> preview::PreviewStyle {
        let p = &self.settings.preview;
        let base = p.color.map(crate::gfx::theme::Color::from_rgb_u32).unwrap_or(self.theme.bg_solid);
        let alpha = if p.blur { 0.12 } else { p.opacity };
        preview::PreviewStyle { fill: base.with_alpha(alpha), radius: p.corner_radius, border: p.border }
    }

    fn acrylic_tint(&self) -> (f32, f32, f32, f32) {
        let c = self.theme.bg_solid;
        (c.r, c.g, c.b, (self.theme.opacity * 0.8).clamp(0.05, 0.95))
    }

    fn apply_window_attrs(&mut self) {
        let excluded = self.settings.hide.exclude_from_capture && !self.capture_allowed;
        let dark = self.theme.dark;
        let backdrop = if self.is_island() {
            if self.settings.island.blur { Backdrop::Acrylic } else { Backdrop::None }
        } else {
            self.settings.appearance.backdrop
        };
        let tint = if self.is_island() { self.island_tint() } else { self.acrylic_tint() };
        if let Some(m) = &self.main {
            m.set_capture_excluded(excluded);
            m.set_backdrop(backdrop, dark, Corners::Square, tint);
        }
        self.apply_main_region();
        let ptint = self.preview_tint();
        if let Some(p) = &self.preview_win {
            p.set_capture_excluded(excluded);
            p.set_backdrop(if self.settings.preview.blur { Backdrop::Acrylic } else { Backdrop::None }, dark, Corners::Square, ptint);
        }
        if let Some(l) = &self.list_win {
            l.set_capture_excluded(excluded);
            l.set_backdrop(Backdrop::None, dark, Corners::Square, tint);
        }
        if let Some(l) = &self.shelf_win {
            l.set_capture_excluded(excluded);
            l.set_backdrop(Backdrop::None, dark, Corners::Square, tint);
        }
        if let Some(s) = &self.sentinel {
            s.set_capture_excluded(true);
        }
    }

    /// Clip the bar window (and its blur) to the drawn bar shape so rounded corners show what is behind.
    fn apply_main_region(&mut self) {
        let Some(g) = self.bar_geom.clone() else {
            if self.island_region.is_none() {
                if let Some(m) = &mut self.main {
                    m.set_region(None);
                }
            }
            return;
        };
        let radius = self.settings.appearance.corner_radius.min(g.area.h / 2.0);
        let needs = !self.is_island() && self.settings.appearance.backdrop != Backdrop::None && (g.floating || radius > 0.0);
        if let Some(m) = &mut self.main {
            if needs {
                let dpi = g.dpi;
                let rx = dip_to_px(g.area.x, dpi);
                let ry = dip_to_px(g.area.y, dpi);
                let rw = dip_to_px(g.area.w, dpi);
                let rh = dip_to_px(g.area.h, dpi);
                let rad = if g.floating { dip_to_px(radius, dpi) } else { 0 };
                m.set_region(Some((rx, ry, rw, rh, rad)));
            } else {
                m.set_region(None);
            }
        }
    }

    /// Recompute geometry for the current mode and move the main window.
    pub fn relayout(&mut self) {
        let mons = monitors::enumerate();
        self.monitor = monitors::choose(&mons, &self.settings.monitor);
        let dpi = self.monitor.dpi;
        if self.is_island() {
            let n = self.history.len().max(1);
            let g = island::compute(&self.settings, &self.monitor, n, self.hint_len());
            if let Some(m) = &mut self.main {
                let r = g.window;
                m.set_bounds(r.left, r.top, r.right - r.left, r.bottom - r.top, dpi);
            }
            self.island_geom = Some(g);
            self.bar_geom = None;
        } else {
            let appbar_rect = if let Some(ab) = &mut self.appbar {
                let thick = dip_to_px((self.settings.bar.height_dip + 2 * self.settings.bar.margin_dip) as f32, dpi);
                Some(ab.set_pos(&self.monitor.rect, self.settings.bar.edge, thick))
            } else {
                None
            };
            let g = bar::compute(&self.settings, &self.monitor, appbar_rect);
            if let Some(m) = &mut self.main {
                let r = g.window;
                m.set_bounds(r.left, r.top, r.right - r.left, r.bottom - r.top, dpi);
                if m.topmost != g.topmost {
                    m.set_topmost(g.topmost);
                }
            }
            self.bar_geom = Some(g);
            self.island_geom = None;
            self.apply_main_region();
        }
        self.compute_layout();
        if self.shelf_shown.is_some() {
            self.relayout_shelf();
        }
    }

    fn layout_params(&self, vertical: bool) -> LayoutParams {
        let s = &self.settings;
        LayoutParams {
            item_len: if vertical { s.bar.height_dip as f32 - 2.0 * s.layout.padding_dip as f32 } else { s.layout.item_width_dip as f32 },
            gap: s.layout.gap_dip as f32,
            padding: s.layout.padding_dip as f32,
            overflow_len: 28.0,
            hint_len: if !vertical { self.hint_len() } else { 0.0 },
            max_visible: s.layout.max_visible as usize,
            align: match s.layout.content_align {
                Align::Left => 0,
                Align::Center => 1,
                Align::Right => 2,
            },
            pill: s.layout.item_style == crate::settings::ItemStyle::Pill,
            shelf_len: if s.shelves.enabled { (s.bar.height_dip as f32 - 2.0 * s.layout.padding_dip as f32).max(24.0) } else { 0.0 },
            hint_pinned: s.layout.hint_pinned_left,
        }
    }

    fn items_area(&self) -> (Rect, bool) {
        if let Some(g) = &self.island_geom {
            (g.expanded, g.vertical)
        } else if let Some(g) = &self.bar_geom {
            (g.area, false)
        } else {
            (Rect::default(), false)
        }
    }

    fn compute_layout(&mut self) {
        let ids = self.visible_ids();
        let (area, vertical) = self.items_area();
        let p = self.layout_params(vertical);
        let max_scroll = ids.len().saturating_sub(1);
        self.scroll = self.scroll.min(max_scroll);
        self.layout = items_view::layout(&ids, area, vertical, &p, self.scroll);
        if self.hover.is_some_and(|h| !ids.contains(&h)) {
            self.hover = None;
        }
    }

    fn invalidate(&self) {
        if let Some(m) = &self.main {
            unsafe {
                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(m.hwnd), None, false);
            }
        }
    }

    fn should_show_main(&self) -> bool {
        if self.hidden_for_capture || self.capture_proc_running {
            return false;
        }
        if self.fullscreen_hidden && self.settings.capture.hide_on_fullscreen {
            return false;
        }
        if self.is_island() {
            return true;
        }
        match self.settings.bar.placement {
            Placement::OnDemand => self.main_shown || self.chord_held,
            _ => true,
        }
    }

    pub fn invalidate_public(&self) {
        self.invalidate();
    }

    pub fn update_visibility(&mut self) {
        let show = self.should_show_main();
        let mut hid = false;
        if let Some(m) = &self.main {
            if show && !m.is_visible() {
                m.show();
                m.raise();
                self.invalidate();
            } else if !show && m.is_visible() {
                m.hide();
                hid = true;
            }
        }
        if hid {
            // The window never gets WM_MOUSELEAVE once hidden; clear hover state by hand.
            self.hover = None;
            self.hover_close = false;
            self.hide_preview();
            self.hide_list();
            if let Some(sn) = &self.sentinel {
                sn.raise();
            }
        }
        if self.fullscreen_hidden || self.hidden_for_capture || self.capture_proc_running {
            self.set_timer(TIMER_FS_CHECK, 1500);
        }
    }

    // ---------------------------------------------------------------- rendering

    fn draw_ctx<'a>(gfx: &'a Gfx, theme: &'a Theme, text: &'a mut TextEngine, thumbs: &'a mut BitmapCache, s: &Settings, hover: Option<ItemId>, hover_close: bool, active: Option<ItemId>, chord: bool, hint: String, flash: Option<(ItemId, f32)>, alpha: f32) -> DrawCtx<'a> {
        DrawCtx {
            gfx,
            theme,
            text,
            thumbs,
            hover,
            hover_close,
            active,
            chord_held: chord,
            now: Instant::now(),
            highlight: s.appearance.highlight,
            show_badges: s.layout.show_badges,
            show_meta: s.layout.show_meta,
            show_source: s.layout.show_source,
            modifier_hint: hint,
            flash,
            content_alpha: alpha,
            pill: s.layout.item_style == crate::settings::ItemStyle::Pill,
            shelf_count: 0,
            shelf_hot: false,
            shelf_color: None,
            thumb_requests: Vec::new(),
        }
    }

    fn flash_value(&self) -> Option<(ItemId, f32)> {
        let now = self.now();
        self.flash.map(|(id, a)| (id, a.value(now)))
    }

    pub fn render_main(&mut self) {
        let now = self.now();
        if let (Some(g), true) = (self.island_geom.clone(), self.settings.island.blur) {
            // The blur is clipped to the (animated) pill by a window region.
            let pill = self.island.rect(&g, now);
            let t = self.island.t.value(now);
            let dpi = g.dpi;
            let half = pill.h.min(pill.w) / 2.0;
            let rad = dip_to_px(half * (1.0 - t) + self.theme.radius.max(6.0).min(half) * t, dpi);
            let region = (dip_to_px(pill.x, dpi), dip_to_px(pill.y, dpi), dip_to_px(pill.w, dpi), dip_to_px(pill.h, dpi), rad);
            self.island_region = Some(region);
            if let Some(m) = &mut self.main {
                if m.region() != Some(region) {
                    m.set_region(Some(region));
                }
            }
        } else {
            self.island_region = None;
            if let Some(m) = &mut self.main {
                if m.region().is_some() && self.bar_geom.is_none() {
                    m.set_region(None);
                }
            }
        }
        let Some(main) = &self.main else { return };
        if !main.is_visible() {
            return;
        }
        let items: Vec<&ClipItem> = self.history.visible();
        let hint = self.modifier_hint();
        let flash = self.flash_value();
        let chord = self.chord_held && self.settings.hotkeys.show_badges_on_chord;
        let theme = self.theme;
        let s = &self.settings;
        let active = self.history.active;
        let (hover, hover_close) = if self.hover_in_list { (None, false) } else { (self.hover, self.hover_close) };
        let layout = &self.layout;
        let gfx = &self.gfx;
        let text = &mut self.text;
        let thumbs = &mut self.thumbs;
        let island_geom = self.island_geom.as_ref();
        let island = &self.island;
        let bar_geom = self.bar_geom.as_ref();
        let shelf_count = self.shelves.active_shelf().map(|sh| sh.entries.len()).unwrap_or(0);
        let shelf_hot = self.drag_hover == Some(Zone::Bar) && self.drag_ok || self.shelf_shown.is_some();
        let shelf_color = self.shelves.active_shelf().map(|sh| crate::gfx::theme::Color::from_rgb_u32(sh.color));
        let mut requests = Vec::new();
        let r = main.surface.render(gfx, |dc| {
            if let Some(g) = island_geom {
                let pill = island.rect(g, now);
                let t = island.t.value(now);
                let radius = pill.h.min(pill.w) / 2.0 * (1.0 - t) + theme.radius.max(6.0).min(pill.h.min(pill.w) / 2.0) * t;
                let pill_color = s.island.color.map(crate::gfx::theme::Color::from_rgb_u32).unwrap_or(theme.bg_solid);
                fill_rr(dc, gfx, pill, radius, pill_color.with_alpha(if s.island.blur { 0.12 } else { s.island.opacity }));
                if s.island.border {
                    stroke_rr(dc, gfx, pill, radius, theme.border, 1.0);
                }
                let ca = island.content_alpha(now);
                if ca > 0.0 && !items.is_empty() {
                    let mut ctx = App::draw_ctx(gfx, &theme, text, thumbs, s, hover, hover_close, active, chord, hint.clone(), flash, ca);
                    ctx.shelf_count = shelf_count;
                    ctx.shelf_hot = shelf_hot;
                    ctx.shelf_color = shelf_color;
                    unsafe { dc.PushAxisAlignedClip(&pill.d2d(), windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
                    items_view::draw_items(dc, &mut ctx, layout, &items);
                    unsafe { dc.PopAxisAlignedClip() };
                    requests = std::mem::take(&mut ctx.thumb_requests);
                }
                let ka = island.collapsed_alpha(now);
                if ka > 0.0 {
                    // Collapsed content: count + kind glyph + pulse.
                    let pulse = island.pulse.value(now);
                    let c = pill;
                    let label = format!("{}", items.len());
                    let glyph = match items.first().map(|i| &i.content) {
                        Some(ClipContent::Image { .. }) => "\u{EB9F}",
                        Some(ClipContent::Files { .. }) => "\u{E8B7}",
                        Some(ClipContent::Text { .. }) => "\u{E8D2}",
                        None => "\u{E77F}",
                    };
                    let fg = theme.fg.with_alpha(ka);
                    if g.vertical {
                        let gr = Rect::new(c.x, c.y + 8.0, c.w, 20.0);
                        draw_text_in(dc, text, gfx, glyph, Font::Icon, gr, fg, false, false);
                        let lr = Rect::new(c.x, c.bottom() - 28.0, c.w, 20.0);
                        draw_text_in(dc, text, gfx, &label, Font::Badge, lr, fg, false, false);
                    } else {
                        let gr = Rect::new(c.x + 12.0, c.y, 20.0, c.h);
                        draw_text_in(dc, text, gfx, glyph, Font::Icon, gr, fg, false, false);
                        let lr = Rect::new(c.x + 36.0, c.y, c.w - 48.0, c.h);
                        draw_text_in(dc, text, gfx, &label, Font::BodyBold, lr, fg, false, true);
                    }
                    if pulse > 0.0 {
                        let (cx, cy) = c.center();
                        let rad = (c.w.max(c.h) / 2.0) * (1.0 + 0.4 * (1.0 - pulse));
                        unsafe {
                            dc.DrawEllipse(&D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: rad, radiusY: rad * (c.h / c.w.max(1.0)).min(1.0).max(0.3) }, gfx.brush(theme.accent.with_alpha(0.6 * pulse * ka)), 2.0, None);
                        }
                    }
                    if items.first().is_some_and(|i| i.sensitivity.is_sensitive()) {
                        let lr = Rect::new(c.right() - 24.0, c.y, 20.0, c.h);
                        draw_text_in(dc, text, gfx, "\u{E72E}", Font::Icon, lr, theme.fg_secondary.with_alpha(ka), false, false);
                    }
                }
            } else if let Some(g) = bar_geom {
                let bg_alpha = match s.appearance.backdrop {
                    Backdrop::None => theme.opacity,
                    Backdrop::Acrylic => 0.12,
                    Backdrop::Mica => theme.opacity * 0.55,
                };
                let radius = if g.floating { theme.radius.min(g.area.h / 2.0) } else { 0.0 };
                fill_rr(dc, gfx, g.area, radius, theme.bg_solid.with_alpha(bg_alpha));
                if g.floating {
                    stroke_rr(dc, gfx, g.area, radius, theme.border, 1.0);
                }
                let mut ctx = App::draw_ctx(gfx, &theme, text, thumbs, s, hover, hover_close, active, chord, hint.clone(), flash, 1.0);
                ctx.shelf_count = shelf_count;
                ctx.shelf_hot = shelf_hot;
                ctx.shelf_color = shelf_color;
                if items.is_empty() {
                    items_view::draw_empty(dc, &mut ctx, g.area, t("Copy something to get started"));
                    if let Some(sr) = layout.shelf {
                        items_view::draw_shelf_button(dc, &mut ctx, sr);
                    }
                } else {
                    items_view::draw_items(dc, &mut ctx, layout, &items);
                }
                requests = std::mem::take(&mut ctx.thumb_requests);
            }
        });
        if let Err(e) = r {
            if is_device_lost(e.code()) {
                self.recover_device();
            } else {
                log::warn!("render: {e}");
            }
            return;
        }
        if !self.first_frame_logged {
            self.first_frame_logged = true;
            log::info!("first frame after {:?}", self.start_time.elapsed());
        }
        self.request_thumbs(requests);
        // animation timer
        let animating = self.island.animating(now) || self.flash.is_some_and(|(_, a)| a.active(now));
        if animating {
            self.set_timer(TIMER_ANIM, 16);
        } else {
            self.kill_timer(TIMER_ANIM);
            if self.flash.is_some() {
                self.flash = None;
            }
        }
    }

    fn request_thumbs(&mut self, ids: Vec<ItemId>) {
        let px = self.thumb_px();
        for id in ids {
            if let Some(item) = self.history.get(id) {
                if let ClipContent::Image { path: Some(p), .. } = &item.content {
                    let _ = self.to_img.send(ToImg::Thumb { id, path: p.clone(), thumb_px: px });
                } else {
                    self.thumbs.pending.remove(&id);
                }
            }
        }
    }

    fn recover_device(&mut self) {
        log::warn!("graphics device lost; rebuilding");
        if let Err(e) = self.gfx.recreate() {
            log::error!("device recreate failed: {e}");
            return;
        }
        self.thumbs.clear();
        self.previews.clear();
        for w in [&mut self.main, &mut self.preview_win, &mut self.list_win, &mut self.sentinel].into_iter().flatten() {
            if let Err(e) = w.recreate_surface(&self.gfx) {
                log::error!("surface recreate: {e}");
            }
        }
        self.invalidate();
    }

    fn render_all(&mut self) {
        self.render_main();
    }

    // ---------------------------------------------------------------- preview

    fn item_screen_rect(&self, id: ItemId, in_list: bool) -> Option<RECT> {
        let (win, layout) = if in_list { (self.list_win.as_ref()?, &self.list_layout) } else { (self.main.as_ref()?, &self.layout) };
        let slot = layout.slot_of(id)?;
        let dpi = win.dpi;
        Some(RECT {
            left: win.x + dip_to_px(slot.rect.x, dpi),
            top: win.y + dip_to_px(slot.rect.y, dpi),
            right: win.x + dip_to_px(slot.rect.right(), dpi),
            bottom: win.y + dip_to_px(slot.rect.bottom(), dpi),
        })
    }

    fn shelf_item_screen_rect(&self, id: ItemId) -> Option<RECT> {
        let win = self.shelf_win.as_ref()?;
        let slot = self.shelf_layout.slot_of(id)?;
        let dpi = win.dpi;
        Some(RECT {
            left: win.x + dip_to_px(slot.rect.x, dpi),
            top: win.y + dip_to_px(slot.rect.y, dpi),
            right: win.x + dip_to_px(slot.rect.right(), dpi),
            bottom: win.y + dip_to_px(slot.rect.bottom(), dpi),
        })
    }

    fn show_preview(&mut self, id: ItemId, in_list: bool) {
        let Some(item) = self.history.get(id).cloned() else { return };
        self.show_preview_for(item, if in_list { 1 } else { 0 });
    }

    fn show_preview_for(&mut self, item: ClipItem, source: u8) {
        if !self.settings.appearance.hover_previews {
            return;
        }
        let id = item.id;
        let in_list = source == 1;
        let now = self.now();
        let kind = preview::kind_for(&item, now);
        if let preview::PreviewKind::Image { id, .. } = &kind {
            if !self.previews.contains(*id) && !self.previews.pending.contains(id) {
                if let ClipContent::Image { path: Some(p), .. } = &item.content {
                    self.previews.pending.insert(*id);
                    let wa = self.monitor.work;
                    let _ = self.to_img.send(ToImg::Preview { id: *id, path: p.clone(), max_px: ((wa.right - wa.left) as u32 / 2, (wa.bottom - wa.top) as u32 / 2) });
                }
            }
        }
        let footer = preview::footer(&item);
        let dpi = self.monitor.dpi;
        let wa0 = self.monitor.work;
        let max_w = px_to_dip(wa0.right - wa0.left, dpi) * self.settings.preview.max_width_pct as f32 / 100.0;
        let max_h = px_to_dip(wa0.bottom - wa0.top, dpi) * self.settings.preview.max_height_pct as f32 / 100.0;
        let (w_dip, h_dip, footer_h) = preview::measure(&kind, &footer, &self.text, &mut self.thumbs, max_w, max_h);
        let (w, h) = (dip_to_px(w_dip, dpi), dip_to_px(h_dip, dpi));
        let Some(anchor) = (if source == 2 { self.shelf_item_screen_rect(id) } else { self.item_screen_rect(id, in_list) }) else { return };
        let wa = self.monitor.work;
        let gap = 8;
        // Prefer above for bottom bars, below for top bars/islands, side for vertical islands.
        let vertical = self.island_geom.as_ref().is_some_and(|g| g.vertical);
        let (mut x, mut y) = if vertical {
            let right_side = anchor.left < (wa.left + wa.right) / 2;
            (if right_side { anchor.right + gap } else { anchor.left - gap - w }, anchor.top)
        } else {
            let island_bottom = self.is_island() && self.settings.island.anchor.is_bottom();
            let below = (self.is_island() && !island_bottom) || (!self.is_island() && self.settings.bar.edge == Edge::Top) || in_list && anchor.top < (wa.top + wa.bottom) / 2;
            ((anchor.left + anchor.right) / 2 - w / 2, if below { anchor.bottom + gap } else { anchor.top - gap - h })
        };
        x = x.clamp(wa.left, (wa.right - w).max(wa.left));
        y = y.clamp(wa.top, (wa.bottom - h).max(wa.top));
        self.preview_item = Some(id);
        let theme = self.theme;
        let style = self.preview_style();
        let blur = self.settings.preview.blur;
        let radius_px = dip_to_px(self.settings.preview.corner_radius, dpi);
        let s = &self.settings;
        if let Some(p) = &mut self.preview_win {
            p.set_bounds(x, y, w, h, dpi);
            p.set_region(if blur && radius_px > 0 { Some((0, 0, w, h, radius_px)) } else { None });
            let gfx = &self.gfx;
            let text = &mut self.text;
            let thumbs = &mut self.thumbs;
            let previews = &mut self.previews;
            let area = Rect::new(0.0, 0.0, w_dip, h_dip);
            let r = p.surface.render(gfx, |dc| {
                let mut ctx = App::draw_ctx(gfx, &theme, text, thumbs, s, None, false, None, false, String::new(), None, 1.0);
                preview::draw(dc, &mut ctx, &kind, &footer, area, footer_h, previews, style);
            });
            if r.is_ok() {
                p.show();
                p.raise();
            }
        }
    }

    fn hide_preview(&mut self) {
        if self.preview_item.take().is_some() {
            if let Some(p) = &self.preview_win {
                p.hide();
            }
        }
    }

    // ---------------------------------------------------------------- list popup

    fn open_list(&mut self) {
        let n = self.history.len();
        if n == 0 {
            return;
        }
        let (w_dip, h_dip) = list_popup::measure(n, &self.settings);
        let dpi = self.monitor.dpi;
        let (w, h) = (dip_to_px(w_dip, dpi), dip_to_px(h_dip, dpi));
        let wa = self.monitor.work;
        let Some(main) = &self.main else { return };
        let anchor = if let Some(o) = self.layout.overflow {
            RECT { left: main.x + dip_to_px(o.x, dpi), top: main.y + dip_to_px(o.y, dpi), right: main.x + dip_to_px(o.right(), dpi), bottom: main.y + dip_to_px(o.bottom(), dpi) }
        } else {
            RECT { left: main.x + main.w / 2, top: main.y, right: main.x + main.w / 2, bottom: main.y + main.h }
        };
        let below = if self.is_island() { !self.settings.island.anchor.is_bottom() } else { self.settings.bar.edge == Edge::Top };
        let mut x = anchor.right - w;
        let mut y = if below { anchor.bottom + 6 } else { anchor.top - 6 - h };
        x = x.clamp(wa.left, (wa.right - w).max(wa.left));
        y = y.clamp(wa.top, (wa.bottom - h).max(wa.top));
        self.list_state.scroll = 0;
        if let Some(l) = &mut self.list_win {
            l.set_bounds(x, y, w, h, dpi);
            l.show();
            l.raise();
        }
        self.render_list();
        self.set_timer(TIMER_LIST_CLOSE, 400);
    }

    fn hide_list(&mut self) {
        if let Some(l) = &self.list_win {
            if l.is_visible() {
                l.hide();
            }
        }
        self.hover_in_list = false;
        self.kill_timer(TIMER_LIST_CLOSE);
    }

    fn list_visible(&self) -> bool {
        self.list_win.as_ref().is_some_and(|l| l.is_visible())
    }

    fn render_list(&mut self) {
        let Some(l) = &self.list_win else { return };
        if !l.is_visible() {
            return;
        }
        let ids = self.visible_ids();
        let (w_dip, h_dip) = l.surface.size_dip();
        let mut p = self.layout_params(true);
        p.max_visible = list_popup::MAX_ROWS;
        p.hint_len = 0.0;
        let area = Rect::new(0.0, 0.0, w_dip, h_dip);
        self.list_layout = items_view::layout(&ids, area, true, &p, self.list_state.scroll);
        let items: Vec<&ClipItem> = self.history.visible();
        let theme = self.theme;
        let s = &self.settings;
        let hint = self.modifier_hint();
        let (hover, hover_close) = if self.hover_in_list { (self.hover, self.hover_close) } else { (None, false) };
        let active = self.history.active;
        let chord = self.chord_held && s.hotkeys.show_badges_on_chord;
        let gfx = &self.gfx;
        let text = &mut self.text;
        let thumbs = &mut self.thumbs;
        let layout = &self.list_layout;
        let mut requests = Vec::new();
        let r = l.surface.render(gfx, |dc| {
            fill_rr(dc, gfx, area, theme.radius.max(6.0), theme.bg_solid.with_alpha(0.98));
            stroke_rr(dc, gfx, area, theme.radius.max(6.0), theme.border, 1.0);
            let mut ctx = App::draw_ctx(gfx, &theme, text, thumbs, s, hover, hover_close, active, chord, hint, None, 1.0);
            items_view::draw_items(dc, &mut ctx, layout, &items);
            requests = std::mem::take(&mut ctx.thumb_requests);
        });
        if let Err(e) = r {
            if is_device_lost(e.code()) {
                self.recover_device();
            }
        }
        self.request_thumbs(requests);
    }

    // ---------------------------------------------------------------- clipboard

    fn on_clipboard_update(&mut self) {
        let seq = unsafe { GetClipboardSequenceNumber() };
        if seq == self.self_set_seq {
            return;
        }
        if self.paused || !self.settings.capture.enabled {
            return;
        }
        let _ = self.to_clip.send(ToClip::Capture { seq });
    }

    fn on_captured(&mut self, raw: RawCapture) {
        if raw.self_set {
            let h = match &raw.content {
                RawContent::Text(t) => hash::hash_text(t),
                RawContent::Files(p) => hash::hash_files(p),
                RawContent::Image(img) => match &img.data {
                    RawImageData::Png(b) => hash::hash_bytes(b"png", b),
                    RawImageData::Bgra { pixels, .. } => hash::hash_bytes(b"bgra", pixels),
                },
            };
            self.history.set_active_by_hash(&h);
            self.invalidate();
            return;
        }
        if raw.skip {
            return;
        }
        let now = self.now();
        let created = now_ms();
        let (content, h, image) = match raw.content {
            RawContent::Text(t) => (ClipContent::text(t.clone()), hash::hash_text(&t), None),
            RawContent::Files(p) => (ClipContent::Files { paths: p.clone() }, hash::hash_files(&p), None),
            RawContent::Image(img) => {
                let h = match &img.data {
                    RawImageData::Png(b) => hash::hash_bytes(b"png", b),
                    RawImageData::Bgra { pixels, .. } => hash::hash_bytes(b"bgra", pixels),
                };
                let has_alpha = matches!(&img.data, RawImageData::Bgra { has_alpha: true, .. });
                (ClipContent::Image { path: None, width: img.width, height: img.height, bytes: 0, has_alpha }, h, Some(img))
            }
        };
        let sensitive = raw.sensitivity.is_sensitive();
        let mut item = ClipItem {
            id: self.history.alloc_id(),
            content,
            created_at_ms: created,
            hash: h,
            source_exe: raw.source_exe,
            sensitivity: raw.sensitivity,
            pinned: false,
            revealed_until: None,
            expires_at: if sensitive && self.settings.privacy.ttl_secs > 0 { Some(now + Duration::from_secs(self.settings.privacy.ttl_secs)) } else { None },
            layout_gen: 0,
        };
        if sensitive {
            if let ClipContent::Text { snippet, .. } = &mut item.content {
                snippet.clear();
            }
        }
        let id = item.id;
        let persist = self.settings.capture.persist && (!sensitive || self.settings.privacy.persist_sensitive);
        match self.history.push(item) {
            PushResult::Inserted(id) => {
                if let Some(img) = image {
                    let path = self.paths.images.join(format!("{id}.png"));
                    let px = self.thumb_px();
                    self.image_paths_pending.insert(id, raw.sensitivity);
                    let _ = self.to_img.send(ToImg::Ingest { id, image: img, path, thumb_px: px });
                } else if persist {
                    if let Some(it) = self.history.get(id) {
                        let _ = self.to_db.send(ToDb::Insert(db::Row::from_item(it)));
                    }
                }
                self.history.active = Some(id);
                self.trim_history();
            }
            PushResult::Bumped(existing) => {
                self.history.active = Some(existing);
                let _ = self.to_db.send(ToDb::Touch { id: existing, created_at: created });
            }
        }
        let _ = id;
        self.scroll = 0;
        self.schedule_expiry();
        self.on_new_item_shown();
        self.compute_layout();
        if self.is_island() {
            self.relayout();
        }
        self.invalidate();
        self.render_list();
    }

    /// Visual reaction to a new item: show on-demand bar, expand island, flash.
    fn on_new_item_shown(&mut self) {
        let now = self.now();
        let animate = self.animate();
        if let Some(active) = self.history.active {
            let mut a = Anim::fixed(1.0);
            a.go(0.0, Duration::from_millis(700), Easing::OutCubic, now);
            self.flash = Some((active, a));
        }
        if self.is_island() {
            let hold = self.settings.island.expand_on_copy_secs;
            if hold > 0.0 {
                self.island.expand(now, animate, Some(Duration::from_secs_f32(hold)));
                self.set_timer(TIMER_ISLAND_HOLD, (hold * 1000.0) as u32 + 50);
            } else {
                self.island.flash(now);
            }
        } else if self.settings.bar.placement == Placement::OnDemand {
            self.main_shown = true;
            self.update_visibility();
            self.set_timer(TIMER_AUTOHIDE, (self.settings.capture.on_demand_show_secs * 1000.0) as u32);
        }
        self.set_timer(TIMER_ANIM, 16);
    }

    fn trim_history(&mut self) {
        let removed = self.history.trim(self.settings.capture.max_history as usize);
        for it in removed {
            self.forget_item_resources(&it);
            let _ = self.to_db.send(ToDb::Delete(it.id));
        }
    }

    fn forget_item_resources(&mut self, it: &ClipItem) {
        self.thumbs.remove(it.id);
        self.previews.remove(it.id);
        self.text.invalidate_item(it.id);
        self.image_paths_pending.remove(&it.id);
        if let ClipContent::Image { path: Some(p), .. } = &it.content {
            let _ = self.to_img.send(ToImg::DeleteFile(p.clone()));
        }
    }

    fn on_image_ingested(&mut self, id: ItemId, path: std::path::PathBuf, width: u32, height: u32, bytes: u64, has_alpha: bool, thumb: BgraBuf) {
        if id < 0 {
            self.on_shelf_image_ingested(-id, path, width, height, bytes, has_alpha, thumb);
            return;
        }
        let sens = self.image_paths_pending.remove(&id);
        let Some(item) = self.history.get_mut(id) else {
            // Item vanished (trimmed/deleted) meanwhile: drop the file.
            let _ = self.to_img.send(ToImg::DeleteFile(path));
            return;
        };
        item.content = ClipContent::Image { path: Some(path), width, height, bytes, has_alpha };
        let sensitive = sens.is_some_and(|s| s.is_sensitive()) || item.sensitivity.is_sensitive();
        let persist = self.settings.capture.persist && (!sensitive || self.settings.privacy.persist_sensitive);
        if persist {
            let _ = self.to_db.send(ToDb::Insert(db::Row::from_item(item)));
        }
        self.thumbs.insert(&self.gfx.dc, id, &thumb);
        self.invalidate();
        self.render_list();
    }

    fn on_loaded(&mut self, rows: Vec<db::Row>) {
        let items: Vec<ClipItem> = rows.into_iter().filter_map(|r| r.into_item()).collect();
        let keep: Vec<std::path::PathBuf> = items.iter().filter_map(|i| if let ClipContent::Image { path: Some(p), .. } = &i.content { Some(p.clone()) } else { None }).collect();
        self.history.load(items);
        self.loaded = true;
        let _ = self.to_img.send(ToImg::SweepOrphans { dir: self.paths.images.clone(), keep });
        self.relayout();
        self.invalidate();
        log::info!("history loaded: {} items after {:?}", self.history.len(), self.start_time.elapsed());
    }

    fn drain_workers(&mut self) {
        let mut needs_render = false;
        loop {
            let m = match self.from_workers.try_recv() {
                Ok(m) => m,
                Err(_) => break,
            };
            match m {
                ToUi::Captured(raw) => self.on_captured(raw),
                ToUi::CaptureFailed => {}
                ToUi::ImageIngested { id, path, width, height, bytes, has_alpha, thumb } => self.on_image_ingested(id, path, width, height, bytes, has_alpha, thumb),
                ToUi::ImageIngestFailed { id } => {
                    if id < 0 {
                        self.remove_shelf_entry(-id);
                        continue;
                    }
                    self.image_paths_pending.remove(&id);
                    if let Some(it) = self.history.remove(id) {
                        self.forget_item_resources(&it);
                    }
                    self.compute_layout();
                    needs_render = true;
                }
                ToUi::ThumbReady { id, thumb } => {
                    self.thumbs.insert(&self.gfx.dc, id, &thumb);
                    needs_render = true;
                    if self.list_visible() {
                        self.render_list();
                    }
                }
                ToUi::ThumbFailed { id } => {
                    self.thumbs.pending.remove(&id);
                    self.thumbs.failed.insert(id);
                }
                ToUi::PreviewReady { id, image } => {
                    self.previews.insert(&self.gfx.dc, id, &image);
                    if self.preview_item == Some(id) {
                        if id < 0 {
                            self.show_shelf_preview(id);
                        } else {
                            let in_list = self.hover_in_list;
                            self.show_preview(id, in_list);
                        }
                    }
                }
                ToUi::PngReady { id, png, dib, reason } => self.on_png_ready(id, png, dib, reason),
                ToUi::Loaded(rows) => self.on_loaded(rows),
                ToUi::ShelvesLoaded { shelves, items } => self.on_shelves_loaded(shelves, items),
                ToUi::DbError(e) => {
                    log::error!("db: {e}");
                    self.tray.balloon("Clipcywin", t("History database unavailable; running without persistence."));
                }
            }
        }
        if needs_render {
            self.invalidate();
        }
    }

    // ---------------------------------------------------------------- selection / paste

    fn activate_item(&mut self, id: ItemId, action: HotkeyAction) {
        let Some(item) = self.history.get(id).cloned() else { return };
        let target = unsafe { GetForegroundWindow() };
        let skip_hist = self.settings.capture.skip_windows_history;
        let seq = match &item.content {
            ClipContent::Text { text, .. } => writer::put_text(self.core, text, skip_hist),
            ClipContent::Files { paths } => writer::put_files(self.core, paths, skip_hist),
            ClipContent::Image { path: Some(p), .. } => {
                let reason = if action == HotkeyAction::Paste { PngReason::Paste } else { PngReason::CopyOnly };
                let _ = self.to_img.send(ToImg::LoadPng { id, path: p.clone(), reason });
                return;
            }
            ClipContent::Image { path: None, .. } => return,
        };
        self.after_clipboard_set(id, seq, action, target);
    }

    fn on_png_ready(&mut self, id: ItemId, png: Vec<u8>, dib: Vec<u8>, reason: PngReason) {
        let target = unsafe { GetForegroundWindow() };
        let seq = writer::put_image(self.core, &png, &dib, self.settings.capture.skip_windows_history);
        let action = if reason == PngReason::Paste { HotkeyAction::Paste } else { HotkeyAction::CopyOnly };
        self.after_clipboard_set(id, seq, action, target);
    }

    fn after_clipboard_set(&mut self, id: ItemId, seq: Option<u32>, action: HotkeyAction, target: HWND) {
        let Some(seq) = seq else {
            self.tray.balloon("Clipcywin", t("Clipboard is busy; try again."));
            return;
        };
        self.self_set_seq = seq;
        if id > 0 {
            self.history.active = Some(id);
        }
        let now = self.now();
        let mut a = Anim::fixed(1.0);
        a.go(0.0, Duration::from_millis(500), Easing::OutCubic, now);
        self.flash = Some((id, a));
        self.set_timer(TIMER_ANIM, 16);
        self.invalidate();
        if action == HotkeyAction::Paste {
            if !target.is_invalid() && process::is_hwnd_elevated(target) == Some(true) && !process::is_self_elevated() {
                self.tray.balloon(t("Copied"), t("Target window is elevated; press Ctrl+V to paste."));
                return;
            }
            self.set_timer(TIMER_PASTE, self.settings.hotkeys.paste_delay_ms.max(1));
        }
    }

    fn hotkey(&mut self, slot: u32) {
        if let Some(item) = self.history.slot(slot) {
            let id = item.id;
            self.activate_item(id, self.settings.hotkeys.action);
        }
    }

    fn delete_item(&mut self, id: ItemId) {
        if let Some(it) = self.history.remove(id) {
            self.forget_item_resources(&it);
            let _ = self.to_db.send(ToDb::Delete(id));
        }
        if self.hover == Some(id) {
            self.hover = None;
        }
        if self.preview_item == Some(id) {
            self.hide_preview();
        }
        self.compute_layout();
        if self.is_island() {
            self.relayout();
        }
        self.invalidate();
        self.render_list();
    }

    pub fn clear_history(&mut self, keep_pinned: bool) {
        let removed = self.history.clear(keep_pinned);
        for it in removed {
            self.forget_item_resources(&it);
        }
        let _ = self.to_db.send(ToDb::Clear { keep_pinned });
        self.hide_preview();
        self.hide_list();
        self.relayout();
        self.invalidate();
    }

    fn toggle_pin(&mut self, id: ItemId) {
        let mut pinned = None;
        if let Some(it) = self.history.get_mut(id) {
            it.pinned = !it.pinned;
            pinned = Some(it.pinned);
        }
        if let Some(p) = pinned {
            let _ = self.to_db.send(ToDb::SetPinned { id, pinned: p });
            self.compute_layout();
            self.invalidate();
            self.render_list();
        }
    }

    fn toggle_sensitive(&mut self, id: ItemId) {
        let persist = self.settings.capture.persist;
        let persist_sensitive = self.settings.privacy.persist_sensitive;
        let ttl = self.settings.privacy.ttl_secs;
        let now = self.now();
        let mut row = None;
        if let Some(it) = self.history.get_mut(id) {
            if it.sensitivity.is_sensitive() {
                it.sensitivity = Sensitivity::None;
                it.expires_at = None;
                it.revealed_until = None;
                if let ClipContent::Text { text, snippet, .. } = &mut it.content {
                    *snippet = crate::model::item::make_snippet(text, 200);
                }
                it.layout_gen = it.layout_gen.wrapping_add(1);
                if persist {
                    row = Some(db::Row::from_item(it));
                }
            } else {
                it.sensitivity = Sensitivity::Manual;
                it.expires_at = if ttl > 0 { Some(now + Duration::from_secs(ttl)) } else { None };
                if let ClipContent::Text { snippet, .. } = &mut it.content {
                    snippet.clear();
                }
                it.layout_gen = it.layout_gen.wrapping_add(1);
                if persist {
                    if persist_sensitive {
                        row = Some(db::Row::from_item(it));
                    } else {
                        let _ = self.to_db.send(ToDb::Delete(id));
                    }
                }
            }
        }
        if let Some(r) = row {
            let _ = self.to_db.send(ToDb::Insert(r));
        }
        self.text.invalidate_item(id);
        self.schedule_expiry();
        self.invalidate();
        self.render_list();
    }

    fn reveal(&mut self, id: ItemId) {
        let secs = self.settings.privacy.reveal_secs;
        let now = self.now();
        if let Some(it) = self.history.get_mut(id) {
            it.revealed_until = Some(now + Duration::from_secs(secs as u64));
            it.layout_gen = it.layout_gen.wrapping_add(1);
        }
        self.text.invalidate_item(id);
        self.set_timer(TIMER_REVEAL, secs * 1000 + 50);
        self.invalidate();
        self.render_list();
    }

    fn schedule_expiry(&mut self) {
        match self.history.next_expiry() {
            Some(t) => {
                let ms = t.saturating_duration_since(self.now()).as_millis() as u32 + 50;
                self.set_timer(TIMER_EXPIRY, ms.max(100));
            }
            None => self.kill_timer(TIMER_EXPIRY),
        }
    }

    fn expire_now(&mut self) {
        let now = self.now();
        let gone = self.history.expire_sensitive(now);
        let clear = self.settings.privacy.clear_clipboard_on_expire;
        for it in gone {
            let was_active = self.history.active.is_none() && it.hash == self.current_clipboard_hash_guess();
            self.forget_item_resources(&it);
            let _ = self.to_db.send(ToDb::Delete(it.id));
            if clear && was_active {
                if writer::clear(self.core) {
                    self.self_set_seq = unsafe { GetClipboardSequenceNumber() };
                }
            }
        }
        self.schedule_expiry();
        self.compute_layout();
        if self.is_island() {
            self.relayout();
        }
        self.invalidate();
        self.render_list();
    }

    fn current_clipboard_hash_guess(&self) -> hash::Hash {
        // Best effort: the last item we put on the clipboard is the active one.
        [0u8; 32]
    }

    // ---------------------------------------------------------------- settings

    pub fn apply_settings(&mut self, mut new: Settings) {
        new.validate();
        let delta = SettingsDelta::compute(&self.settings, &new);
        self.settings = new;
        if let Err(e) = self.settings.save(&self.paths.settings) {
            log::warn!("save settings: {e}");
        }
        if let Ok(mut c) = self.capture_cfg.lock() {
            *c = CaptureConfig::from_settings(&self.settings);
        }
        self.history.pinned_first = self.settings.layout.pinned_first;
        self.history.dedupe = self.settings.capture.dedupe;
        if delta.hook {
            hook::configure(&hook::HookConfig::from_settings(&self.settings));
        }
        if delta.theme {
            self.theme = Theme::resolve(&self.settings.appearance);
        }
        if delta.text {
            if let Err(e) = self.text.rebuild(&self.settings.appearance, self.settings.preview.font_size) {
                log::warn!("text rebuild: {e}");
            }
        }
        if delta.cache {
            self.thumbs.set_cap(self.settings.general.thumb_cache_mb as usize * 1024 * 1024);
        }
        if delta.language {
            crate::i18n::set(&self.settings.general.language);
            let tip = if self.paused { format!("Clipcywin {}", t("(paused)")) } else { "Clipcywin".to_string() };
                self.tray.set_tip(&tip);
        }
        if delta.autostart {
            autostart::set_enabled(self.settings.general.autostart);
        }
        if delta.db_trim {
            let _ = self.to_db.send(ToDb::Trim { max: self.settings.capture.max_history as usize });
            self.trim_history();
        }
        if delta.capture_guard {
            self.poller.enabled.store(self.settings.hide.on_capture_procs, std::sync::atomic::Ordering::Relaxed);
            if let Ok(mut l) = self.poller.list.lock() {
                *l = self.settings.hide.procs.clone();
            }
        }
        if !self.settings.shelves.enabled && self.shelf_shown.is_some() {
            self.close_shelf();
        }
        if delta.window_rebuild || (self.settings.shelves.enabled && self.shelf_win.is_none()) {
            self.rebuild_windows();
        } else {
            self.apply_window_attrs();
            self.relayout();
        }
        self.thumbs.clear();
        self.text.clear_cache();
        self.update_visibility();
        self.invalidate();
        self.render_list();
    }

    /// Re-read settings.json from disk and apply it (tray "Reload" / `--reload`).
    pub fn reload_settings_file(&mut self) {
        let s = Settings::load(&self.paths.settings);
        log::info!("reloading settings.json (mode {:?}, placement {:?})", s.mode, s.bar.placement);
        self.apply_settings(s);
        bridge::send_settings(self);
    }

    fn open_settings(&mut self) {
        if let Some(w) = &self.settings_win {
            w.focus();
            return;
        }
        let dpi = self.monitor.dpi;
        let (w, h) = (dip_to_px(900.0, dpi), dip_to_px(660.0, dpi));
        let wa = self.monitor.work;
        let x = (wa.left + wa.right) / 2 - w / 2;
        let y = (wa.top + wa.bottom) / 2 - h / 2;
        match SettingsWindow::open(x, y, w, h, self.theme.dark, &self.paths.webview, SETTINGS_HTML, UiWaker::new(self.core)) {
            Ok(win) => {
                log::info!("settings window created");
                self.settings_snapshot = Some(self.settings.clone());
                self.settings_win = Some(win);
            }
            Err(e) => {
                log::error!("settings window: {e}");
                self.tray.balloon("Clipcywin", t("Could not open settings (WebView2 runtime missing?). Edit settings.json in the data folder."));
            }
        }
    }

    pub fn close_settings(&mut self) {
        if let Some(w) = self.settings_win.take() {
            w.close();
        }
        self.settings_snapshot = None;
    }

    // ---------------------------------------------------------------- menus

    fn tray_menu(&self) -> Vec<MenuItem> {
        let bar_label = if self.is_island() { t("Show island") } else { t("Show bar") };
        vec![
            MenuItem::new(CMD_TRAY_TOGGLE, bar_label).checked(self.main.as_ref().is_some_and(|m| m.is_visible())),
            MenuItem::new(CMD_TRAY_MODE_BAR, t("Mode: Bar")).checked(!self.is_island()),
            MenuItem::new(CMD_TRAY_MODE_ISLAND, t("Mode: Island")).checked(self.is_island()),
            MenuItem::sep(),
            MenuItem::new(CMD_TRAY_PAUSE, t("Pause capture")).checked(self.paused),
            MenuItem::new(CMD_TRAY_ALLOW_CAPTURE, t("Allow screenshots temporarily")).checked(self.capture_allowed),
            MenuItem::sep(),
            MenuItem::new(CMD_TRAY_CLEAR, t("Clear history (keep pinned)")),
            MenuItem::new(CMD_TRAY_CLEAR_ALL, t("Clear everything")),
            MenuItem::sep(),
            MenuItem::new(CMD_SHELF_OPEN, t("Open shelf")).disabled(!self.settings.shelves.enabled),
            MenuItem::sep(),
            MenuItem::new(CMD_TRAY_SETTINGS, t("Settings…")),
            MenuItem::new(CMD_TRAY_RELOAD, t("Reload settings.json")),
            MenuItem::new(CMD_TRAY_FOLDER, t("Open data folder")),
            MenuItem::sep(),
            MenuItem::new(CMD_TRAY_EXIT, t("Exit")),
        ]
    }

    fn item_menu(&self, id: ItemId) -> Vec<MenuItem> {
        let Some(it) = self.history.get(id) else { return vec![] };
        let masked = it.is_masked(self.now());
        let mut v = vec![
            MenuItem::new(CMD_PASTE, t("Paste")),
            MenuItem::new(CMD_COPY, t("Copy to clipboard")),
            MenuItem::sep(),
            MenuItem::new(CMD_PIN, if it.pinned { t("Unpin") } else { t("Pin") }),
            MenuItem::new(CMD_SENSITIVE, if it.sensitivity.is_sensitive() { t("Unmark sensitive") } else { t("Mark sensitive") }),
        ];
        if masked && self.settings.privacy.click_to_reveal {
            v.push(MenuItem::new(CMD_REVEAL, t("Reveal for a moment")));
        }
        match &it.content {
            ClipContent::Image { path: Some(_), .. } => v.push(MenuItem::new(CMD_OPEN, t("Open image"))),
            ClipContent::Files { .. } => v.push(MenuItem::new(CMD_OPEN, t("Open containing folder"))),
            _ => {}
        }
        if self.settings.shelves.enabled {
            let name = self.shelves.active_shelf().map(|sh| sh.name.clone()).unwrap_or_else(|| "shelf".into());
            v.push(MenuItem::new(CMD_ADD_TO_SHELF, &format!("{} \"{}\"", t("Add to shelf"), name)));
        }
        v.push(MenuItem::sep());
        v.push(MenuItem::new(CMD_DELETE, t("Delete")));
        v.push(MenuItem::sep());
        v.push(MenuItem::new(CMD_SETTINGS, t("Settings…")));
        v
    }

    fn run_menu_command(&mut self, cmd: u32, id: Option<ItemId>) {
        match (cmd, id) {
            (CMD_PASTE, Some(id)) => self.activate_item(id, HotkeyAction::Paste),
            (CMD_COPY, Some(id)) => self.activate_item(id, HotkeyAction::CopyOnly),
            (CMD_PIN, Some(id)) => self.toggle_pin(id),
            (CMD_SENSITIVE, Some(id)) => self.toggle_sensitive(id),
            (CMD_REVEAL, Some(id)) => self.reveal(id),
            (CMD_OPEN, Some(id)) => {
                if let Some(it) = self.history.get(id) {
                    match &it.content {
                        ClipContent::Image { path: Some(p), .. } => shell_open(&p.to_string_lossy()),
                        ClipContent::Files { paths } => {
                            if let Some(p) = paths.first().and_then(|p| p.parent()) {
                                shell_open(&p.to_string_lossy());
                            }
                        }
                        _ => {}
                    }
                }
            }
            (CMD_DELETE, Some(id)) => self.delete_item(id),
            (CMD_ADD_TO_SHELF, Some(id)) => {
                let shelf = self.shelves.active;
                self.add_history_item_to_shelf(shelf, id);
                self.open_shelf(Some(shelf));
            }
            (CMD_SHELF_REMOVE, Some(id)) if id < 0 => self.remove_shelf_entry(-id),
            (CMD_SHELF_COPY_ALL, _) => self.shelf_copy_all(false),
            (CMD_SHELF_PASTE_ALL, _) => self.shelf_copy_all(true),
            (CMD_SHELF_CLEAR, _) => {
                if let Some(id) = self.shelf_shown.or(Some(self.shelves.active)) {
                    self.clear_shelf(id);
                }
            }
            (CMD_SHELF_NEW, _) => {
                let n = self.shelves.shelves.len() + 1;
                let color = self.shelves.next_color();
                let id = self.new_shelf(format!("{} {n}", t("Shelf")), color);
                self.shelves.active = id;
                self.open_shelf(Some(id));
            }
            (CMD_SHELF_DELETE, _) => {
                if let Some(id) = self.shelf_shown.or(Some(self.shelves.active)) {
                    self.delete_shelf(id);
                }
            }
            (CMD_SHELF_OPEN, _) => self.open_shelf(None),
            (c, _) if c >= CMD_SHELF_SWITCH_BASE => {
                let idx = (c - CMD_SHELF_SWITCH_BASE) as usize;
                if let Some(sh) = self.shelves.shelves.get(idx) {
                    let id = sh.id;
                    self.shelves.active = id;
                    self.open_shelf(Some(id));
                }
            }
            (CMD_SETTINGS, _) | (CMD_TRAY_SETTINGS, _) => self.open_settings(),
            (CMD_TRAY_TOGGLE, _) => self.toggle_main(),
            (CMD_TRAY_MODE_BAR, _) => {
                let mut s = self.settings.clone();
                s.mode = ModeKind::Bar;
                self.apply_settings(s);
                bridge::send_settings(self);
            }
            (CMD_TRAY_MODE_ISLAND, _) => {
                let mut s = self.settings.clone();
                s.mode = ModeKind::Island;
                self.apply_settings(s);
                bridge::send_settings(self);
            }
            (CMD_TRAY_PAUSE, _) => {
                self.paused = !self.paused;
                let tip = if self.paused { format!("Clipcywin {}", t("(paused)")) } else { "Clipcywin".to_string() };
                self.tray.set_tip(&tip);
            }
            (CMD_TRAY_ALLOW_CAPTURE, _) => {
                self.capture_allowed = !self.capture_allowed;
                self.apply_window_attrs();
            }
            (CMD_TRAY_CLEAR, _) => self.clear_history(true),
            (CMD_TRAY_CLEAR_ALL, _) => self.clear_history(false),
            (CMD_TRAY_FOLDER, _) => shell_open(&self.paths.root.to_string_lossy()),
            (CMD_TRAY_RELOAD, _) => self.reload_settings_file(),
            (CMD_TRAY_EXIT, _) => unsafe { PostQuitMessage(0) },
            _ => {}
        }
    }

    fn show_item_menu_deferred(&self, id: ItemId, x: i32, y: i32) {
        let items = self.item_menu(id);
        let owner = self.core;
        defer(move || {
            let cmd = context_menu::show(owner, &items, x, y, false);
            if let Some(c) = cmd {
                with_app(|app| app.run_menu_command(c, Some(id)));
            }
        });
    }

    fn show_tray_menu_deferred(&self, x: i32, y: i32) {
        let items = self.tray_menu();
        let owner = self.core;
        defer(move || {
            let cmd = context_menu::show(owner, &items, x, y, true);
            if let Some(c) = cmd {
                with_app(|app| app.run_menu_command(c, None));
            }
        });
    }

    fn toggle_main(&mut self) {
        if self.is_island() {
            let now = self.now();
            let animate = self.animate();
            if self.island.state == island::IslandState::Expanded {
                self.island.collapse(now, animate);
            } else {
                self.island.expand(now, animate, Some(Duration::from_secs(4)));
                self.set_timer(TIMER_ISLAND_HOLD, 4100);
            }
            self.set_timer(TIMER_ANIM, 16);
        } else if self.settings.bar.placement == Placement::OnDemand {
            self.main_shown = !self.main_shown;
            if self.main_shown {
                self.set_timer(TIMER_AUTOHIDE, (self.settings.capture.on_demand_show_secs * 1000.0) as u32 * 2);
            }
            self.update_visibility();
        } else if let Some(m) = &self.main {
            if m.is_visible() {
                m.hide();
                self.hide_preview();
                self.hide_list();
            } else {
                m.show();
                self.invalidate();
            }
        }
    }

    // ---------------------------------------------------------------- capture guard

    fn hide_for_capture(&mut self) {
        if !self.settings.hide.on_capture_keys {
            return;
        }
        self.hidden_for_capture = true;
        self.update_visibility();
        self.set_timer(TIMER_CAPTURE_HIDE, (self.settings.hide.hide_secs * 1000.0) as u32);
    }

    fn on_foreground(&mut self, hwnd: HWND) {
        let fullscreen = self.settings.capture.hide_on_fullscreen && capture_guard::is_fullscreen_foreground();
        let mut capture = false;
        if self.settings.hide.on_capture_foreground && !hwnd.is_invalid() {
            if let Some(name) = process::exe_name_for_hwnd(hwnd) {
                capture = process::name_in_list(&name, &self.settings.hide.procs);
            }
        }
        let changed = fullscreen != self.fullscreen_hidden || (capture && !self.hidden_for_capture);
        self.fullscreen_hidden = fullscreen;
        if capture {
            self.hidden_for_capture = true;
            self.set_timer(TIMER_CAPTURE_HIDE, (self.settings.hide.hide_secs * 1000.0) as u32);
        }
        if changed {
            self.update_visibility();
        }
    }

    // ---------------------------------------------------------------- clicks vs drags

    /// Mouse button went down on an item: decide between click and OLE drag once the mouse moves.
    fn begin_press(&mut self, id: ItemId, where_: u8, hwnd: HWND, px: i32, py: i32) {
        self.pending_click = Some((id, where_));
        defer(move || {
            let dragging = unsafe { DragDetect(hwnd, POINT { x: px, y: py }) }.as_bool();
            if dragging {
                let data = with_app(|a| {
                    a.pending_click = None;
                    a.hide_preview();
                    a.drag_data_for(id)
                })
                .flatten();
                if let Some(d) = data {
                    with_app(|a| a.drag_in_progress = true);
                    let _ = dnd::start_drag(d);
                    with_app(|a| {
                        a.drag_in_progress = false;
                        a.drag_hover = None;
                        a.invalidate();
                    });
                }
            } else {
                // Released inside the drag rectangle: DragDetect consumed the button-up, so click now.
                with_app(|a| {
                    if a.pending_click.take().is_some_and(|(pid, _)| pid == id) {
                        a.click_item(id, where_ == 1);
                    }
                });
            }
        });
    }

    fn click_item(&mut self, id: ItemId, in_list: bool) {
        if id < 0 {
            self.shelf_entry_click(-id);
            return;
        }
        let masked = self.history.get(id).is_some_and(|i| i.is_masked(self.now()));
        if masked && self.settings.privacy.click_to_reveal {
            self.reveal(id);
        } else {
            self.activate_item(id, self.settings.hotkeys.click_action);
            if in_list {
                self.hide_list();
            }
        }
    }

    /// Everything a drag of `id` (history item, or shelf entry when negative) offers to drop targets.
    fn drag_data_for(&self, id: ItemId) -> Option<DragData> {
        let content = if id < 0 { self.shelves.find_entry(-id).map(|(_, e)| e.content.clone())? } else { self.history.get(id)?.content.clone() };
        if id > 0 && self.history.get(id).is_some_and(|i| i.is_masked(self.now())) {
            return None;
        }
        let mut d = DragData { internal: vec![id], ..Default::default() };
        match content {
            ClipContent::Text { text, .. } => d.text = Some(text.to_string()),
            ClipContent::Files { paths } => d.files = paths,
            ClipContent::Image { path: Some(p), .. } => {
                d.files = vec![p.clone()];
                if let Ok(bytes) = std::fs::read(&p) {
                    if bytes.len() < 64 << 20 {
                        d.png = Some(bytes);
                    }
                }
            }
            ClipContent::Image { path: None, .. } => return None,
        }
        Some(d)
    }

    /// Drag data for a whole shelf: all text joined, all files and images as a file list.
    fn drag_data_for_shelf(&self, shelf_id: i64) -> Option<DragData> {
        let sh = self.shelves.get(shelf_id)?;
        if sh.entries.is_empty() {
            return None;
        }
        let (text, files) = self.shelf_aggregate(sh);
        Some(DragData { internal: sh.entries.iter().map(|e| -e.id).collect(), text, files, png: None, dib: None })
    }

    fn shelf_aggregate(&self, sh: &crate::model::shelf::Shelf) -> (Option<String>, Vec<std::path::PathBuf>) {
        let sep = self.settings.shelves.join_separator.replace("\\n", "\n").replace("\\t", "\t");
        let mut texts: Vec<String> = Vec::new();
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        for e in &sh.entries {
            match &e.content {
                ClipContent::Text { text, .. } => texts.push(text.to_string()),
                ClipContent::Files { paths } => files.extend(paths.iter().cloned()),
                ClipContent::Image { path: Some(p), .. } => files.push(p.clone()),
                ClipContent::Image { path: None, .. } => {}
            }
        }
        (if texts.is_empty() { None } else { Some(texts.join(&sep)) }, files)
    }

    // ---------------------------------------------------------------- drop target callbacks

    fn shelf_button_screen_rect(&self) -> Option<RECT> {
        let m = self.main.as_ref()?;
        let sr = self.layout.shelf?;
        let dpi = m.dpi;
        Some(RECT { left: m.x + dip_to_px(sr.x, dpi), top: m.y + dip_to_px(sr.y, dpi), right: m.x + dip_to_px(sr.right(), dpi), bottom: m.y + dip_to_px(sr.bottom(), dpi) })
    }

    pub fn on_drag_enter(&mut self, zone: Zone, ok: bool, x: i32, y: i32) {
        self.drag_hover = Some(zone);
        self.drag_ok = ok;
        self.kill_timer(TIMER_SHELF_CLOSE);
        if ok {
            self.on_drag_over(zone, x, y);
        }
        self.invalidate();
    }

    pub fn on_drag_over(&mut self, zone: Zone, x: i32, y: i32) -> bool {
        if !self.drag_ok {
            return false;
        }
        if zone == Zone::Bar && self.settings.shelves.open_on_drag && self.shelf_shown.is_none() {
            if let Some(r) = self.shelf_button_screen_rect() {
                if x >= r.left && x < r.right && y >= r.top && y < r.bottom {
                    self.open_shelf(None);
                }
            }
        }
        if self.is_island() && self.island.state != island::IslandState::Expanded {
            let now = self.now();
            let animate = self.animate();
            self.island.expand(now, animate, None);
            self.set_timer(TIMER_ANIM, 16);
        }
        true
    }

    pub fn on_drag_leave(&mut self, zone: Zone) {
        if self.drag_hover == Some(zone) {
            self.drag_hover = None;
        }
        self.invalidate();
        if self.shelf_shown.is_some() && self.settings.shelves.auto_close_secs > 0.0 {
            self.set_timer(TIMER_SHELF_CLOSE, (self.settings.shelves.auto_close_secs * 1000.0) as u32);
        }
    }

    pub fn on_drop(&mut self, zone: Zone, payload: DragPayload, _x: i32, _y: i32) -> bool {
        self.drag_hover = None;
        if payload.is_empty() {
            self.invalidate();
            return false;
        }
        let target = match zone {
            Zone::Shelf => self.shelf_shown.unwrap_or(self.shelves.active),
            Zone::Bar => self.shelves.active,
        };
        if self.shelves.get(target).is_none() {
            let id = self.new_shelf(format!("{} 1", t("Shelf")), crate::model::shelf::PALETTE[0]);
            self.shelves.active = id;
        }
        let target = if self.shelves.get(target).is_some() { target } else { self.shelves.active };
        let added = self.add_payload_to_shelf(target, payload);
        if added > 0 {
            self.open_shelf(Some(target));
        }
        self.invalidate();
        added > 0
    }

    // ---------------------------------------------------------------- shelf data

    fn shelf_row(&self, id: i64) -> Option<db::ShelfRow> {
        let pos = self.shelves.shelves.iter().position(|s| s.id == id)? as i64;
        let sh = self.shelves.get(id)?;
        Some(db::ShelfRow { id: sh.id, name: sh.name.clone(), color: sh.color as i64, position: pos })
    }

    fn persist_entry(&self, shelf_id: i64, entry_id: i64) {
        if let Some(sh) = self.shelves.get(shelf_id) {
            if let Some((pos, e)) = sh.entries.iter().enumerate().find(|(_, e)| e.id == entry_id) {
                let _ = self.to_db.send(ToDb::ShelfItemInsert(db::ShelfItemRow::from_entry(shelf_id, e, pos as i64)));
            }
        }
    }

    pub fn new_shelf(&mut self, name: String, color: u32) -> i64 {
        let id = self.shelves.add_shelf(name, color);
        if let Some(r) = self.shelf_row(id) {
            let _ = self.to_db.send(ToDb::ShelfUpsert(r));
        }
        self.invalidate();
        id
    }

    pub fn update_shelf(&mut self, id: i64, name: Option<String>, color: Option<u32>) {
        if let Some(sh) = self.shelves.get_mut(id) {
            if let Some(n) = name {
                if !n.trim().is_empty() {
                    sh.name = n;
                }
            }
            if let Some(c) = color {
                sh.color = c;
            }
        }
        if let Some(r) = self.shelf_row(id) {
            let _ = self.to_db.send(ToDb::ShelfUpsert(r));
        }
        self.invalidate();
        self.render_shelf();
    }

    pub fn delete_shelf(&mut self, id: i64) {
        if let Some(sh) = self.shelves.remove_shelf(id) {
            for e in &sh.entries {
                self.forget_entry_resources(e);
            }
            let _ = self.to_db.send(ToDb::ShelfDelete(id));
        }
        if self.shelf_shown == Some(id) {
            self.shelf_shown = None;
            if self.shelves.shelves.is_empty() {
                self.close_shelf();
            } else {
                self.open_shelf(None);
            }
        }
        self.invalidate();
    }

    fn forget_entry_resources(&mut self, e: &ShelfEntry) {
        self.thumbs.remove(-e.id);
        self.previews.remove(-e.id);
        self.text.invalidate_item(-e.id);
        if let ClipContent::Image { path: Some(p), .. } = &e.content {
            if p.starts_with(&self.paths.shelves) {
                let _ = self.to_img.send(ToImg::DeleteFile(p.clone()));
            }
        }
    }

    pub fn remove_shelf_entry(&mut self, entry_id: i64) {
        if let Some((_, e)) = self.shelves.remove_entry(entry_id) {
            self.forget_entry_resources(&e);
            let _ = self.to_db.send(ToDb::ShelfItemDelete(entry_id));
        }
        if self.shelf_hover == Some(-entry_id) {
            self.shelf_hover = None;
        }
        self.hide_preview();
        self.relayout_shelf();
        self.invalidate();
    }

    pub fn clear_shelf(&mut self, shelf_id: i64) {
        for e in self.shelves.clear(shelf_id) {
            self.forget_entry_resources(&e);
        }
        let _ = self.to_db.send(ToDb::ShelfClear(shelf_id));
        self.hide_preview();
        self.relayout_shelf();
        self.invalidate();
    }

    fn add_history_item_to_shelf(&mut self, shelf_id: i64, id: ItemId) -> bool {
        let Some(item) = self.history.get(id).cloned() else { return false };
        if item.is_masked(self.now()) {
            return false;
        }
        let content = match item.content {
            ClipContent::Image { path: Some(p), width, height, bytes, has_alpha } => {
                // Copy the file so trimming the history never breaks the shelf.
                let Some(entry_id) = self.shelves.add_entry(shelf_id, ClipContent::Image { path: None, width, height, bytes, has_alpha }, now_ms()) else { return false };
                let dst = self.paths.shelves.join(format!("{entry_id}.png"));
                if std::fs::copy(&p, &dst).is_err() {
                    self.shelves.remove_entry(entry_id);
                    return false;
                }
                if let Some(e) = self.shelves.entry_mut(entry_id) {
                    e.content = ClipContent::Image { path: Some(dst), width, height, bytes, has_alpha };
                }
                self.persist_entry(shelf_id, entry_id);
                self.relayout_shelf();
                return true;
            }
            ClipContent::Image { path: None, .. } => return false,
            other => other,
        };
        if let Some(entry_id) = self.shelves.add_entry(shelf_id, content, now_ms()) {
            self.persist_entry(shelf_id, entry_id);
            self.relayout_shelf();
            return true;
        }
        false
    }

    /// Add dropped data to a shelf. Returns the number of entries created.
    fn add_payload_to_shelf(&mut self, shelf_id: i64, p: DragPayload) -> usize {
        let mut added = 0;
        for id in &p.internal {
            if *id > 0 {
                if self.add_history_item_to_shelf(shelf_id, *id) {
                    added += 1;
                }
            } else {
                // Move an entry between shelves.
                let from = self.shelves.find_entry(-id).map(|(s, _)| s.id);
                if from.is_some_and(|f| f != shelf_id) {
                    if let Some((_, e)) = self.shelves.remove_entry(-id) {
                        let _ = self.to_db.send(ToDb::ShelfItemDelete(e.id));
                        if let Some(nid) = self.shelves.add_entry(shelf_id, e.content, e.created_at_ms) {
                            self.persist_entry(shelf_id, nid);
                            added += 1;
                        }
                    }
                }
            }
        }
        for f in p.files {
            if let Some(entry_id) = self.shelves.add_entry(shelf_id, ClipContent::Files { paths: vec![f] }, now_ms()) {
                self.persist_entry(shelf_id, entry_id);
                added += 1;
            }
        }
        if let Some(t) = p.text {
            if let Some(entry_id) = self.shelves.add_entry(shelf_id, ClipContent::text(t), now_ms()) {
                self.persist_entry(shelf_id, entry_id);
                added += 1;
            }
        }
        let image = if let Some(png) = p.png {
            crate::clipboard::dib::png_dimensions(&png).map(|(w, h)| RawImage { width: w, height: h, data: RawImageData::Png(png) })
        } else if let Some(d) = p.dib {
            crate::clipboard::dib::decode(&d).ok().map(|d| RawImage { width: d.width, height: d.height, data: RawImageData::Bgra { pixels: d.pixels, has_alpha: d.has_alpha } })
        } else {
            None
        };
        if let Some(img) = image {
            let content = ClipContent::Image { path: None, width: img.width, height: img.height, bytes: 0, has_alpha: false };
            if let Some(entry_id) = self.shelves.add_entry(shelf_id, content, now_ms()) {
                let path = self.paths.shelves.join(format!("{entry_id}.png"));
                let px = self.thumb_px();
                let _ = self.to_img.send(ToImg::Ingest { id: -entry_id, image: img, path, thumb_px: px });
                added += 1;
            }
        }
        if added > 0 {
            self.relayout_shelf();
        }
        added
    }

    fn on_shelf_image_ingested(&mut self, entry_id: i64, path: std::path::PathBuf, width: u32, height: u32, bytes: u64, has_alpha: bool, thumb: BgraBuf) {
        let shelf_id = self.shelves.find_entry(entry_id).map(|(s, _)| s.id);
        let Some(shelf_id) = shelf_id else {
            let _ = self.to_img.send(ToImg::DeleteFile(path));
            return;
        };
        if let Some(e) = self.shelves.entry_mut(entry_id) {
            e.content = ClipContent::Image { path: Some(path), width, height, bytes, has_alpha };
        }
        self.persist_entry(shelf_id, entry_id);
        self.thumbs.insert(&self.gfx.dc, -entry_id, &thumb);
        self.render_shelf();
    }

    fn on_shelves_loaded(&mut self, rows: Vec<db::ShelfRow>, items: Vec<db::ShelfItemRow>) {
        let mut shelves: Vec<crate::model::shelf::Shelf> = rows.into_iter().map(|r| crate::model::shelf::Shelf { id: r.id, name: r.name, color: r.color as u32, entries: Vec::new() }).collect();
        for it in items {
            let sid = it.shelf_id;
            if let Some(e) = it.into_entry() {
                if let Some(sh) = shelves.iter_mut().find(|s| s.id == sid) {
                    sh.entries.push(e);
                }
            }
        }
        self.shelves.load(shelves);
        if let Some(id) = self.shelves.ensure_default() {
            if let Some(r) = self.shelf_row(id) {
                let _ = self.to_db.send(ToDb::ShelfUpsert(r));
            }
        }
        self.invalidate();
    }

    fn shelf_entry_click(&mut self, entry_id: i64) {
        let Some((_, e)) = self.shelves.find_entry(entry_id) else { return };
        let target = unsafe { GetForegroundWindow() };
        let skip = self.settings.capture.skip_windows_history;
        let seq = match &e.content {
            ClipContent::Text { text, .. } => writer::put_text(self.core, text, skip),
            ClipContent::Files { paths } => writer::put_files(self.core, paths, skip),
            ClipContent::Image { path: Some(p), .. } => {
                let reason = if self.settings.hotkeys.click_action == HotkeyAction::Paste { PngReason::Paste } else { PngReason::CopyOnly };
                let _ = self.to_img.send(ToImg::LoadPng { id: -entry_id, path: p.clone(), reason });
                return;
            }
            _ => None,
        };
        self.after_clipboard_set(-entry_id, seq, self.settings.hotkeys.click_action, target);
    }

    /// Put the whole shelf on the clipboard (text joined, files as a list); optionally paste.
    pub fn shelf_copy_all(&mut self, paste: bool) {
        let Some(id) = self.shelf_shown.or(Some(self.shelves.active)) else { return };
        let Some(sh) = self.shelves.get(id) else { return };
        let (text, files) = self.shelf_aggregate(sh);
        if text.is_none() && files.is_empty() {
            return;
        }
        let target = unsafe { GetForegroundWindow() };
        let seq = writer::put_multi(self.core, text.as_deref(), &files, self.settings.capture.skip_windows_history);
        self.after_clipboard_set(0, seq, if paste { HotkeyAction::Paste } else { HotkeyAction::CopyOnly }, target);
    }

    // ---------------------------------------------------------------- shelf panel

    fn shelf_panel_size(&self, shelf_id: i64) -> (f32, f32) {
        let s = &self.settings;
        let n = self.shelves.count(shelf_id);
        let pad = s.layout.padding_dip as f32;
        let gap = s.layout.gap_dip as f32;
        let row_h = s.bar.height_dip as f32 - 2.0 * pad;
        let rows = n.clamp(1, s.shelves.panel_rows as usize) as f32;
        let overflow = if n > s.shelves.panel_rows as usize { 28.0 + gap } else { 0.0 };
        let tabs = if self.shelves.shelves.len() > 1 { 30.0 } else { 0.0 };
        let h = 40.0 + tabs + rows * row_h + (rows - 1.0) * gap + 2.0 * pad + overflow;
        (s.shelves.panel_width_dip as f32, h.max(120.0))
    }

    pub fn open_shelf(&mut self, shelf_id: Option<i64>) {
        if !self.settings.shelves.enabled {
            return;
        }
        if self.shelves.shelves.is_empty() {
            let id = self.new_shelf(format!("{} 1", t("Shelf")), crate::model::shelf::PALETTE[0]);
            self.shelves.active = id;
        }
        let id = shelf_id.or(self.shelf_shown).unwrap_or(self.shelves.active);
        let id = if self.shelves.get(id).is_some() { id } else { self.shelves.active };
        if self.shelf_shown != Some(id) {
            self.shelf_scroll = 0;
        }
        self.shelf_shown = Some(id);
        self.shelves.active = id;
        self.kill_timer(TIMER_SHELF_CLOSE);
        self.hide_list();
        self.relayout_shelf();
        self.invalidate();
    }

    pub fn close_shelf(&mut self) {
        self.shelf_shown = None;
        self.shelf_hover = None;
        self.kill_timer(TIMER_SHELF_CLOSE);
        if let Some(w) = &self.shelf_win {
            if w.is_visible() {
                w.hide();
            }
        }
        self.hide_preview();
        self.invalidate();
    }

    /// Position and size the panel next to the shelf button, then render it.
    fn relayout_shelf(&mut self) {
        let Some(id) = self.shelf_shown else { return };
        let (w_dip, h_dip) = self.shelf_panel_size(id);
        let dpi = self.monitor.dpi;
        let (w, h) = (dip_to_px(w_dip, dpi), dip_to_px(h_dip, dpi));
        let wa = self.monitor.work;
        let gap = dip_to_px(6.0, dpi);
        let btn = self.shelf_button_screen_rect().or_else(|| self.main.as_ref().map(|m| RECT { left: m.x + m.w - 1, top: m.y, right: m.x + m.w, bottom: m.y + m.h }));
        let Some(btn) = btn else { return };
        let pos = match self.settings.shelves.panel_position {
            PanelPos::Auto => {
                if self.island_geom.as_ref().is_some_and(|g| g.vertical) {
                    if btn.left < (wa.left + wa.right) / 2 { PanelPos::Right } else { PanelPos::Left }
                } else if (self.is_island() && !self.settings.island.anchor.is_bottom()) || (!self.is_island() && self.settings.bar.edge == Edge::Top) {
                    PanelPos::Bottom
                } else {
                    PanelPos::Top
                }
            }
            p => p,
        };
        let (mut x, mut y) = match self.settings.shelves.window_pos {
            // The user placed the panel: keep it there (clamped to the work area).
            Some((px, py)) => (px, py),
            None => match pos {
                PanelPos::Top | PanelPos::Auto => (btn.right - w, btn.top - gap - h),
                PanelPos::Bottom => (btn.right - w, btn.bottom + gap),
                PanelPos::Left => (btn.left - gap - w, btn.top),
                PanelPos::Right => (btn.right + gap, btn.top),
            },
        };
        x = x.clamp(wa.left, (wa.right - w).max(wa.left));
        y = y.clamp(wa.top, (wa.bottom - h).max(wa.top));
        let radius_px = dip_to_px(self.settings.shelves.corner_radius, dpi);
        if let Some(win) = &mut self.shelf_win {
            win.set_bounds(x, y, w, h, dpi);
            win.set_region(if radius_px > 0 { Some((0, 0, w, h, radius_px)) } else { None });
            if !win.is_visible() {
                win.show();
            }
            win.raise();
        }
        self.render_shelf();
    }

    /// Persist the panel position after the user dragged it.
    fn shelf_moved(&mut self) {
        let Some(win) = &mut self.shelf_win else { return };
        win.sync_position();
        let pos = (win.x, win.y);
        if self.settings.shelves.window_pos != Some(pos) {
            self.settings.shelves.window_pos = Some(pos);
            if let Err(e) = self.settings.save(&self.paths.settings) {
                log::warn!("save settings: {e}");
            }
        }
    }

    fn render_shelf(&mut self) {
        let Some(win) = &self.shelf_win else { return };
        let Some(id) = self.shelf_shown else { return };
        if !win.is_visible() {
            return;
        }
        let Some(sh) = self.shelves.get(id).cloned() else { return };
        let (w_dip, h_dip) = win.surface.size_dip();
        let theme = self.theme;
        let s = &self.settings;
        let pad = s.layout.padding_dip as f32;
        let header = Rect::new(0.0, 0.0, w_dip, 40.0);
        let tabs_h = if self.shelves.shelves.len() > 1 { 30.0 } else { 0.0 };
        let list_area = Rect::new(0.0, 40.0 + tabs_h, w_dip, h_dip - 40.0 - tabs_h);
        let mut p = self.layout_params(true);
        p.max_visible = s.shelves.panel_rows as usize;
        p.hint_len = 0.0;
        p.shelf_len = 0.0;
        let items_owned: Vec<ClipItem> = sh.entries.iter().map(|e| e.as_item()).collect();
        let ids: Vec<ItemId> = items_owned.iter().map(|i| i.id).collect();
        self.shelf_layout = items_view::layout(&ids, list_area, true, &p, self.shelf_scroll);
        let items: Vec<&ClipItem> = items_owned.iter().collect();
        let b = 28.0;
        let close_btn = Rect::new(w_dip - 8.0 - b, 6.0, b, b);
        let clear_btn = Rect::new(close_btn.x - 4.0 - b, 6.0, b, b);
        let copy_btn = Rect::new(clear_btn.x - 4.0 - b, 6.0, b, b);
        let add_btn = Rect::new(copy_btn.x - 4.0 - b, 6.0, b, b);
        let grip = Rect::new(pad, 6.0, 18.0, b);
        let mut tabs: Vec<(i64, Rect)> = Vec::new();
        if tabs_h > 0.0 {
            let mut x = pad;
            for other in &self.shelves.shelves {
                let (tw, _) = self.text.measure(&other.name, Font::Small);
                let r = Rect::new(x, 40.0 + 4.0, tw + 26.0, 22.0);
                tabs.push((other.id, r));
                x += r.w + 6.0;
            }
        }
        self.shelf_ui = ShelfUi { header, grip, add_btn, copy_btn, clear_btn, close_btn, tabs: tabs.clone() };
        let panel_radius = s.shelves.corner_radius;
        let hover = self.shelf_hover;
        let hover_close = self.shelf_hover_close;
        let chord = self.chord_held && s.hotkeys.show_badges_on_chord;
        let drop_hot = self.drag_hover == Some(Zone::Shelf) && self.drag_ok;
        let shown_id = id;
        let shelves_meta: Vec<(i64, String, u32)> = self.shelves.shelves.iter().map(|x| (x.id, x.name.clone(), x.color)).collect();
        let gfx = &self.gfx;
        let text = &mut self.text;
        let thumbs = &mut self.thumbs;
        let layout = &self.shelf_layout;
        let mut requests = Vec::new();
        let r = win.surface.render(gfx, |dc| {
            let area = Rect::new(0.0, 0.0, w_dip, h_dip);
            fill_rr(dc, gfx, area, panel_radius, theme.bg_solid.with_alpha(0.98));
            stroke_rr(dc, gfx, area, panel_radius, if drop_hot { theme.accent } else { theme.border }, if drop_hot { 2.0 } else { 1.0 });
            // grip (drag everything out), color dot, name, count, buttons
            for (gx, gy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0), (0.0, 2.0), (1.0, 2.0)] {
                let (cx, cy) = (grip.x + 5.0 + gx * 7.0, grip.y + 7.0 + gy * 7.0);
                unsafe {
                    dc.FillEllipse(&D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: 1.6, radiusY: 1.6 }, gfx.brush(theme.fg_secondary));
                }
            }
            let dot = Rect::new(grip.right() + 6.0, 14.0, 12.0, 12.0);
            let color = crate::gfx::theme::Color::from_rgb_u32(sh.color);
            unsafe {
                let (cx, cy) = dot.center();
                dc.FillEllipse(&D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: 6.0, radiusY: 6.0 }, gfx.brush(color));
            }
            let title = format!("{}  ·  {}", sh.name, sh.entries.len());
            draw_text_in(dc, text, gfx, &title, Font::BodyBold, Rect::new(dot.right() + 8.0, 0.0, add_btn.x - dot.right() - 12.0, 40.0), theme.fg, false, true);
            for (b, glyph) in [(add_btn, "\u{E710}"), (copy_btn, "\u{E8C8}"), (clear_btn, "\u{E74D}"), (close_btn, "\u{E711}")] {
                fill_rr(dc, gfx, b, 6.0, theme.item_bg);
                draw_text_in(dc, text, gfx, glyph, Font::Icon, b, theme.fg_secondary, false, false);
            }
            // tabs
            for (tid, r) in &tabs {
                let meta = shelves_meta.iter().find(|m| m.0 == *tid);
                let on = *tid == shown_id;
                let c = meta.map(|m| crate::gfx::theme::Color::from_rgb_u32(m.2)).unwrap_or(theme.accent);
                fill_rr(dc, gfx, *r, 11.0, if on { c.with_alpha(0.35) } else { theme.item_bg });
                stroke_rr(dc, gfx, *r, 11.0, if on { c } else { theme.border }, 1.0);
                let tdot = Rect::new(r.x + 8.0, r.y + 7.0, 8.0, 8.0);
                unsafe {
                    let (cx, cy) = tdot.center();
                    dc.FillEllipse(&D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: 4.0, radiusY: 4.0 }, gfx.brush(c));
                }
                if let Some(m) = meta {
                    draw_text_in(dc, text, gfx, &m.1, Font::Small, Rect::new(r.x + 20.0, r.y, r.w - 24.0, r.h), theme.fg, false, true);
                }
            }
            // entries
            let mut ctx = App::draw_ctx(gfx, &theme, text, thumbs, s, hover, hover_close, None, chord, String::new(), None, 1.0);
            if items.is_empty() {
                items_view::draw_empty(dc, &mut ctx, list_area, t("Drop files, text or images here"));
            } else {
                items_view::draw_items(dc, &mut ctx, layout, &items);
            }
            requests = std::mem::take(&mut ctx.thumb_requests);
        });
        if let Err(e) = r {
            if is_device_lost(e.code()) {
                self.recover_device();
            }
        }
        // thumbnails for shelf images
        let px = self.thumb_px();
        for id in requests {
            let path = self.shelves.find_entry(-id).and_then(|(_, e)| if let ClipContent::Image { path: Some(p), .. } = &e.content { Some(p.clone()) } else { None });
            match path {
                Some(p) => {
                    let _ = self.to_img.send(ToImg::Thumb { id, path: p, thumb_px: px });
                }
                None => {
                    self.thumbs.pending.remove(&id);
                }
            }
        }
    }

    fn shelf_menu(&self, entry: Option<i64>) -> Vec<MenuItem> {
        let mut v = Vec::new();
        if let Some(e) = entry {
            let _ = e;
            v.push(MenuItem::new(CMD_SHELF_REMOVE, t("Remove from shelf")));
            v.push(MenuItem::sep());
        }
        v.push(MenuItem::new(CMD_SHELF_PASTE_ALL, t("Paste all")));
        v.push(MenuItem::new(CMD_SHELF_COPY_ALL, t("Copy all")));
        v.push(MenuItem::new(CMD_SHELF_CLEAR, t("Clear shelf")));
        v.push(MenuItem::sep());
        for (i, sh) in self.shelves.shelves.iter().enumerate() {
            v.push(MenuItem::new(CMD_SHELF_SWITCH_BASE + i as u32, &format!("{} ({})", sh.name, sh.entries.len())).checked(Some(sh.id) == self.shelf_shown.or(Some(self.shelves.active))));
        }
        v.push(MenuItem::sep());
        v.push(MenuItem::new(CMD_SHELF_NEW, t("New shelf")));
        v.push(MenuItem::new(CMD_SHELF_DELETE, t("Delete this shelf")));
        v.push(MenuItem::new(CMD_TRAY_SETTINGS, t("Shelf settings…")));
        v
    }

    fn show_shelf_menu_deferred(&self, entry: Option<i64>, x: i32, y: i32) {
        let items = self.shelf_menu(entry);
        let owner = self.core;
        defer(move || {
            let cmd = context_menu::show(owner, &items, x, y, false);
            if let Some(c) = cmd {
                with_app(|app| app.run_menu_command(c, entry.map(|e| -e)));
            }
        });
    }

    fn handle_shelf(&mut self, msg: u32, w: WPARAM, l: LPARAM) -> Option<LRESULT> {
        let Some(win) = &self.shelf_win else { return None };
        let hwnd = win.hwnd;
        let dpi = win.dpi;
        match msg {
            WM_PAINT => {
                unsafe {
                    let _ = ValidateRect(Some(hwnd), None);
                }
                self.render_shelf();
                Some(LRESULT(0))
            }
            WM_ERASEBKGND => Some(LRESULT(1)),
            WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE as isize)),
            WM_NCHITTEST => {
                // The header (minus its buttons and the grip) acts as a caption so the panel can be dragged anywhere.
                let (sx, sy) = lparam_xy(l);
                let x = px_to_dip(sx - win.x, dpi);
                let y = px_to_dip(sy - win.y, dpi);
                let ui = &self.shelf_ui;
                let on_button = ui.grip.contains(x, y) || ui.add_btn.contains(x, y) || ui.copy_btn.contains(x, y) || ui.clear_btn.contains(x, y) || ui.close_btn.contains(x, y);
                if ui.header.contains(x, y) && !on_button {
                    return Some(LRESULT(windows::Win32::UI::WindowsAndMessaging::HTCAPTION as isize));
                }
                Some(LRESULT(HTCLIENT as isize))
            }
            WM_EXITSIZEMOVE => {
                self.shelf_moved();
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                let (px, py) = lparam_xy(l);
                let (x, y) = (px_to_dip(px, dpi), px_to_dip(py, dpi));
                App::track_mouse(hwnd, self.settings.appearance.hover_delay_ms);
                self.kill_timer(TIMER_SHELF_CLOSE);
                let (nh, nc) = match self.shelf_layout.hit(x, y) {
                    Hit::Item(id) | Hit::Badge(id) => (Some(id), false),
                    Hit::Close(id) => (Some(id), true),
                    _ => (None, false),
                };
                if nh != self.shelf_hover || nc != self.shelf_hover_close {
                    if nh != self.shelf_hover {
                        self.hide_preview();
                    }
                    self.shelf_hover = nh;
                    self.shelf_hover_close = nc;
                    self.render_shelf();
                }
                Some(LRESULT(0))
            }
            WM_MOUSEHOVER => {
                if let Some(id) = self.shelf_hover {
                    if !self.shelf_hover_close && self.preview_item != Some(id) {
                        self.show_shelf_preview(id);
                    }
                }
                Some(LRESULT(0))
            }
            WM_MOUSELEAVE => {
                self.shelf_hover = None;
                self.shelf_hover_close = false;
                self.hide_preview();
                self.render_shelf();
                if self.settings.shelves.auto_close_secs > 0.0 {
                    self.set_timer(TIMER_SHELF_CLOSE, (self.settings.shelves.auto_close_secs * 1000.0) as u32);
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                let (px, py) = lparam_xy(l);
                let (x, y) = (px_to_dip(px, dpi), px_to_dip(py, dpi));
                match self.shelf_layout.hit(x, y) {
                    Hit::Item(id) | Hit::Badge(id) => self.begin_press(id, 2, hwnd, px, py),
                    _ => {
                        let ui = self.shelf_ui.clone();
                        if ui.grip.contains(x, y) {
                            // Drag the whole shelf into another application.
                            if let Some(id) = self.shelf_shown {
                                defer(move || {
                                    let dragging = unsafe { DragDetect(hwnd, POINT { x: px, y: py }) }.as_bool();
                                    if dragging {
                                        let data = with_app(|a| a.drag_data_for_shelf(id)).flatten();
                                        if let Some(d) = data {
                                            with_app(|a| a.drag_in_progress = true);
                                            let _ = dnd::start_drag(d);
                                            with_app(|a| {
                                                a.drag_in_progress = false;
                                                a.drag_hover = None;
                                                a.invalidate();
                                            });
                                        }
                                    }
                                });
                            }
                        }
                    }
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let (px, py) = lparam_xy(l);
                let (x, y) = (px_to_dip(px, dpi), px_to_dip(py, dpi));
                let ui = self.shelf_ui.clone();
                if ui.close_btn.contains(x, y) {
                    self.close_shelf();
                    return Some(LRESULT(0));
                }
                if ui.add_btn.contains(x, y) {
                    let n = self.shelves.shelves.len() + 1;
                    let color = self.shelves.next_color();
                    let id = self.new_shelf(format!("{} {n}", t("Shelf")), color);
                    self.shelves.active = id;
                    self.open_shelf(Some(id));
                    return Some(LRESULT(0));
                }
                if ui.copy_btn.contains(x, y) {
                    self.shelf_copy_all(false);
                    return Some(LRESULT(0));
                }
                if ui.clear_btn.contains(x, y) {
                    if let Some(id) = self.shelf_shown {
                        self.clear_shelf(id);
                    }
                    return Some(LRESULT(0));
                }
                for (tid, r) in &ui.tabs {
                    if r.contains(x, y) {
                        let tid = *tid;
                        self.open_shelf(Some(tid));
                        return Some(LRESULT(0));
                    }
                }
                match self.shelf_layout.hit(x, y) {
                    Hit::Item(id) | Hit::Badge(id) => {
                        if self.pending_click.take().is_some_and(|(pid, _)| pid == id) {
                            self.click_item(id, false);
                        }
                    }
                    Hit::Close(id) => self.remove_shelf_entry(-id),
                    Hit::Overflow => {
                        let n = self.shelf_shown.map(|id| self.shelves.count(id)).unwrap_or(0);
                        let rows = self.settings.shelves.panel_rows as usize;
                        self.shelf_scroll = (self.shelf_scroll + rows).min(n.saturating_sub(rows));
                        self.render_shelf();
                    }
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_MBUTTONUP => {
                let (px, py) = lparam_xy(l);
                let (x, y) = (px_to_dip(px, dpi), px_to_dip(py, dpi));
                if let Hit::Item(id) | Hit::Badge(id) | Hit::Close(id) = self.shelf_layout.hit(x, y) {
                    self.remove_shelf_entry(-id);
                }
                Some(LRESULT(0))
            }
            WM_RBUTTONUP => {
                let (px, py) = lparam_xy(l);
                let (x, y) = (px_to_dip(px, dpi), px_to_dip(py, dpi));
                let (sx, sy) = App::cursor_pos();
                let entry = match self.shelf_layout.hit(x, y) {
                    Hit::Item(id) | Hit::Badge(id) | Hit::Close(id) => Some(-id),
                    _ => None,
                };
                self.hide_preview();
                self.show_shelf_menu_deferred(entry, sx, sy);
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                let d = wheel_delta(w);
                let n = self.shelf_shown.map(|id| self.shelves.count(id)).unwrap_or(0);
                let rows = self.settings.shelves.panel_rows as usize;
                let max = n.saturating_sub(rows);
                let cur = self.shelf_scroll as i32 + if d > 0 { -1 } else { 1 };
                self.shelf_scroll = cur.clamp(0, max as i32) as usize;
                self.hide_preview();
                self.render_shelf();
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    fn show_shelf_preview(&mut self, id: ItemId) {
        let Some((_, e)) = self.shelves.find_entry(-id) else { return };
        let item = e.as_item();
        // Reuse the history preview path by temporarily viewing the entry as an item.
        let saved = self.hover_in_list;
        self.hover_in_list = false;
        self.show_preview_for(item, 2);
        self.hover_in_list = saved;
    }

    // ---------------------------------------------------------------- message handling

    fn handle(&mut self, hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> Option<LRESULT> {
        if hwnd == self.core {
            return self.handle_core(msg, w, l);
        }
        if self.main.as_ref().is_some_and(|m| m.hwnd == hwnd) {
            return self.handle_main(msg, w, l);
        }
        if self.list_win.as_ref().is_some_and(|m| m.hwnd == hwnd) {
            return self.handle_list(msg, w, l);
        }
        if self.sentinel.as_ref().is_some_and(|m| m.hwnd == hwnd) {
            return self.handle_sentinel(msg, w, l);
        }
        if self.shelf_win.as_ref().is_some_and(|m| m.hwnd == hwnd) {
            return self.handle_shelf(msg, w, l);
        }
        if self.preview_win.as_ref().is_some_and(|m| m.hwnd == hwnd) {
            return match msg {
                WM_PAINT => {
                    unsafe {
                        let _ = ValidateRect(Some(hwnd), None);
                    }
                    if let Some(id) = self.preview_item {
                        let in_list = self.hover_in_list;
                        self.show_preview(id, in_list);
                    }
                    Some(LRESULT(0))
                }
                WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE as isize)),
                WM_ERASEBKGND => Some(LRESULT(1)),
                _ => None,
            };
        }
        if self.settings_win.as_ref().is_some_and(|s| s.hwnd == hwnd) {
            return match msg {
                WM_SIZE => {
                    if let Some(s) = &self.settings_win {
                        s.resize();
                    }
                    Some(LRESULT(0))
                }
                WM_CLOSE => {
                    self.close_settings();
                    Some(LRESULT(0))
                }
                WM_DPICHANGED => {
                    let rc = unsafe { &*(l.0 as *const RECT) };
                    let hwnd = hwnd;
                    let r = *rc;
                    defer(move || unsafe {
                        let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER | windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE);
                    });
                    Some(LRESULT(0))
                }
                _ => None,
            };
        }
        None
    }

    fn handle_core(&mut self, msg: u32, w: WPARAM, l: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_CLIPBOARDUPDATE => {
                self.on_clipboard_update();
                Some(LRESULT(0))
            }
            WM_APP_UI_WAKE => {
                self.drain_workers();
                Some(LRESULT(0))
            }
            WM_APP_HOTKEY => {
                self.hotkey(w.0 as u32);
                Some(LRESULT(0))
            }
            WM_APP_CHORD => {
                let held = w.0 != 0;
                if held != self.chord_held {
                    self.chord_held = held;
                    if self.is_island() {
                        let now = self.now();
                        let animate = self.animate();
                        if held {
                            self.island.expand(now, animate, None);
                        } else if self.island.wants_collapse(now, false) {
                            self.island.collapse(now, animate);
                        }
                        self.set_timer(TIMER_ANIM, 16);
                    } else if self.settings.bar.placement == Placement::OnDemand {
                        self.update_visibility();
                        if !held && self.main_shown {
                            self.set_timer(TIMER_AUTOHIDE, (self.settings.capture.on_demand_show_secs * 1000.0) as u32);
                        }
                    }
                    self.invalidate();
                    self.render_list();
                }
                Some(LRESULT(0))
            }
            WM_APP_TOGGLE_BAR => {
                self.toggle_main();
                Some(LRESULT(0))
            }
            WM_APP_CAPTURE_KEY => {
                self.hide_for_capture();
                Some(LRESULT(0))
            }
            WM_APP_CAPTURE_PROC => {
                self.capture_proc_running = w.0 != 0;
                self.update_visibility();
                Some(LRESULT(0))
            }
            WM_APP_FOREGROUND => {
                self.on_foreground(HWND(w.0 as *mut _));
                Some(LRESULT(0))
            }
            WM_APP_APPBAR => {
                match w.0 as u32 {
                    1 => {
                        // ABN_POSCHANGED
                        defer(|| {
                            with_app(|app| {
                                app.relayout();
                                app.invalidate();
                            });
                        });
                    }
                    2 => {
                        // ABN_FULLSCREENAPP
                        self.fullscreen_hidden = l.0 != 0;
                        self.update_visibility();
                    }
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_APP_TRAY => {
                let event = (l.0 as u32) & 0xffff;
                let (x, y) = ((w.0 as u32 & 0xffff) as i16 as i32, ((w.0 as u32) >> 16) as i16 as i32);
                match event {
                    WM_CONTEXTMENU | WM_RBUTTONUP => self.show_tray_menu_deferred(x, y),
                    WM_LBUTTONUP | 0x0400 => self.toggle_main(), // NIN_SELECT
                    WM_LBUTTONDBLCLK => self.open_settings(),
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_APP_SETTINGS_MSG => {
                let msgs = self.settings_win.as_ref().map(|s| s.drain()).unwrap_or_default();
                for m in msgs {
                    bridge::handle_message(self, &m);
                }
                Some(LRESULT(0))
            }
            WM_APP_SETTINGS_CLOSED => Some(LRESULT(0)),
            WM_TIMER => {
                self.on_timer(w.0);
                Some(LRESULT(0))
            }
            WM_SETTINGCHANGE | WM_THEMECHANGED | WM_DISPLAYCHANGE => {
                defer(|| {
                    with_app(|app| {
                        app.theme = Theme::resolve(&app.settings.appearance);
                        app.apply_window_attrs();
                        app.relayout();
                        app.invalidate();
                    });
                });
                Some(LRESULT(0))
            }
            WM_DESTROY => {
                unsafe { PostQuitMessage(0) };
                Some(LRESULT(0))
            }
            m if m == self.activate_msg => {
                if self.is_island() {
                    self.toggle_main();
                } else {
                    self.main_shown = true;
                    self.update_visibility();
                    if let Some(w) = &self.main {
                        w.show();
                    }
                }
                Some(LRESULT(0))
            }
            m if m == self.settings_msg => {
                log::info!("open settings requested");
                self.open_settings();
                Some(LRESULT(0))
            }
            m if m == self.toggle_msg => {
                self.toggle_main();
                Some(LRESULT(0))
            }
            m if m == self.reload_msg => {
                self.reload_settings_file();
                Some(LRESULT(0))
            }
            m if m == self.taskbar_created_msg => {
                self.tray.add("Clipcywin");
                defer(|| {
                    with_app(|app| {
                        if app.appbar.is_some() {
                            let hwnd = app.main.as_ref().map(|m| m.hwnd);
                            if let Some(h) = hwnd {
                                let mut ab = AppBar::new(h);
                                ab.register();
                                app.appbar = Some(ab);
                            }
                        }
                        app.relayout();
                        app.invalidate();
                    });
                });
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    fn on_timer(&mut self, id: usize) {
        match id {
            TIMER_ANIM => {
                self.render_main();
            }
            TIMER_AUTOHIDE => {
                self.kill_timer(TIMER_AUTOHIDE);
                let (cx, cy) = App::cursor_pos();
                let over_main = self.main.as_ref().is_some_and(|m| m.is_visible() && m.contains_screen_point(cx, cy));
                if !over_main && !self.chord_held && !self.list_visible() {
                    self.main_shown = false;
                    self.update_visibility();
                } else {
                    self.set_timer(TIMER_AUTOHIDE, 1000);
                }
            }
            TIMER_EXPIRY => {
                self.kill_timer(TIMER_EXPIRY);
                self.expire_now();
            }
            TIMER_REVEAL => {
                self.kill_timer(TIMER_REVEAL);
                let now = self.now();
                let mut any = false;
                let mut ids = Vec::new();
                for it in self.history.iter() {
                    if it.revealed_until.is_some_and(|t| t <= now) {
                        ids.push(it.id);
                    }
                }
                for id in ids {
                    if let Some(it) = self.history.get_mut(id) {
                        it.revealed_until = None;
                        it.layout_gen = it.layout_gen.wrapping_add(1);
                        any = true;
                    }
                    self.text.invalidate_item(id);
                }
                if let Some(t) = self.history.next_reveal_end() {
                    self.set_timer(TIMER_REVEAL, t.saturating_duration_since(now).as_millis() as u32 + 50);
                }
                if any {
                    self.invalidate();
                    self.render_list();
                }
            }
            TIMER_PASTE => {
                self.kill_timer(TIMER_PASTE);
                paste::send_ctrl_v();
            }
            TIMER_CAPTURE_HIDE => {
                self.kill_timer(TIMER_CAPTURE_HIDE);
                self.hidden_for_capture = false;
                // The capture overlay may have left a stale "full screen" reading; re-evaluate.
                self.fullscreen_hidden = self.settings.capture.hide_on_fullscreen && capture_guard::is_fullscreen_foreground();
                self.update_visibility();
            }
            TIMER_FS_CHECK => {
                // While hidden for a full-screen app, poll so we come back even without a foreground event.
                let fs = self.settings.capture.hide_on_fullscreen && capture_guard::is_fullscreen_foreground();
                if fs != self.fullscreen_hidden {
                    self.fullscreen_hidden = fs;
                    self.update_visibility();
                }
                if !self.fullscreen_hidden && !self.hidden_for_capture && !self.capture_proc_running {
                    self.kill_timer(TIMER_FS_CHECK);
                }
            }
            TIMER_ISLAND_HOLD => {
                let now = self.now();
                if self.island.wants_collapse(now, self.chord_held) {
                    self.kill_timer(TIMER_ISLAND_HOLD);
                    let animate = self.animate();
                    self.island.collapse(now, animate);
                    self.hide_preview();
                    self.set_timer(TIMER_ANIM, 16);
                }
            }
            TIMER_SHELF_CLOSE => {
                let (x, y) = App::cursor_pos();
                let over_shelf = self.shelf_win.as_ref().is_some_and(|l| l.contains_screen_point(x, y));
                let over_main = self.main.as_ref().is_some_and(|m| m.contains_screen_point(x, y));
                if !over_shelf && !over_main && !self.drag_in_progress && self.drag_hover.is_none() {
                    self.close_shelf();
                }
            }
            TIMER_LIST_CLOSE => {
                let (x, y) = App::cursor_pos();
                let over_list = self.list_win.as_ref().is_some_and(|l| l.contains_screen_point(x, y));
                let over_main = self.main.as_ref().is_some_and(|m| m.contains_screen_point(x, y));
                if !over_list && !over_main {
                    self.hide_list();
                    if self.hover_in_list {
                        self.hover = None;
                        self.hover_in_list = false;
                        self.hide_preview();
                    }
                }
            }
            _ => {}
        }
    }

    fn track_mouse(hwnd: HWND, hover_ms: u32) {
        let mut t = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE | TME_HOVER, hwndTrack: hwnd, dwHoverTime: hover_ms };
        unsafe {
            let _ = TrackMouseEvent(&mut t);
        }
    }

    fn handle_main(&mut self, msg: u32, w: WPARAM, l: LPARAM) -> Option<LRESULT> {
        let Some(main) = &self.main else { return None };
        let hwnd = main.hwnd;
        let dpi = main.dpi;
        match msg {
            WM_PAINT => {
                unsafe {
                    let _ = ValidateRect(Some(hwnd), None);
                }
                self.render_main();
                Some(LRESULT(0))
            }
            WM_ERASEBKGND => Some(LRESULT(1)),
            WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE as isize)),
            WM_NCHITTEST => {
                let (sx, sy) = lparam_xy(l);
                let x = px_to_dip(sx - main.x, dpi);
                let y = px_to_dip(sy - main.y, dpi);
                let inside = if let Some(g) = &self.island_geom {
                    self.island.rect(g, self.now()).contains(x, y)
                } else if let Some(g) = &self.bar_geom {
                    g.area.contains(x, y)
                } else {
                    false
                };
                Some(LRESULT(if inside { HTCLIENT as isize } else { HTTRANSPARENT as isize }))
            }
            WM_WINDOWPOSCHANGED => {
                if let Some(ab) = &self.appbar {
                    ab.window_pos_changed();
                }
                None
            }
            WM_MOUSEMOVE => {
                let (px, py) = lparam_xy(l);
                let (x, y) = (px_to_dip(px, dpi), px_to_dip(py, dpi));
                App::track_mouse(hwnd, self.settings.appearance.hover_delay_ms);
                if self.is_island() {
                    let now = self.now();
                    if !self.island.hovered {
                        self.island.hovered = true;
                        if self.settings.island.expand_on_hover && self.island.state != island::IslandState::Expanded {
                            let animate = self.animate();
                            self.island.expand(now, animate, None);
                            self.set_timer(TIMER_ANIM, 16);
                        }
                    }
                }
                if self.settings.bar.placement == Placement::OnDemand && !self.is_island() {
                    self.kill_timer(TIMER_AUTOHIDE);
                }
                let hit = self.layout.hit(x, y);
                let (nh, nc) = match hit {
                    Hit::Item(id) | Hit::Badge(id) => (Some(id), false),
                    Hit::Close(id) => (Some(id), true),
                    _ => (None, false),
                };
                if nh != self.hover || nc != self.hover_close || self.hover_in_list {
                    if nh != self.hover {
                        self.hide_preview();
                    }
                    self.hover = nh;
                    self.hover_close = nc;
                    self.hover_in_list = false;
                    self.invalidate();
                }
                Some(LRESULT(0))
            }
            WM_MOUSEHOVER => {
                if let Some(id) = self.hover {
                    if !self.hover_close && self.preview_item != Some(id) {
                        self.show_preview(id, false);
                    }
                }
                Some(LRESULT(0))
            }
            WM_MOUSELEAVE => {
                self.hover = None;
                self.hover_close = false;
                self.hide_preview();
                if self.is_island() {
                    self.island.hovered = false;
                    let now = self.now();
                    if self.island.wants_collapse(now, self.chord_held) {
                        let animate = self.animate();
                        self.island.collapse(now, animate);
                        self.set_timer(TIMER_ANIM, 16);
                    }
                } else if self.settings.bar.placement == Placement::OnDemand && self.main_shown {
                    self.set_timer(TIMER_AUTOHIDE, (self.settings.capture.on_demand_show_secs * 1000.0) as u32);
                }
                self.invalidate();
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                let (px, py) = lparam_xy(l);
                let (x, y) = (px_to_dip(px, dpi), px_to_dip(py, dpi));
                if let Hit::Item(id) | Hit::Badge(id) = self.layout.hit(x, y) {
                    self.begin_press(id, 0, hwnd, px, py);
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let (px, py) = lparam_xy(l);
                let (x, y) = (px_to_dip(px, dpi), px_to_dip(py, dpi));
                match self.layout.hit(x, y) {
                    Hit::Item(id) | Hit::Badge(id) => {
                        if self.pending_click.take().is_some_and(|(pid, _)| pid == id) {
                            self.click_item(id, false);
                        }
                    }
                    Hit::Close(id) => self.delete_item(id),
                    Hit::Overflow => {
                        if self.list_visible() {
                            self.hide_list();
                        } else {
                            self.open_list();
                        }
                    }
                    Hit::Hint => self.open_settings(),
                    Hit::Shelf => {
                        if self.shelf_shown.is_some() {
                            self.close_shelf();
                        } else {
                            self.open_shelf(None);
                        }
                    }
                    Hit::Empty => {}
                }
                Some(LRESULT(0))
            }
            WM_MBUTTONUP => {
                let (px, py) = lparam_xy(l);
                let (x, y) = (px_to_dip(px, dpi), px_to_dip(py, dpi));
                if let Hit::Item(id) | Hit::Badge(id) | Hit::Close(id) = self.layout.hit(x, y) {
                    self.delete_item(id);
                }
                Some(LRESULT(0))
            }
            WM_RBUTTONUP => {
                let (px, py) = lparam_xy(l);
                let (x, y) = (px_to_dip(px, dpi), px_to_dip(py, dpi));
                let (sx, sy) = App::cursor_pos();
                match self.layout.hit(x, y) {
                    Hit::Item(id) | Hit::Badge(id) | Hit::Close(id) => {
                        self.hide_preview();
                        self.show_item_menu_deferred(id, sx, sy);
                    }
                    Hit::Shelf => self.show_shelf_menu_deferred(None, sx, sy),
                    _ => self.show_tray_menu_deferred(sx, sy),
                }
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                let d = wheel_delta(w);
                let n = self.history.len();
                let step: i32 = if d > 0 { -1 } else { 1 };
                let max_scroll = n.saturating_sub(self.layout.visible_len().max(1));
                let ns = (self.scroll as i32 + step).clamp(0, max_scroll as i32) as usize;
                if ns != self.scroll {
                    self.scroll = ns;
                    self.hide_preview();
                    self.compute_layout();
                    self.invalidate();
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    fn handle_list(&mut self, msg: u32, w: WPARAM, l: LPARAM) -> Option<LRESULT> {
        let Some(list) = &self.list_win else { return None };
        let hwnd = list.hwnd;
        let dpi = list.dpi;
        match msg {
            WM_PAINT => {
                unsafe {
                    let _ = ValidateRect(Some(hwnd), None);
                }
                self.render_list();
                Some(LRESULT(0))
            }
            WM_ERASEBKGND => Some(LRESULT(1)),
            WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE as isize)),
            WM_MOUSEMOVE => {
                let (px, py) = lparam_xy(l);
                let (x, y) = (px_to_dip(px, dpi), px_to_dip(py, dpi));
                App::track_mouse(hwnd, self.settings.appearance.hover_delay_ms);
                let hit = self.list_layout.hit(x, y);
                let (nh, nc) = match hit {
                    Hit::Item(id) | Hit::Badge(id) => (Some(id), false),
                    Hit::Close(id) => (Some(id), true),
                    _ => (None, false),
                };
                if nh != self.hover || nc != self.hover_close || !self.hover_in_list {
                    if nh != self.hover {
                        self.hide_preview();
                    }
                    self.hover = nh;
                    self.hover_close = nc;
                    self.hover_in_list = true;
                    self.render_list();
                    self.invalidate();
                }
                Some(LRESULT(0))
            }
            WM_MOUSEHOVER => {
                if let Some(id) = self.hover {
                    if self.hover_in_list && !self.hover_close && self.preview_item != Some(id) {
                        self.show_preview(id, true);
                    }
                }
                Some(LRESULT(0))
            }
            WM_MOUSELEAVE => {
                if self.hover_in_list {
                    self.hover = None;
                    self.hover_close = false;
                    self.hover_in_list = false;
                    self.hide_preview();
                    self.render_list();
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                let (px, py) = lparam_xy(l);
                let (x, y) = (px_to_dip(px, dpi), px_to_dip(py, dpi));
                if let Hit::Item(id) | Hit::Badge(id) = self.list_layout.hit(x, y) {
                    self.begin_press(id, 1, hwnd, px, py);
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let (px, py) = lparam_xy(l);
                let (x, y) = (px_to_dip(px, dpi), px_to_dip(py, dpi));
                match self.list_layout.hit(x, y) {
                    Hit::Item(id) | Hit::Badge(id) => {
                        if self.pending_click.take().is_some_and(|(pid, _)| pid == id) {
                            self.click_item(id, true);
                        }
                    }
                    Hit::Close(id) => self.delete_item(id),
                    Hit::Overflow => {
                        let n = self.history.len();
                        self.list_state.scroll_by(list_popup::MAX_ROWS as i32, n);
                        self.render_list();
                    }
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_MBUTTONUP => {
                let (px, py) = lparam_xy(l);
                let (x, y) = (px_to_dip(px, dpi), px_to_dip(py, dpi));
                if let Hit::Item(id) | Hit::Badge(id) | Hit::Close(id) = self.list_layout.hit(x, y) {
                    self.delete_item(id);
                }
                Some(LRESULT(0))
            }
            WM_RBUTTONUP => {
                let (px, py) = lparam_xy(l);
                let (x, y) = (px_to_dip(px, dpi), px_to_dip(py, dpi));
                let (sx, sy) = App::cursor_pos();
                if let Hit::Item(id) | Hit::Badge(id) | Hit::Close(id) = self.list_layout.hit(x, y) {
                    self.hide_preview();
                    self.show_item_menu_deferred(id, sx, sy);
                }
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                let d = wheel_delta(w);
                let n = self.history.len();
                self.list_state.scroll_by(if d > 0 { -1 } else { 1 }, n);
                self.hide_preview();
                self.render_list();
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    fn handle_sentinel(&mut self, msg: u32, _w: WPARAM, _l: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_MOUSEMOVE => {
                if !self.main_shown {
                    self.main_shown = true;
                    self.update_visibility();
                    self.set_timer(TIMER_AUTOHIDE, (self.settings.capture.on_demand_show_secs * 1000.0) as u32);
                }
                Some(LRESULT(0))
            }
            WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE as isize)),
            WM_PAINT => {
                let Some(s) = &self.sentinel else { return None };
                unsafe {
                    let _ = ValidateRect(Some(s.hwnd), None);
                }
                let gfx = &self.gfx;
                let _ = s.surface.render(gfx, |dc| {
                    // Nearly invisible so the window still receives mouse input.
                    let (w, h) = s.surface.size_dip();
                    fill_rr(dc, gfx, Rect::new(0.0, 0.0, w, h), 0.0, crate::gfx::theme::Color::rgba(0.0, 0.0, 0.0, 0.01));
                });
                Some(LRESULT(0))
            }
            WM_ERASEBKGND => Some(LRESULT(1)),
            _ => None,
        }
    }
}

