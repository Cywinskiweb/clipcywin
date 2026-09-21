//! Dynamic-island geometry and expand/collapse state machine.

use crate::gfx::anim::{Anim, Easing};
use crate::gfx::{dip_to_px, px_to_dip, Rect};
use crate::settings::{Anchor, Settings};
use crate::ui::monitors::MonitorInfo;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::RECT;

/// Extra room around the expanded pill so overshoot never clips.
const OVERSHOOT_PAD: f32 = 10.0;

#[derive(Clone, Debug, PartialEq)]
pub struct IslandGeometry {
    pub window: RECT,
    /// Expanded pill rect in DIPs relative to the window.
    pub expanded: Rect,
    /// Collapsed pill rect in DIPs relative to the window.
    pub collapsed: Rect,
    pub vertical: bool,
    pub dpi: u32,
}

/// Compute geometry for `n_items` visible items.
pub fn compute(s: &Settings, mon: &MonitorInfo, n_items: usize, hint_len: f32) -> IslandGeometry {
    let dpi = mon.dpi;
    let vertical = s.island.anchor.is_vertical();
    let item_len = s.layout.item_width_dip as f32;
    let gap = s.layout.gap_dip as f32;
    let pad = s.layout.padding_dip as f32;
    let n = n_items.clamp(1, s.layout.max_visible as usize) as f32;
    let hint = if s.layout.show_modifier_hint && !vertical && hint_len > 0.0 { hint_len + gap } else { 0.0 };
    let thick = s.bar.height_dip as f32;
    let shelf = if s.shelves.enabled { (thick - 2.0 * pad).max(24.0) + gap } else { 0.0 };
    // Rows are as tall as in the bar: the island is exactly one bar height thick, padding included.
    let row = (thick - 2.0 * pad).max(16.0);
    let (exp_w, exp_h) = if vertical {
        (item_len + 2.0 * pad, (n * row + (n - 1.0) * gap + 2.0 * pad + shelf).max(120.0))
    } else {
        ((hint + shelf + n * item_len + (n - 1.0) * gap + 2.0 * pad).max(s.island.collapsed_len_dip as f32 + 40.0), thick)
    };
    let col_len = s.island.collapsed_len_dip as f32;
    let col_thick = s.island.collapsed_thick_dip as f32;
    let (col_w, col_h) = if vertical { (col_thick, col_len) } else { (col_len, col_thick) };

    let win_w = exp_w + 2.0 * OVERSHOOT_PAD;
    let win_h = exp_h + 2.0 * OVERSHOOT_PAD;
    let win_w_px = dip_to_px(win_w, dpi);
    let win_h_px = dip_to_px(win_h, dpi);
    let margin = dip_to_px(s.island.margin_dip as f32, dpi);
    let work = mon.work;
    let ov = dip_to_px(OVERSHOOT_PAD, dpi);
    let top_y = work.top + margin - ov;
    let bottom_y = work.bottom - margin - win_h_px + ov;
    let (x, y) = match s.island.anchor {
        Anchor::TopCenter => ((work.left + work.right) / 2 - win_w_px / 2, top_y),
        Anchor::TopLeft => (work.left + margin - ov, top_y),
        Anchor::TopRight => (work.right - margin - win_w_px + ov, top_y),
        Anchor::BottomCenter => ((work.left + work.right) / 2 - win_w_px / 2, bottom_y),
        Anchor::BottomLeft => (work.left + margin - ov, bottom_y),
        Anchor::BottomRight => (work.right - margin - win_w_px + ov, bottom_y),
        Anchor::LeftEdge => (work.left + margin - ov, (work.top + work.bottom) / 2 - win_h_px / 2),
        Anchor::RightEdge => (work.right - margin - win_w_px + ov, (work.top + work.bottom) / 2 - win_h_px / 2),
    };
    let window = RECT { left: x, top: y, right: x + win_w_px, bottom: y + win_h_px };
    let ww = px_to_dip(win_w_px, dpi);
    let wh = px_to_dip(win_h_px, dpi);

    // Anchor the pills to the edge they hang from; they grow away from it.
    let exp_y = if vertical { (wh - exp_h) / 2.0 } else if s.island.anchor.is_bottom() { wh - OVERSHOOT_PAD - exp_h } else { OVERSHOOT_PAD };
    let col_y = if vertical { (wh - col_h) / 2.0 } else if s.island.anchor.is_bottom() { wh - OVERSHOOT_PAD - col_h } else { OVERSHOOT_PAD };
    let expanded = match s.island.anchor {
        Anchor::TopCenter | Anchor::BottomCenter => Rect::new((ww - exp_w) / 2.0, exp_y, exp_w, exp_h),
        Anchor::TopLeft | Anchor::BottomLeft | Anchor::LeftEdge => Rect::new(OVERSHOOT_PAD, exp_y, exp_w, exp_h),
        Anchor::TopRight | Anchor::BottomRight | Anchor::RightEdge => Rect::new(ww - OVERSHOOT_PAD - exp_w, exp_y, exp_w, exp_h),
    };
    let collapsed = match s.island.anchor {
        Anchor::TopCenter | Anchor::BottomCenter => Rect::new((ww - col_w) / 2.0, col_y, col_w, col_h),
        Anchor::TopLeft | Anchor::BottomLeft | Anchor::LeftEdge => Rect::new(OVERSHOOT_PAD, col_y, col_w, col_h),
        Anchor::TopRight | Anchor::BottomRight | Anchor::RightEdge => Rect::new(ww - OVERSHOOT_PAD - col_w, col_y, col_w, col_h),
    };
    IslandGeometry { window, expanded, collapsed, vertical, dpi }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IslandState {
    Collapsed,
    Expanded,
}

pub struct Island {
    pub state: IslandState,
    /// 0 = collapsed, 1 = expanded
    pub t: Anim,
    pub hold_until: Option<Instant>,
    pub hovered: bool,
    pub pulse: Anim,
}

impl Island {
    pub fn new() -> Island {
        Island { state: IslandState::Collapsed, t: Anim::fixed(0.0), hold_until: None, hovered: false, pulse: Anim::fixed(0.0) }
    }

    pub fn expand(&mut self, now: Instant, animate: bool, hold: Option<Duration>) {
        self.state = IslandState::Expanded;
        self.t.go(1.0, if animate { Duration::from_millis(220) } else { Duration::ZERO }, Easing::OutBack, now);
        if let Some(h) = hold {
            let until = now + h;
            self.hold_until = Some(self.hold_until.map_or(until, |u| u.max(until)));
        }
    }

    pub fn collapse(&mut self, now: Instant, animate: bool) {
        self.state = IslandState::Collapsed;
        self.hold_until = None;
        self.t.go(0.0, if animate { Duration::from_millis(180) } else { Duration::ZERO }, Easing::InOutCubic, now);
    }

    pub fn flash(&mut self, now: Instant) {
        self.pulse.set(1.0);
        self.pulse.go(0.0, Duration::from_millis(600), Easing::OutCubic, now);
    }

    /// Should we collapse now? (hover gone, hold expired, chord not held)
    pub fn wants_collapse(&self, now: Instant, chord_held: bool) -> bool {
        self.state == IslandState::Expanded && !self.hovered && !chord_held && self.hold_until.is_none_or(|u| u <= now)
    }

    pub fn animating(&self, now: Instant) -> bool {
        self.t.active(now) || self.pulse.active(now)
    }

    /// Interpolated pill rect.
    pub fn rect(&self, g: &IslandGeometry, now: Instant) -> Rect {
        let t = self.t.value(now);
        let a = g.collapsed;
        let b = g.expanded;
        Rect::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t, a.w + (b.w - a.w) * t, a.h + (b.h - a.h) * t)
    }

    /// Opacity of the expanded content: fades in over the last 60 % of the expansion.
    pub fn content_alpha(&self, now: Instant) -> f32 {
        let t = self.t.value(now);
        ((t - 0.4) / 0.6).clamp(0.0, 1.0)
    }

    pub fn collapsed_alpha(&self, now: Instant) -> f32 {
        let t = self.t.value(now);
        (1.0 - t / 0.4).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon() -> MonitorInfo {
        MonitorInfo {
            handle: 0,
            index: 0,
            device: "d".into(),
            rect: RECT { left: 0, top: 0, right: 2560, bottom: 1440 },
            work: RECT { left: 0, top: 0, right: 2560, bottom: 1364 },
            primary: true,
            dpi: 96,
        }
    }

    #[test]
    fn top_center_centered() {
        let mut s = Settings::default();
        s.island.anchor = Anchor::TopCenter;
        let g = compute(&s, &mon(), 3, 60.0);
        let cx = (g.window.left + g.window.right) / 2;
        assert!((cx - 1280).abs() <= 1);
        assert!(!g.vertical);
        assert!(g.expanded.w > g.collapsed.w);
        assert_eq!(g.window.top, 8 - 10);
    }

    #[test]
    fn bottom_anchor_hangs_from_bottom() {
        let mut s = Settings::default();
        s.island.anchor = Anchor::BottomCenter;
        let g = compute(&s, &mon(), 2, 60.0);
        assert_eq!(g.window.bottom, 1364 - 8 + 10);
        assert!((g.expanded.bottom() - (g.window.bottom - g.window.top) as f32 + OVERSHOOT_PAD).abs() < 0.01);
        assert!((g.collapsed.bottom() - g.expanded.bottom()).abs() < 0.01);
    }

    #[test]
    fn side_edge_vertical() {
        let mut s = Settings::default();
        s.island.anchor = Anchor::RightEdge;
        let g = compute(&s, &mon(), 4, 60.0);
        assert!(g.vertical);
        assert!(g.expanded.h > g.expanded.w);
        assert!(g.window.right <= 2560 + 10);
    }

    #[test]
    fn state_machine() {
        let t0 = Instant::now();
        let mut i = Island::new();
        i.expand(t0, true, Some(Duration::from_secs(2)));
        assert!(!i.wants_collapse(t0 + Duration::from_secs(1), false));
        assert!(i.wants_collapse(t0 + Duration::from_secs(3), false));
        i.hovered = true;
        assert!(!i.wants_collapse(t0 + Duration::from_secs(3), false));
        i.hovered = false;
        i.collapse(t0 + Duration::from_secs(3), false);
        assert_eq!(i.state, IslandState::Collapsed);
        assert_eq!(i.t.value(t0 + Duration::from_secs(4)), 0.0);
    }
}
