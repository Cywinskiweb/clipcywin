//! Shared item strip: layout, hit-testing and drawing for bar, island and list popup.

use crate::gfx::device::Gfx;
use crate::gfx::text::{Font, TextEngine};
use crate::gfx::theme::{Color, Theme};
use crate::gfx::Rect;
use crate::i18n::t as i18n_t;
use crate::images::cache::BitmapCache;
use crate::model::item::{ClipContent, ClipItem, ItemId};
use crate::settings::Highlight;
use std::time::Instant;
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D_RECT_F};
use windows::Win32::Graphics::Direct2D::{
    ID2D1DeviceContext, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
    D2D1_ELLIPSE, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_LAYER_OPTIONS1_NONE, D2D1_LAYER_PARAMETERS1, D2D1_ROUNDED_RECT,
};
use windows_numerics::{Matrix3x2, Vector2};

pub const BADGE: f32 = 18.0;
pub const CLOSE: f32 = 16.0;
pub const CARD_RADIUS: f32 = 6.0;

#[derive(Clone, Copy, Debug)]
pub struct LayoutParams {
    /// Item extent along the strip (width in horizontal mode, height in vertical mode).
    pub item_len: f32,
    pub gap: f32,
    pub padding: f32,
    pub overflow_len: f32,
    /// Extent of the modifier-hint badge at the leading end (0 = none).
    pub hint_len: f32,
    pub max_visible: usize,
    /// 0 = leading, 1 = center, 2 = trailing (horizontal strips only).
    pub align: u8,
    /// Pill style: one line, fully rounded items.
    pub pill: bool,
    /// Extent of the shelf button at the trailing end (0 = none).
    pub shelf_len: f32,
    /// Keep the hint at the leading edge and align only the items.
    pub hint_pinned: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Slot {
    pub id: ItemId,
    pub index: usize,
    pub rect: Rect,
    pub badge: Rect,
    pub close: Rect,
}

#[derive(Clone, Debug, Default)]
pub struct ItemsLayout {
    pub slots: Vec<Slot>,
    pub overflow: Option<Rect>,
    pub hint: Option<Rect>,
    pub shelf: Option<Rect>,
    pub hidden: usize,
    pub vertical: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Item(ItemId),
    Close(ItemId),
    Badge(ItemId),
    Overflow,
    Hint,
    Shelf,
    Empty,
}

/// Compute item slots inside `area`. `ids` yields the display-ordered item ids.
pub fn layout(ids: &[ItemId], area: Rect, vertical: bool, p: &LayoutParams, scroll: usize) -> ItemsLayout {
    let n = ids.len();
    let along = if vertical { area.h } else { area.w };
    let across = if vertical { area.w } else { area.h };
    let inner_along = (along - 2.0 * p.padding).max(0.0);
    let inner_across = (across - 2.0 * p.padding).max(0.0);
    let hint_len = if p.hint_len > 0.0 && !vertical { p.hint_len + p.gap } else { 0.0 };
    let shelf_len = if p.shelf_len > 0.0 { p.shelf_len + p.gap } else { 0.0 };
    let avail = (inner_along - hint_len - shelf_len).max(0.0);

    let per = p.item_len + p.gap;
    let fit_all = ((avail + p.gap) / per).floor().max(0.0) as usize;
    let scroll = scroll.min(n.saturating_sub(1));
    let remaining = n.saturating_sub(scroll);
    let mut visible = fit_all.min(remaining).min(p.max_visible);
    let needs_overflow = remaining > visible || scroll > 0;
    if needs_overflow {
        let with_overflow = ((avail - p.overflow_len).max(0.0) / per).floor() as usize;
        visible = with_overflow.min(remaining).min(p.max_visible);
    }

    // Extent of the aligned block (items + overflow, plus the hint unless it is pinned).
    let mut items_extent = visible as f32 * per - if visible > 0 { p.gap } else { 0.0 };
    if needs_overflow {
        items_extent += if visible > 0 { p.gap } else { 0.0 } + p.overflow_len;
    }
    let pinned = p.hint_pinned && hint_len > 0.0;
    let content = if pinned { items_extent } else { hint_len + items_extent };
    let region_start = p.padding + if pinned { hint_len } else { 0.0 };
    let region_len = (inner_along - shelf_len - if pinned { hint_len } else { 0.0 }).max(0.0);
    let slack = (region_len - content).max(0.0);
    let mut cursor = region_start
        + if vertical {
            0.0
        } else {
            match p.align {
                1 => (slack / 2.0).floor(),
                2 => slack.floor(),
                _ => 0.0,
            }
        };

    let hint = if hint_len > 0.0 {
        if pinned {
            Some(Rect::new(area.x + p.padding, area.y + p.padding, p.hint_len, inner_across))
        } else {
            let r = Rect::new(area.x + cursor, area.y + p.padding, p.hint_len, inner_across);
            cursor += hint_len;
            Some(r)
        }
    } else {
        None
    };

    let mut slots = Vec::with_capacity(visible);
    for i in 0..visible {
        let idx = scroll + i;
        let rect = if vertical {
            Rect::new(area.x + p.padding, area.y + cursor, inner_across, p.item_len)
        } else {
            Rect::new(area.x + cursor, area.y + p.padding, p.item_len, inner_across)
        };
        let (badge, close) = if p.pill {
            let b = (rect.h - 8.0).min(BADGE + 4.0).max(12.0);
            (
                Rect::new(rect.x + 6.0, rect.y + (rect.h - b) / 2.0, b, b),
                Rect::new(rect.right() - CLOSE - 6.0, rect.y + (rect.h - CLOSE) / 2.0, CLOSE, CLOSE),
            )
        } else {
            (Rect::new(rect.x + 4.0, rect.y + 4.0, BADGE, BADGE), Rect::new(rect.right() - CLOSE - 3.0, rect.y + 3.0, CLOSE, CLOSE))
        };
        slots.push(Slot { id: ids[idx], index: idx, rect, badge, close });
        cursor += per;
    }
    let overflow = if needs_overflow {
        Some(if vertical {
            Rect::new(area.x + p.padding, area.y + cursor, inner_across, p.overflow_len.min((area.bottom() - p.padding - shelf_len - (area.y + cursor)).max(0.0)))
        } else {
            Rect::new(area.x + cursor, area.y + p.padding, p.overflow_len.min((area.right() - p.padding - shelf_len - (area.x + cursor)).max(0.0)), inner_across)
        })
    } else {
        None
    };
    let shelf = if shelf_len > 0.0 {
        Some(if vertical {
            let side = p.shelf_len.min(inner_across);
            Rect::new(area.x + p.padding + (inner_across - side) / 2.0, area.bottom() - p.padding - p.shelf_len, side, side)
        } else {
            Rect::new(area.right() - p.padding - p.shelf_len, area.y + p.padding, p.shelf_len, inner_across)
        })
    } else {
        None
    };
    ItemsLayout { slots, overflow, hint, shelf, hidden: n.saturating_sub(scroll + visible), vertical }
}

impl ItemsLayout {
    pub fn hit(&self, x: f32, y: f32) -> Hit {
        if let Some(sr) = self.shelf {
            if sr.contains(x, y) {
                return Hit::Shelf;
            }
        }
        if let Some(h) = self.hint {
            if h.contains(x, y) {
                return Hit::Hint;
            }
        }
        if let Some(o) = self.overflow {
            if o.contains(x, y) {
                return Hit::Overflow;
            }
        }
        for s in &self.slots {
            if s.rect.contains(x, y) {
                if s.close.contains(x, y) {
                    return Hit::Close(s.id);
                }
                if s.badge.contains(x, y) {
                    return Hit::Badge(s.id);
                }
                return Hit::Item(s.id);
            }
        }
        Hit::Empty
    }

    pub fn slot_of(&self, id: ItemId) -> Option<&Slot> {
        self.slots.iter().find(|s| s.id == id)
    }

    pub fn visible_len(&self) -> usize {
        self.slots.len()
    }
}

/// Everything needed to paint items.
pub struct DrawCtx<'a> {
    pub gfx: &'a Gfx,
    pub theme: &'a Theme,
    pub text: &'a mut TextEngine,
    pub thumbs: &'a mut BitmapCache,
    pub hover: Option<ItemId>,
    pub hover_close: bool,
    pub active: Option<ItemId>,
    pub chord_held: bool,
    pub now: Instant,
    pub highlight: Highlight,
    pub show_badges: bool,
    pub show_meta: bool,
    pub show_source: bool,
    pub modifier_hint: String,
    pub flash: Option<(ItemId, f32)>,
    pub content_alpha: f32,
    pub pill: bool,
    pub shelf_count: usize,
    pub shelf_hot: bool,
    pub shelf_color: Option<Color>,
    /// Thumbnails that need to be requested from the image worker (filled while drawing).
    pub thumb_requests: Vec<ItemId>,
}

fn rr(r: Rect, radius: f32) -> D2D1_ROUNDED_RECT {
    // A radius above half the shorter side makes Direct2D draw a distorted shape; clamp to a true pill.
    let radius = radius.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius, radiusY: radius }
}

pub fn fill_rr(dc: &ID2D1DeviceContext, gfx: &Gfx, r: Rect, radius: f32, c: Color) {
    unsafe { dc.FillRoundedRectangle(&rr(r, radius), gfx.brush(c)) }
}

pub fn stroke_rr(dc: &ID2D1DeviceContext, gfx: &Gfx, r: Rect, radius: f32, c: Color, width: f32) {
    unsafe { dc.DrawRoundedRectangle(&rr(r.inset(width / 2.0), radius), gfx.brush(c), width, None) }
}

pub fn draw_text_in(dc: &ID2D1DeviceContext, text: &TextEngine, gfx: &Gfx, s: &str, font: Font, r: Rect, c: Color, wrap: bool, trim: bool) {
    if s.is_empty() || r.w <= 1.0 || r.h <= 1.0 {
        return;
    }
    if let Ok(layout) = text.layout_once(s, font, r.w, r.h, wrap, trim) {
        unsafe {
            dc.DrawTextLayout(Vector2 { X: r.x, Y: r.y }, &layout, gfx.brush(c), D2D1_DRAW_TEXT_OPTIONS_CLIP | D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
        }
    }
}

pub fn draw_text_cached(dc: &ID2D1DeviceContext, ctx: &mut DrawCtx, id: ItemId, slot: u8, gen: u32, s: &str, font: Font, r: Rect, c: Color, wrap: bool) {
    if s.is_empty() || r.w <= 1.0 || r.h <= 1.0 {
        return;
    }
    if let Some(layout) = ctx.text.layout_cached(id, slot, gen, s, font, r.w, r.h, wrap) {
        unsafe {
            dc.DrawTextLayout(Vector2 { X: r.x, Y: r.y }, &layout, ctx.gfx.brush(c), D2D1_DRAW_TEXT_OPTIONS_CLIP | D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
        }
    }
}

fn push_opacity(dc: &ID2D1DeviceContext, alpha: f32) -> bool {
    if alpha >= 0.999 {
        return false;
    }
    let params = D2D1_LAYER_PARAMETERS1 {
        contentBounds: D2D_RECT_F { left: -1e6, top: -1e6, right: 1e6, bottom: 1e6 },
        geometricMask: std::mem::ManuallyDrop::new(None),
        maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
        maskTransform: Matrix3x2::identity(),
        opacity: alpha.clamp(0.0, 1.0),
        opacityBrush: std::mem::ManuallyDrop::new(None),
        layerOptions: D2D1_LAYER_OPTIONS1_NONE,
    };
    unsafe { dc.PushLayer(&params, None) };
    true
}

pub fn draw_items(dc: &ID2D1DeviceContext, ctx: &mut DrawCtx, layout: &ItemsLayout, items: &[&ClipItem]) {
    let pushed = push_opacity(dc, ctx.content_alpha);
    let theme = *ctx.theme;

    if let Some(h) = layout.hint {
        let held = ctx.chord_held;
        let radius = if ctx.pill { h.h / 2.0 } else { CARD_RADIUS };
        fill_rr(dc, ctx.gfx, h, radius, if held { theme.accent } else { theme.badge_bg });
        let fg = if held { theme.on_accent } else { theme.fg_secondary };
        let f = ctx.text.format(Font::Badge).clone();
        unsafe {
            let _ = f.SetTextAlignment(windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT_CENTER);
        }
        let hint = ctx.modifier_hint.clone();
        if let Some(rest) = hint.strip_prefix("Win") {
            // Windows logo (four squares) followed by the remaining modifiers.
            let logo = 12.0_f32;
            let (tw, _) = if rest.is_empty() { (0.0, 0.0) } else { ctx.text.measure(rest, Font::Badge) };
            let total = logo + if rest.is_empty() { 0.0 } else { 3.0 + tw };
            let x0 = h.x + (h.w - total) / 2.0;
            let (_, cy) = h.center();
            draw_win_logo(dc, ctx.gfx, Rect::new(x0, cy - logo / 2.0, logo, logo), fg);
            if !rest.is_empty() {
                let tr = Rect::new(x0 + logo + 3.0, h.y, (h.right() - (x0 + logo + 3.0)).max(0.0), h.h);
                unsafe {
                    let _ = f.SetTextAlignment(windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT_LEADING);
                }
                draw_text_in(dc, ctx.text, ctx.gfx, rest, Font::Badge, tr, fg, false, false);
                unsafe {
                    let _ = f.SetTextAlignment(windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT_CENTER);
                }
            }
        } else {
            draw_text_in(dc, ctx.text, ctx.gfx, &hint, Font::Badge, h, fg, false, false);
        }
    }

    for slot in &layout.slots {
        let Some(item) = items.iter().find(|i| i.id == slot.id) else { continue };
        draw_card(dc, ctx, slot, item, layout.vertical);
    }

    if let Some(o) = layout.overflow {
        let radius = if ctx.pill { o.h.min(o.w) / 2.0 } else { CARD_RADIUS };
        fill_rr(dc, ctx.gfx, o, radius, theme.item_bg);
        let label = if layout.hidden > 0 { format!("+{}", layout.hidden) } else { "…".to_string() };
        let f = ctx.text.format(Font::Badge).clone();
        unsafe {
            let _ = f.SetTextAlignment(windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT_CENTER);
        }
        draw_text_in(dc, ctx.text, ctx.gfx, &label, Font::Badge, o, theme.fg_secondary, false, false);
    }

    if let Some(sr) = layout.shelf {
        draw_shelf_button(dc, ctx, sr);
    }

    if pushed {
        unsafe { dc.PopLayer() };
    }
}

/// Shelf button: rounded square with three dots; gray when the shelf is empty, shelf-colored otherwise.
pub fn draw_shelf_button(dc: &ID2D1DeviceContext, ctx: &mut DrawCtx, r: Rect) {
    let theme = *ctx.theme;
    let radius = if ctx.pill { r.w.min(r.h) / 2.0 } else { CARD_RADIUS };
    let hot = ctx.shelf_hot;
    fill_rr(dc, ctx.gfx, r, radius, if hot { theme.accent.with_alpha(0.35) } else { theme.item_bg });
    stroke_rr(dc, ctx.gfx, r, radius, if hot { theme.accent } else { theme.border }, if hot { 2.0 } else { 1.0 });
    let filled = ctx.shelf_count > 0;
    let fill = match (filled, ctx.shelf_color) {
        (true, Some(c)) => c,
        (true, None) => theme.accent,
        (false, _) => theme.fg_secondary.with_alpha(0.55),
    };
    let size = (r.w.min(r.h) * 0.42).max(10.0);
    let (cx, cy) = r.center();
    let sq = Rect::new(cx - size / 2.0, cy - size / 2.0, size, size);
    fill_rr(dc, ctx.gfx, sq, size * 0.16, fill);
    let dot_r = size * 0.065;
    let dy = cy + size * 0.12;
    let dots = if filled { theme.bg_solid.with_alpha(0.95) } else { theme.bg_solid.with_alpha(0.9) };
    for k in [-1.0f32, 0.0, 1.0] {
        let x = cx + k * size * 0.22;
        unsafe {
            dc.FillEllipse(&D2D1_ELLIPSE { point: Vector2 { X: x, Y: dy }, radiusX: dot_r, radiusY: dot_r }, ctx.gfx.brush(dots));
        }
    }
}

/// Four rounded squares, the shape of the Windows key.
pub fn draw_win_logo(dc: &ID2D1DeviceContext, gfx: &Gfx, r: Rect, c: Color) {
    let gap = r.w * 0.12;
    let sq = (r.w - gap) / 2.0;
    let rad = sq * 0.18;
    for (ix, iy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
        let cell = Rect::new(r.x + ix * (sq + gap), r.y + iy * (sq + gap), sq, sq);
        fill_rr(dc, gfx, cell, rad, c);
    }
}

fn draw_pill(dc: &ID2D1DeviceContext, ctx: &mut DrawCtx, slot: &Slot, item: &ClipItem) {
    let theme = *ctx.theme;
    let r = slot.rect;
    let radius = r.h / 2.0;
    let hovered = ctx.hover == Some(item.id);
    let active = ctx.active == Some(item.id);
    let masked = item.is_masked(ctx.now);

    let mut bg = if hovered { theme.item_hover } else { theme.item_bg };
    if active && matches!(ctx.highlight, Highlight::Fill | Highlight::Both) {
        bg = bg.mix(theme.accent.with_alpha(0.28), 0.6);
    }
    if let Some((fid, t)) = ctx.flash {
        if fid == item.id {
            bg = bg.mix(theme.accent.with_alpha(0.6), t);
        }
    }
    fill_rr(dc, ctx.gfx, r, radius, bg);
    if active && matches!(ctx.highlight, Highlight::Border | Highlight::Both) {
        stroke_rr(dc, ctx.gfx, r, radius, theme.accent, 2.0);
    } else {
        stroke_rr(dc, ctx.gfx, r, radius, theme.border, 1.0);
    }

    let mut x = r.x + 6.0;
    if ctx.show_badges {
        let n = slot.index + 1;
        if n <= 10 {
            let b = slot.badge;
            let held = ctx.chord_held;
            let (cx, cy) = b.center();
            let scale = if held { 1.12 } else { 1.0 };
            unsafe {
                dc.FillEllipse(&D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: b.w / 2.0 * scale, radiusY: b.h / 2.0 * scale }, ctx.gfx.brush(if held { theme.accent } else { theme.badge_bg }));
            }
            let f = ctx.text.format(Font::Badge).clone();
            unsafe {
                let _ = f.SetTextAlignment(windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT_CENTER);
            }
            draw_text_in(dc, ctx.text, ctx.gfx, &(n % 10).to_string(), Font::Badge, b, if held { theme.on_accent } else { theme.badge_fg }, false, false);
            x = b.right() + 8.0;
        }
    }
    let right_reserved = if hovered { CLOSE + 8.0 } else if item.pinned { 18.0 } else { 0.0 };
    let content = Rect::new(x, r.y, (r.right() - radius.min(10.0) - right_reserved - x).max(0.0), r.h);

    if masked {
        let icon = Rect::new(content.x, content.y, 18.0, content.h);
        draw_text_in(dc, ctx.text, ctx.gfx, "\u{E72E}", Font::Icon, icon, theme.fg_secondary, false, false);
        let t = Rect::new(content.x + 22.0, content.y, (content.w - 22.0).max(0.0), content.h);
        draw_text_in(dc, ctx.text, ctx.gfx, "••••••••", Font::Body, t, theme.fg, false, true);
    } else {
        match &item.content {
            ClipContent::Text { looks_like_code, .. } => {
                let snippet = item.display_snippet(ctx.now).unwrap_or("").to_string();
                let font = if *looks_like_code { Font::Mono } else { Font::Body };
                draw_text_cached(dc, ctx, item.id, 0, item.layout_gen, &snippet, font, content, theme.fg, false);
            }
            ClipContent::Image { width, height, .. } => {
                let th = (r.h - 8.0).max(8.0);
                let max_tw = (content.w * 0.6).max(16.0);
                let tw = max_tw.min(th * (*width as f32 / (*height).max(1) as f32)).max(16.0_f32.min(max_tw));
                let thumb_rect = Rect::new(content.x, r.y + 4.0, tw, th);
                draw_thumb(dc, ctx, item.id, thumb_rect, (th / 2.0).min(8.0));
                let tx = thumb_rect.right() + 8.0;
                let tr = Rect::new(tx, content.y, (content.right() - tx).max(0.0), content.h);
                draw_text_in(dc, ctx.text, ctx.gfx, &format!("{} × {}", width, height), Font::Body, tr, theme.fg, false, true);
            }
            ClipContent::Files { paths } => {
                let icon = Rect::new(content.x, content.y, 18.0, content.h);
                let glyph = if paths.len() > 1 { "\u{E8B7}" } else { "\u{E8A5}" };
                draw_text_in(dc, ctx.text, ctx.gfx, glyph, Font::Icon, icon, theme.fg_secondary, false, false);
                let first = paths.first().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let label = if paths.len() > 1 { format!("{} +{}", first, paths.len() - 1) } else { first };
                let t = Rect::new(content.x + 22.0, content.y, (content.w - 22.0).max(0.0), content.h);
                draw_text_cached(dc, ctx, item.id, 0, item.layout_gen, &label, Font::Body, t, theme.fg, false);
            }
        }
    }

    if item.pinned && !hovered {
        let p = Rect::new(r.right() - radius.min(10.0) - 16.0, r.y, 16.0, r.h);
        draw_text_in(dc, ctx.text, ctx.gfx, "\u{E718}", Font::Icon, p, theme.accent, false, false);
    }
    if hovered {
        let c = slot.close;
        let (cx, cy) = c.center();
        unsafe {
            dc.FillEllipse(&D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: c.w / 2.0, radiusY: c.h / 2.0 }, ctx.gfx.brush(if ctx.hover_close { theme.danger } else { theme.badge_bg }));
        }
        let f = ctx.text.format(Font::Badge).clone();
        unsafe {
            let _ = f.SetTextAlignment(windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT_CENTER);
        }
        draw_text_in(dc, ctx.text, ctx.gfx, "✕", Font::Badge, c, if ctx.hover_close { Color::rgba(1.0, 1.0, 1.0, 1.0) } else { theme.fg_secondary }, false, false);
    }
}

fn draw_card(dc: &ID2D1DeviceContext, ctx: &mut DrawCtx, slot: &Slot, item: &ClipItem, vertical: bool) {
    if ctx.pill {
        draw_pill(dc, ctx, slot, item);
        return;
    }
    let theme = *ctx.theme;
    let r = slot.rect;
    let hovered = ctx.hover == Some(item.id);
    let active = ctx.active == Some(item.id);
    let masked = item.is_masked(ctx.now);

    // Card background
    let mut bg = if hovered { theme.item_hover } else { theme.item_bg };
    if active && matches!(ctx.highlight, Highlight::Fill | Highlight::Both) {
        bg = bg.mix(theme.accent.with_alpha(0.28), 0.6);
    }
    if let Some((fid, t)) = ctx.flash {
        if fid == item.id {
            bg = bg.mix(theme.accent.with_alpha(0.6), t);
        }
    }
    fill_rr(dc, ctx.gfx, r, CARD_RADIUS, bg);
    if active && matches!(ctx.highlight, Highlight::Border | Highlight::Both) {
        stroke_rr(dc, ctx.gfx, r, CARD_RADIUS, theme.accent, 2.0);
    } else {
        stroke_rr(dc, ctx.gfx, r, CARD_RADIUS, theme.border, 1.0);
    }

    // Badge
    let mut content_x = r.x + 6.0;
    if ctx.show_badges {
        let n = slot.index + 1;
        let label = if n <= 10 { (n % 10).to_string() } else { String::new() };
        if !label.is_empty() {
            let b = slot.badge;
            let held = ctx.chord_held;
            let (cx, cy) = b.center();
            let scale = if held { 1.12 } else { 1.0 };
            unsafe {
                dc.FillEllipse(&D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: b.w / 2.0 * scale, radiusY: b.h / 2.0 * scale }, ctx.gfx.brush(if held { theme.accent } else { theme.badge_bg }));
            }
            let f = ctx.text.format(Font::Badge).clone();
            unsafe {
                let _ = f.SetTextAlignment(windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT_CENTER);
            }
            draw_text_in(dc, ctx.text, ctx.gfx, &label, Font::Badge, b, if held { theme.on_accent } else { theme.badge_fg }, false, false);
        }
        content_x = slot.badge.right() + 6.0;
    }
    let close_reserved = if hovered { CLOSE + 4.0 } else { 0.0 };
    let content = Rect::new(content_x, r.y + 4.0, (r.right() - 6.0 - content_x - close_reserved).max(0.0), r.h - 8.0);
    let _ = vertical;

    let meta_h = if ctx.show_meta { ctx.text.font_size * 1.35 } else { 0.0 };
    let main_h = (content.h - meta_h).max(ctx.text.font_size * 1.3);
    let main = Rect::new(content.x, content.y, content.w, main_h);
    let meta = Rect::new(content.x, content.y + main_h, content.w, meta_h);

    let source = if ctx.show_source { item.source_exe.as_deref().map(|s| s.trim_end_matches(".exe").to_string()) } else { None };
    let mut meta_text = String::new();

    if masked {
        let icon = Rect::new(main.x, main.y, 18.0, main.h);
        draw_text_in(dc, ctx.text, ctx.gfx, "\u{E72E}", Font::Icon, icon, theme.fg_secondary, false, false);
        let t = Rect::new(main.x + 22.0, main.y, (main.w - 22.0).max(0.0), main.h);
        draw_text_in(dc, ctx.text, ctx.gfx, "••••••••", Font::Body, t, theme.fg, false, true);
        meta_text = i18n_t("Sensitive").to_string();
        if let Some(s) = &source {
            meta_text.push_str(" · ");
            meta_text.push_str(s);
        }
    } else {
        match &item.content {
            ClipContent::Text { chars, lines, looks_like_code, .. } => {
                let snippet = item.display_snippet(ctx.now).unwrap_or("").to_string();
                let font = if *looks_like_code { Font::Mono } else { Font::Body };
                let wrap = !ctx.show_meta;
                draw_text_cached(dc, ctx, item.id, 0, item.layout_gen, &snippet, font, main, theme.fg, wrap);
                meta_text = if *lines > 1 { format!("{} {} · {} {}", lines, i18n_t("lines"), chars, i18n_t("chars")) } else { format!("{} {}", chars, i18n_t("chars")) };
                if let Some(s) = &source {
                    meta_text.push_str(" · ");
                    meta_text.push_str(s);
                }
            }
            ClipContent::Image { width, height, .. } => {
                // Thumbnail on the left, dimensions to the right.
                let th = content.h;
                let max_tw = (content.w * 0.55).max(24.0);
                let mut tw = max_tw.min(th * (*width as f32 / (*height).max(1) as f32));
                if tw < 24.0 {
                    tw = 24.0_f32.min(max_tw);
                }
                let thumb_rect = Rect::new(content.x, content.y, tw, th);
                draw_thumb(dc, ctx, item.id, thumb_rect, 4.0);
                let tx = thumb_rect.right() + 8.0;
                let tr = Rect::new(tx, main.y, (content.right() - tx).max(0.0), main.h);
                draw_text_in(dc, ctx.text, ctx.gfx, &format!("{} × {}", width, height), Font::Body, tr, theme.fg, false, true);
                let mut img_meta = i18n_t("Image").to_string();
                if let Some(s) = &source {
                    img_meta.push_str(" · ");
                    img_meta.push_str(s);
                }
                let mr = Rect::new(tx, meta.y, (content.right() - tx).max(0.0), meta.h);
                if ctx.show_meta {
                    draw_text_in(dc, ctx.text, ctx.gfx, &img_meta, Font::Small, mr, theme.fg_secondary, false, true);
                }
            }
            ClipContent::Files { paths } => {
                let icon = Rect::new(main.x, main.y, 18.0, main.h);
                let glyph = if paths.len() > 1 { "\u{E8B7}" } else { "\u{E8A5}" };
                draw_text_in(dc, ctx.text, ctx.gfx, glyph, Font::Icon, icon, theme.fg_secondary, false, false);
                let first = paths.first().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let label = if paths.len() > 1 { format!("{} +{}", first, paths.len() - 1) } else { first };
                let t = Rect::new(main.x + 22.0, main.y, (main.w - 22.0).max(0.0), main.h);
                draw_text_cached(dc, ctx, item.id, 0, item.layout_gen, &label, Font::Body, t, theme.fg, false);
                meta_text = if paths.len() == 1 { i18n_t("File").into() } else { format!("{} {}", paths.len(), i18n_t("files")) };
                if let Some(s) = &source {
                    meta_text.push_str(" · ");
                    meta_text.push_str(s);
                }
            }
        }
    }
    if ctx.show_meta && !meta_text.is_empty() {
        draw_text_in(dc, ctx.text, ctx.gfx, &meta_text, Font::Small, meta, theme.fg_secondary, false, true);
    }

    if item.pinned {
        let p = Rect::new(r.right() - 18.0, r.bottom() - 18.0, 16.0, 16.0);
        draw_text_in(dc, ctx.text, ctx.gfx, "\u{E718}", Font::Icon, p, theme.accent, false, false);
    }

    if hovered {
        let c = slot.close;
        let (cx, cy) = c.center();
        unsafe {
            dc.FillEllipse(&D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: c.w / 2.0, radiusY: c.h / 2.0 }, ctx.gfx.brush(if ctx.hover_close { theme.danger } else { theme.badge_bg }));
        }
        let f = ctx.text.format(Font::Badge).clone();
        unsafe {
            let _ = f.SetTextAlignment(windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT_CENTER);
        }
        draw_text_in(dc, ctx.text, ctx.gfx, "✕", Font::Badge, c, if ctx.hover_close { Color::rgba(1.0, 1.0, 1.0, 1.0) } else { theme.fg_secondary }, false, false);
    }
}

fn draw_thumb(dc: &ID2D1DeviceContext, ctx: &mut DrawCtx, id: ItemId, r: Rect, radius: f32) {
    let theme = *ctx.theme;
    if let Some(e) = ctx.thumbs.get(id) {
        // letterbox inside r
        let (bw, bh) = (e.width as f32, e.height as f32);
        let s = (r.w / bw).min(r.h / bh);
        let (dw, dh) = (bw * s, bh * s);
        let dst = Rect::new(r.x + (r.w - dw) / 2.0, r.y + (r.h - dh) / 2.0, dw, dh);
        let bitmap = e.bitmap.clone();
        unsafe {
            let geom = ctx.gfx.d2d_factory.CreateRoundedRectangleGeometry(&rr(dst, radius)).ok();
            if let Some(g) = geom {
                let params = D2D1_LAYER_PARAMETERS1 {
                    contentBounds: dst.d2d(),
                    geometricMask: std::mem::ManuallyDrop::new(Some(g.into())),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: Matrix3x2::identity(),
                    opacity: 1.0,
                    opacityBrush: std::mem::ManuallyDrop::new(None),
                    layerOptions: D2D1_LAYER_OPTIONS1_NONE,
                };
                dc.PushLayer(&params, None);
                dc.DrawBitmap(&bitmap, Some(&dst.d2d()), 1.0, D2D1_INTERPOLATION_MODE_LINEAR, None, None);
                dc.PopLayer();
                let _ = std::mem::ManuallyDrop::into_inner(params.geometricMask);
            } else {
                dc.DrawBitmap(&bitmap, Some(&dst.d2d()), 1.0, D2D1_INTERPOLATION_MODE_LINEAR, None, None);
            }
        }
    } else {
        fill_rr(dc, ctx.gfx, r, radius, theme.badge_bg);
        draw_text_in(dc, ctx.text, ctx.gfx, "\u{EB9F}", Font::Icon, r, theme.fg_secondary, false, false);
        if !ctx.thumbs.pending.contains(&id) && !ctx.thumbs.failed.contains(&id) {
            ctx.thumbs.pending.insert(id);
            ctx.thumb_requests.push(id);
        }
    }
}

pub fn draw_empty(dc: &ID2D1DeviceContext, ctx: &mut DrawCtx, area: Rect, text: &str) {
    let f = ctx.text.format(Font::Body).clone();
    unsafe {
        let _ = f.SetTextAlignment(windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT_CENTER);
    }
    draw_text_in(dc, ctx.text, ctx.gfx, text, Font::Body, area, ctx.theme.fg_secondary, false, false);
    unsafe {
        let _ = f.SetTextAlignment(windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT_LEADING);
    }
}

#[allow(dead_code)]
fn _color(c: D2D1_COLOR_F) -> D2D1_COLOR_F {
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> LayoutParams {
        LayoutParams { item_len: 100.0, gap: 10.0, padding: 5.0, overflow_len: 30.0, hint_len: 0.0, max_visible: 10, align: 0, pill: false, shelf_len: 0.0, hint_pinned: false }
    }

    #[test]
    fn fits_without_overflow() {
        let ids: Vec<ItemId> = (1..=3).collect();
        let l = layout(&ids, Rect::new(0.0, 0.0, 340.0, 50.0), false, &p(), 0);
        assert_eq!(l.slots.len(), 3);
        assert!(l.overflow.is_none());
        assert_eq!(l.slots[0].rect.x, 5.0);
        assert_eq!(l.slots[1].rect.x, 115.0);
    }

    #[test]
    fn overflow_when_too_many() {
        let ids: Vec<ItemId> = (1..=10).collect();
        let l = layout(&ids, Rect::new(0.0, 0.0, 340.0, 50.0), false, &p(), 0);
        // 330 inner; with overflow 30 → 300 → floor(310/110)=2
        assert_eq!(l.slots.len(), 2);
        assert!(l.overflow.is_some());
        assert_eq!(l.hidden, 8);
        assert_eq!(l.hit(12.0, 12.0), Hit::Badge(1));
        assert_eq!(l.hit(60.0, 30.0), Hit::Item(1));
        assert_eq!(l.hit(l.overflow.unwrap().x + 1.0, 10.0), Hit::Overflow);
    }

    #[test]
    fn scroll_and_vertical() {
        let ids: Vec<ItemId> = (1..=5).collect();
        let l = layout(&ids, Rect::new(0.0, 0.0, 200.0, 340.0), true, &p(), 3);
        assert_eq!(l.slots.len(), 2);
        assert_eq!(l.slots[0].id, 4);
        assert!(l.overflow.is_some());
        assert_eq!(l.slots[0].rect.y, 5.0);
        assert_eq!(l.slots[1].rect.y, 115.0);
    }

    #[test]
    fn alignment_shifts_content() {
        let ids: Vec<ItemId> = (1..=2).collect();
        let mut pp = p();
        pp.align = 1;
        let l = layout(&ids, Rect::new(0.0, 0.0, 400.0, 50.0), false, &pp, 0);
        // inner 390, content 210 → slack 180 → offset 90
        assert_eq!(l.slots[0].rect.x, 95.0);
        pp.align = 2;
        let l = layout(&ids, Rect::new(0.0, 0.0, 400.0, 50.0), false, &pp, 0);
        assert_eq!(l.slots[1].rect.right(), 395.0);
        pp.pill = true;
        let l = layout(&ids, Rect::new(0.0, 0.0, 400.0, 50.0), false, &pp, 0);
        let s0 = l.slots[0];
        assert!((s0.badge.center().1 - s0.rect.center().1).abs() < 0.01);
    }

    #[test]
    fn pinned_hint_and_shelf_button() {
        let ids: Vec<ItemId> = (1..=2).collect();
        let mut pp = p();
        pp.align = 1;
        pp.hint_len = 40.0;
        pp.hint_pinned = true;
        pp.shelf_len = 30.0;
        let l = layout(&ids, Rect::new(0.0, 0.0, 500.0, 50.0), false, &pp, 0);
        assert_eq!(l.hint.unwrap().x, 5.0);
        let sr = l.shelf.unwrap();
        assert_eq!(sr.right(), 495.0);
        assert_eq!(l.hit(480.0, 20.0), Hit::Shelf);
        // items centered in the region between hint and shelf button
        let region_start = 5.0 + 50.0;
        let region_len = 490.0 - 40.0 - 50.0;
        let expected = region_start + ((region_len - 210.0) / 2.0_f32).floor();
        assert_eq!(l.slots[0].rect.x, expected);
    }

    #[test]
    fn hint_reserves_space() {
        let mut pp = p();
        pp.hint_len = 40.0;
        let ids: Vec<ItemId> = (1..=2).collect();
        let l = layout(&ids, Rect::new(0.0, 0.0, 300.0, 50.0), false, &pp, 0);
        assert_eq!(l.hint.unwrap().w, 40.0);
        assert_eq!(l.slots[0].rect.x, 55.0);
        assert_eq!(l.hit(10.0, 10.0), Hit::Hint);
    }
}
