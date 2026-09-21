//! Settings model, defaults, persistence and change diffing.

pub mod paths;

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ModeKind {
    #[default]
    Bar,
    Island,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    Docked,
    Overlay,
    OnDemand,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    Top,
    Bottom,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind", content = "px")]
pub enum BarWidth {
    Full,
    Px(u32),
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    TopCenter,
    TopLeft,
    TopRight,
    BottomCenter,
    BottomLeft,
    BottomRight,
    LeftEdge,
    RightEdge,
}

impl Anchor {
    pub fn is_vertical(self) -> bool {
        matches!(self, Anchor::LeftEdge | Anchor::RightEdge)
    }
    pub fn is_bottom(self) -> bool {
        matches!(self, Anchor::BottomCenter | Anchor::BottomLeft | Anchor::BottomRight)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum MonitorChoice {
    #[default]
    Primary,
    Index(u32),
    Device(String),
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    Dark,
    Light,
    System,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind", content = "rgb")]
pub enum AccentChoice {
    Windows,
    Custom(u32),
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Backdrop {
    None,
    Acrylic,
    Mica,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ItemStyle {
    /// One line: badge + title / thumbnail, fully rounded.
    #[default]
    Pill,
    /// Two lines with meta info, rounded card.
    Card,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PanelPos {
    #[default]
    Auto,
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct ShelfSettings {
    pub enabled: bool,
    pub panel_position: PanelPos,
    pub panel_width_dip: u32,
    pub panel_rows: u32,
    pub open_on_drag: bool,
    pub auto_close_secs: f32,
    pub join_separator: String,
    /// Corner radius of the shelf panel (independent of the bar).
    pub corner_radius: f32,
    /// Screen position (physical px) once the user dragged the panel; `None` = auto next to the button.
    pub window_pos: Option<(i32, i32)>,
}
impl Default for ShelfSettings {
    fn default() -> Self {
        ShelfSettings {
            enabled: true,
            panel_position: PanelPos::Auto,
            panel_width_dip: 320,
            panel_rows: 8,
            open_on_drag: true,
            auto_close_secs: 0.0,
            join_separator: "\n".into(),
            corner_radius: 10.0,
            window_pos: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct PreviewSettings {
    pub opacity: f32,
    /// Custom background (0xRRGGBB); `None` = theme background.
    pub color: Option<u32>,
    pub blur: bool,
    pub border: bool,
    pub corner_radius: f32,
    /// Maximum size as a percentage of the work area.
    pub max_width_pct: u32,
    pub max_height_pct: u32,
    /// Font size for preview text (0 = same as the bar).
    pub font_size: f32,
}
impl Default for PreviewSettings {
    fn default() -> Self {
        PreviewSettings { opacity: 0.98, color: None, blur: false, border: true, corner_radius: 10.0, max_width_pct: 60, max_height_pct: 75, font_size: 0.0 }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Highlight {
    Border,
    Fill,
    Both,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyAction {
    Paste,
    CopyOnly,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Dedupe {
    Consecutive,
    Anywhere,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SensitiveFormats {
    Skip,
    MarkSensitive,
    Ignore,
}

pub const MOD_WIN: u8 = 1;
pub const MOD_CTRL: u8 = 2;
pub const MOD_ALT: u8 = 4;
pub const MOD_SHIFT: u8 = 8;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct Chord {
    pub modifiers: u8,
    pub vk: u8,
}
impl Default for Chord {
    fn default() -> Self {
        Chord { modifiers: MOD_WIN | MOD_CTRL, vk: 0xC0 /* VK_OEM_3, the backtick key */ }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct BarSettings {
    pub placement: Placement,
    pub edge: Edge,
    pub align: Align,
    pub width: BarWidth,
    pub height_dip: u32,
    /// Distance from the screen edge (and from the sides for floating bars).
    pub margin_dip: u32,
    pub overlap_taskbar: bool,
    pub docked_topmost: bool,
}
impl Default for BarSettings {
    fn default() -> Self {
        BarSettings {
            placement: Placement::Overlay,
            edge: Edge::Bottom,
            align: Align::Center,
            width: BarWidth::Full,
            height_dip: 56,
            margin_dip: 0,
            overlap_taskbar: false,
            docked_topmost: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct IslandSettings {
    pub anchor: Anchor,
    /// Pill opacity (independent of the bar).
    pub opacity: f32,
    /// Custom pill color (0xRRGGBB); `None` = theme background.
    pub color: Option<u32>,
    pub border: bool,
    /// Acrylic blur behind the pill (clipped to its shape).
    pub blur: bool,
    pub collapsed_len_dip: u32,
    pub collapsed_thick_dip: u32,
    pub margin_dip: u32,
    pub expand_on_copy_secs: f32,
    pub expand_on_hover: bool,
}
impl Default for IslandSettings {
    fn default() -> Self {
        IslandSettings {
            anchor: Anchor::TopCenter,
            opacity: 0.96,
            color: None,
            border: true,
            blur: false,
            collapsed_len_dip: 120,
            collapsed_thick_dip: 32,
            margin_dip: 8,
            expand_on_copy_secs: 2.5,
            expand_on_hover: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct LayoutSettings {
    pub item_style: ItemStyle,
    /// Alignment of the items inside the bar (independent of the bar position).
    pub content_align: Align,
    /// Keep the modifier hint at the left edge even when items are centered.
    pub hint_pinned_left: bool,
    pub item_width_dip: u32,
    pub max_visible: u32,
    pub gap_dip: u32,
    pub padding_dip: u32,
    pub pinned_first: bool,
    pub show_badges: bool,
    pub show_meta: bool,
    pub show_source: bool,
    pub show_modifier_hint: bool,
}
impl Default for LayoutSettings {
    fn default() -> Self {
        LayoutSettings {
            item_style: ItemStyle::Pill,
            content_align: Align::Center,
            hint_pinned_left: true,
            item_width_dip: 220,
            max_visible: 10,
            gap_dip: 6,
            padding_dip: 8,
            pinned_first: true,
            show_badges: true,
            show_meta: true,
            show_source: true,
            show_modifier_hint: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Appearance {
    pub theme: Theme,
    pub accent: AccentChoice,
    pub opacity: f32,
    pub backdrop: Backdrop,
    pub highlight: Highlight,
    pub font_family: String,
    pub font_size: f32,
    pub mono_font: String,
    pub corner_radius: f32,
    pub animations: bool,
    pub hover_delay_ms: u32,
    pub hover_previews: bool,
}
impl Default for Appearance {
    fn default() -> Self {
        Appearance {
            theme: Theme::System,
            accent: AccentChoice::Windows,
            opacity: 0.92,
            backdrop: Backdrop::Acrylic,
            highlight: Highlight::Both,
            font_family: "Segoe UI Variable Text".into(),
            font_size: 12.0,
            mono_font: "Cascadia Mono".into(),
            corner_radius: 8.0,
            animations: true,
            hover_delay_ms: 450,
            hover_previews: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Hotkeys {
    pub enabled: bool,
    pub modifiers: u8,
    pub action: HotkeyAction,
    pub paste_delay_ms: u32,
    pub toggle_chord_enabled: bool,
    pub toggle_chord: Chord,
    pub show_badges_on_chord: bool,
    pub click_action: HotkeyAction,
}
impl Default for Hotkeys {
    fn default() -> Self {
        Hotkeys {
            enabled: true,
            modifiers: MOD_WIN | MOD_CTRL,
            action: HotkeyAction::Paste,
            paste_delay_ms: 30,
            toggle_chord_enabled: true,
            toggle_chord: Chord::default(),
            show_badges_on_chord: true,
            click_action: HotkeyAction::CopyOnly,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Capture {
    pub enabled: bool,
    pub text: bool,
    pub images: bool,
    pub files: bool,
    pub max_image_mb: u32,
    pub max_text_kb: u32,
    pub max_history: u32,
    pub persist: bool,
    pub dedupe: Dedupe,
    pub on_demand_show_secs: f32,
    pub edge_trigger: bool,
    pub hide_on_fullscreen: bool,
    pub skip_windows_history: bool,
}
impl Default for Capture {
    fn default() -> Self {
        Capture {
            enabled: true,
            text: true,
            images: true,
            files: true,
            max_image_mb: 64,
            max_text_kb: 1024,
            max_history: 200,
            persist: true,
            dedupe: Dedupe::Anywhere,
            on_demand_show_secs: 3.0,
            edge_trigger: true,
            hide_on_fullscreen: true,
            skip_windows_history: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Privacy {
    pub formats: SensitiveFormats,
    pub process_detection: bool,
    pub process_list: Vec<String>,
    pub heuristic: bool,
    pub threshold: u8,
    pub keys_are_sensitive: bool,
    pub persist_sensitive: bool,
    pub ttl_secs: u64,
    pub click_to_reveal: bool,
    pub reveal_secs: u32,
    pub clear_clipboard_on_expire: bool,
}
impl Default for Privacy {
    fn default() -> Self {
        Privacy {
            formats: SensitiveFormats::MarkSensitive,
            process_detection: true,
            process_list: [
                "1Password.exe",
                "Bitwarden.exe",
                "KeePass.exe",
                "KeePassXC.exe",
                "LastPass.exe",
                "Dashlane.exe",
                "Proton Pass.exe",
                "Enpass.exe",
                "NordPass.exe",
                "RoboForm.exe",
                "Keeper.exe",
                "KeeperPasswordManager.exe",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            heuristic: true,
            threshold: 55,
            keys_are_sensitive: true,
            persist_sensitive: false,
            ttl_secs: 120,
            click_to_reveal: true,
            reveal_secs: 5,
            clear_clipboard_on_expire: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct HideSettings {
    pub exclude_from_capture: bool,
    pub on_capture_keys: bool,
    pub on_capture_foreground: bool,
    pub on_capture_procs: bool,
    pub procs: Vec<String>,
    pub hide_secs: f32,
}
impl Default for HideSettings {
    fn default() -> Self {
        HideSettings {
            exclude_from_capture: true,
            on_capture_keys: true,
            on_capture_foreground: true,
            on_capture_procs: false,
            procs: [
                "SnippingTool.exe",
                "ScreenClippingHost.exe",
                "GameBar.exe",
                "GameBarFTServer.exe",
                "obs64.exe",
                "obs32.exe",
                "ShareX.exe",
                "Flameshot.exe",
                "Greenshot.exe",
                "ScreenToGif.exe",
                "LICEcap.exe",
                "bdcam.exe",
                "Lightshot.exe",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            hide_secs: 5.0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct General {
    pub autostart: bool,
    pub log_level: String,
    pub thumb_cache_mb: u32,
    /// UI language: "en" or "pl".
    pub language: String,
    /// "warp" (software, lowest RAM), "hardware" (GPU), "auto" (= warp).
    pub renderer: String,
}
impl Default for General {
    fn default() -> Self {
        General { autostart: false, log_level: "warn".into(), thumb_cache_mb: 16, language: "en".into(), renderer: "auto".into() }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub mode: ModeKind,
    pub bar: BarSettings,
    pub island: IslandSettings,
    pub monitor: MonitorChoice,
    pub layout: LayoutSettings,
    pub appearance: Appearance,
    pub hotkeys: Hotkeys,
    pub capture: Capture,
    pub privacy: Privacy,
    pub hide: HideSettings,
    pub general: General,
    pub shelves: ShelfSettings,
    pub preview: PreviewSettings,
}

impl Settings {
    pub fn load(path: &Path) -> Settings {
        match std::fs::read(path) {
            Ok(bytes) => match serde_json::from_slice::<Settings>(&bytes) {
                Ok(mut s) => {
                    s.validate();
                    s
                }
                Err(e) => {
                    log::warn!("settings.json invalid ({e}); using defaults");
                    Settings::default()
                }
            },
            Err(_) => Settings::default(),
        }
    }

    /// Write temp file then rename over the target.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let tmp = path.with_extension("json.tmp");
        let data = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(&tmp, data)?;
        let _ = std::fs::remove_file(path);
        std::fs::rename(&tmp, path)
    }

    /// Clamp values into sane ranges.
    pub fn validate(&mut self) {
        self.bar.height_dip = self.bar.height_dip.clamp(36, 160);
        self.bar.margin_dip = self.bar.margin_dip.min(200);
        if let BarWidth::Px(w) = self.bar.width {
            self.bar.width = BarWidth::Px(w.clamp(200, 8000));
        }
        self.island.collapsed_len_dip = self.island.collapsed_len_dip.clamp(48, 600);
        self.island.collapsed_thick_dip = self.island.collapsed_thick_dip.clamp(16, 96);
        self.island.margin_dip = self.island.margin_dip.min(200);
        self.island.opacity = self.island.opacity.clamp(0.2, 1.0);
        self.preview.opacity = self.preview.opacity.clamp(0.2, 1.0);
        self.preview.corner_radius = self.preview.corner_radius.clamp(0.0, 40.0);
        self.preview.max_width_pct = self.preview.max_width_pct.clamp(20, 100);
        self.preview.max_height_pct = self.preview.max_height_pct.clamp(20, 100);
        if self.preview.font_size != 0.0 {
            self.preview.font_size = self.preview.font_size.clamp(8.0, 32.0);
        }
        self.island.expand_on_copy_secs = self.island.expand_on_copy_secs.clamp(0.0, 60.0);
        self.layout.item_width_dip = self.layout.item_width_dip.clamp(80, 600);
        self.layout.max_visible = self.layout.max_visible.clamp(1, 40);
        self.layout.gap_dip = self.layout.gap_dip.min(64);
        self.layout.padding_dip = self.layout.padding_dip.min(64);
        self.appearance.opacity = self.appearance.opacity.clamp(0.2, 1.0);
        self.appearance.font_size = self.appearance.font_size.clamp(8.0, 24.0);
        self.appearance.corner_radius = self.appearance.corner_radius.clamp(0.0, 40.0);
        self.appearance.hover_delay_ms = self.appearance.hover_delay_ms.clamp(50, 5000);
        if self.appearance.font_family.trim().is_empty() {
            self.appearance.font_family = "Segoe UI Variable Text".into();
        }
        if self.appearance.mono_font.trim().is_empty() {
            self.appearance.mono_font = "Cascadia Mono".into();
        }
        if self.hotkeys.modifiers == 0 {
            self.hotkeys.modifiers = MOD_WIN | MOD_CTRL;
        }
        self.hotkeys.paste_delay_ms = self.hotkeys.paste_delay_ms.min(1000);
        self.capture.max_history = self.capture.max_history.clamp(1, 5000);
        self.capture.max_image_mb = self.capture.max_image_mb.clamp(1, 512);
        self.capture.max_text_kb = self.capture.max_text_kb.clamp(1, 16384);
        self.capture.on_demand_show_secs = self.capture.on_demand_show_secs.clamp(0.5, 60.0);
        self.privacy.threshold = self.privacy.threshold.clamp(10, 100);
        self.privacy.reveal_secs = self.privacy.reveal_secs.clamp(1, 120);
        self.hide.hide_secs = self.hide.hide_secs.clamp(1.0, 60.0);
        self.general.thumb_cache_mb = self.general.thumb_cache_mb.clamp(2, 256);
        if self.general.language != "pl" {
            self.general.language = "en".into();
        }
        self.shelves.panel_width_dip = self.shelves.panel_width_dip.clamp(160, 900);
        self.shelves.panel_rows = self.shelves.panel_rows.clamp(2, 30);
        self.shelves.auto_close_secs = self.shelves.auto_close_secs.clamp(0.0, 120.0);
        self.shelves.corner_radius = self.shelves.corner_radius.clamp(0.0, 40.0);
    }

    pub fn log_level(&self) -> log::LevelFilter {
        match self.general.log_level.as_str() {
            "off" => log::LevelFilter::Off,
            "error" => log::LevelFilter::Error,
            "info" => log::LevelFilter::Info,
            "debug" => log::LevelFilter::Debug,
            "trace" => log::LevelFilter::Trace,
            _ => log::LevelFilter::Warn,
        }
    }
}

/// What changed between two settings, so `App::apply_settings` can do the minimum.
#[derive(Default, Debug, Clone, Copy)]
pub struct SettingsDelta {
    pub window_rebuild: bool,
    #[allow(dead_code)]
    pub relayout: bool,
    pub theme: bool,
    pub text: bool,
    pub hook: bool,
    pub capture_guard: bool,
    pub autostart: bool,
    pub db_trim: bool,
    pub cache: bool,
    pub language: bool,
}

impl SettingsDelta {
    pub fn compute(old: &Settings, new: &Settings) -> SettingsDelta {
        SettingsDelta {
            window_rebuild: old.mode != new.mode
                || old.bar.placement != new.bar.placement
                || old.bar.edge != new.bar.edge
                || old.bar.docked_topmost != new.bar.docked_topmost
                || old.monitor != new.monitor
                || old.island.anchor != new.island.anchor
                || old.appearance.backdrop != new.appearance.backdrop
                || old.hide.exclude_from_capture != new.hide.exclude_from_capture
                || old.capture.edge_trigger != new.capture.edge_trigger,
            relayout: old.bar != new.bar
                || old.island != new.island
                || old.layout != new.layout
                || old.appearance != new.appearance
                || old.shelves != new.shelves
                || old.preview != new.preview,
            theme: old.appearance != new.appearance || old.island != new.island || old.preview != new.preview,
            text: old.appearance.font_family != new.appearance.font_family
                || old.appearance.font_size != new.appearance.font_size
                || old.appearance.mono_font != new.appearance.mono_font
                || old.preview.font_size != new.preview.font_size
                || old.layout != new.layout,
            hook: old.hotkeys != new.hotkeys || old.hide.on_capture_keys != new.hide.on_capture_keys,
            capture_guard: old.hide != new.hide,
            autostart: old.general.autostart != new.general.autostart,
            db_trim: old.capture.max_history != new.capture.max_history || old.capture.persist != new.capture.persist,
            cache: old.general.thumb_cache_mb != new.general.thumb_cache_mb,
            language: old.general.language != new.general.language,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_unknown_fields() {
        let s = Settings::default();
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
        let partial = r#"{"bar":{"height_dip":80,"bogus":1},"unknown_section":{"x":1}}"#;
        let p: Settings = serde_json::from_str(partial).unwrap();
        assert_eq!(p.bar.height_dip, 80);
        assert_eq!(p.layout, LayoutSettings::default());
    }

    #[test]
    fn validate_clamps() {
        let mut s = Settings::default();
        s.bar.height_dip = 1;
        s.appearance.opacity = 5.0;
        s.hotkeys.modifiers = 0;
        s.validate();
        assert_eq!(s.bar.height_dip, 36);
        assert_eq!(s.appearance.opacity, 1.0);
        assert_eq!(s.hotkeys.modifiers, MOD_WIN | MOD_CTRL);
    }

    #[test]
    fn delta_detects_rebuild() {
        let a = Settings::default();
        let mut b = a.clone();
        b.mode = ModeKind::Island;
        let d = SettingsDelta::compute(&a, &b);
        assert!(d.window_rebuild);
        let mut c = a.clone();
        c.appearance.opacity = 0.5;
        let d = SettingsDelta::compute(&a, &c);
        assert!(!d.window_rebuild && d.theme && d.relayout);
    }
}
