//! Bar geometry: where the window goes and where the bar is drawn inside it.

use crate::gfx::{dip_to_px, px_to_dip, Rect};
use crate::settings::{Align, BarWidth, Edge, Placement, Settings};
use crate::ui::monitors::{rect_h, rect_w, MonitorInfo};
use windows::Win32::Foundation::RECT;

#[derive(Clone, Debug, PartialEq)]
pub struct BarGeometry {
    /// Window rect in screen pixels.
    pub window: RECT,
    /// Drawing area of the bar in DIPs relative to the window.
    pub area: Rect,
    pub dpi: u32,
    pub docked: bool,
    pub topmost: bool,
    /// Whether the outer corners should be rounded (floating bars).
    pub floating: bool,
}

/// Compute window and bar rects. `appbar_rect` is the shell-reserved strip for docked mode.
pub fn compute(s: &Settings, mon: &MonitorInfo, appbar_rect: Option<RECT>) -> BarGeometry {
    let dpi = mon.dpi;
    let h_px = dip_to_px(s.bar.height_dip as f32, dpi);
    let margin = dip_to_px(s.bar.margin_dip as f32, dpi);
    let docked = s.bar.placement == Placement::Docked;
    let base = if s.bar.overlap_taskbar && !docked { mon.rect } else { mon.work };

    // Docked: the reserved strip is the bar plus margin on both sides; the bar is drawn inside it.
    // Overlay: the window is the bar itself, offset from the edge by the margin.
    let strip = match (docked, appbar_rect) {
        (true, Some(r)) => r,
        (true, None) => match s.bar.edge {
            Edge::Top => RECT { left: base.left, top: base.top, right: base.right, bottom: base.top + h_px + 2 * margin },
            Edge::Bottom => RECT { left: base.left, top: base.bottom - h_px - 2 * margin, right: base.right, bottom: base.bottom },
        },
        (false, _) => match s.bar.edge {
            Edge::Top => RECT { left: base.left, top: base.top + margin, right: base.right, bottom: base.top + margin + h_px },
            Edge::Bottom => RECT { left: base.left, top: base.bottom - margin - h_px, right: base.right, bottom: base.bottom - margin },
        },
    };
    let bar_y_px = if docked { (rect_h(&strip) - h_px) / 2 } else { 0 };

    let strip_w = rect_w(&strip);
    let usable_w = (strip_w - 2 * margin).max(1);
    let (window, area_px_x, area_px_w, floating) = match s.bar.width {
        BarWidth::Full => {
            if docked {
                (strip, margin, usable_w, margin > 0)
            } else {
                (RECT { left: strip.left + margin, top: strip.top, right: strip.right - margin, bottom: strip.bottom }, 0, usable_w, margin > 0)
            }
        }
        BarWidth::Px(w) => {
            let w_px = dip_to_px(w as f32, dpi).min(usable_w);
            let x = match s.bar.align {
                Align::Left => strip.left + margin,
                Align::Center => strip.left + (strip_w - w_px) / 2,
                Align::Right => strip.right - margin - w_px,
            };
            if docked {
                (strip, x - strip.left, w_px, true)
            } else {
                (RECT { left: x, top: strip.top, right: x + w_px, bottom: strip.bottom }, 0, w_px, true)
            }
        }
    };
    let area = Rect::new(px_to_dip(area_px_x, dpi), px_to_dip(bar_y_px, dpi), px_to_dip(area_px_w, dpi), px_to_dip(h_px, dpi));
    BarGeometry { window, area, dpi, docked, topmost: !docked || s.bar.docked_topmost, floating }
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
    fn overlay_bottom_full() {
        let s = Settings::default();
        let g = compute(&s, &mon(), None);
        assert_eq!(g.window.bottom, 1364);
        assert_eq!(g.window.top, 1364 - 56);
        assert_eq!(g.area.w, 2560.0);
        assert!(!g.docked);
    }

    #[test]
    fn overlay_px_centered() {
        let mut s = Settings::default();
        s.bar.width = BarWidth::Px(1000);
        let g = compute(&s, &mon(), None);
        assert_eq!(g.window.left, 780);
        assert_eq!(g.window.right, 1780);
        assert_eq!(g.area.x, 0.0);
        assert!(g.floating);
    }

    #[test]
    fn docked_px_right_uses_reserved_strip() {
        let mut s = Settings::default();
        s.bar.placement = Placement::Docked;
        s.bar.width = BarWidth::Px(500);
        s.bar.align = Align::Right;
        let strip = RECT { left: 0, top: 1308, right: 2560, bottom: 1364 };
        let g = compute(&s, &mon(), Some(strip));
        assert_eq!(g.window, strip);
        assert_eq!(g.area.x, 2060.0);
        assert_eq!(g.area.w, 500.0);
    }

    #[test]
    fn margin_offsets_overlay_and_docked() {
        let mut s = Settings::default();
        s.bar.margin_dip = 10;
        let g = compute(&s, &mon(), None);
        assert_eq!(g.window.bottom, 1364 - 10);
        assert_eq!(g.window.left, 10);
        assert_eq!(g.window.right, 2550);
        assert!(g.floating);
        s.bar.placement = Placement::Docked;
        let g = compute(&s, &mon(), None);
        assert_eq!(g.window.bottom - g.window.top, 56 + 20);
        assert_eq!(g.area.y, 10.0);
        assert_eq!(g.area.x, 10.0);
    }

    #[test]
    fn dpi_scaling() {
        let mut m = mon();
        m.dpi = 144;
        let s = Settings::default();
        let g = compute(&s, &m, None);
        assert_eq!(g.window.bottom - g.window.top, 84);
        assert!((g.area.h - 56.0).abs() < 0.01);
    }
}
