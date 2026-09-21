//! Hover preview: size computation and drawing.

use crate::gfx::text::Font;
use crate::gfx::Rect;
use crate::i18n::t as i18n_t;
use crate::images::cache::BitmapCache;
use crate::model::item::{ClipContent, ClipItem, ItemId};
use crate::ui::items_view::{draw_text_in, fill_rr, stroke_rr, DrawCtx};
use std::time::Instant;
use windows::Win32::Graphics::Direct2D::{ID2D1DeviceContext, D2D1_INTERPOLATION_MODE_LINEAR};

pub const PAD: f32 = 12.0;
/// Hard cap so a multi-megabyte paste cannot stall layout; everything below it is shown in full.
pub const MAX_CHARS: usize = 200_000;

/// What the preview shows for an item.
pub enum PreviewKind {
    Text { text: String, mono: bool },
    Image { id: ItemId, width: u32, height: u32 },
    Files { lines: Vec<String> },
    Masked,
}

pub fn kind_for(item: &ClipItem, now: Instant) -> PreviewKind {
    if item.is_masked(now) {
        return PreviewKind::Masked;
    }
    match &item.content {
        ClipContent::Text { text, looks_like_code, .. } => {
            let mut t: String = text.chars().take(MAX_CHARS).collect();
            if text.chars().count() > MAX_CHARS {
                t.push_str(&format!("\n… ({} {})", text.chars().count() - MAX_CHARS, crate::i18n::t("more characters")));
            }
            PreviewKind::Text { text: t, mono: *looks_like_code }
        }
        ClipContent::Image { width, height, .. } => PreviewKind::Image { id: item.id, width: *width, height: *height },
        ClipContent::Files { paths } => PreviewKind::Files { lines: paths.iter().take(30).map(|p| p.to_string_lossy().into_owned()).collect() },
    }
}

pub fn footer(item: &ClipItem) -> String {
    let mut s = String::new();
    if let Some(src) = &item.source_exe {
        s.push_str(src.trim_end_matches(".exe"));
    }
    let age_ms = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0) - item.created_at_ms).max(0);
    let age = if age_ms < 60_000 {
        i18n_t("just now").to_string()
    } else if age_ms < 3_600_000 {
        format!("{} {}", age_ms / 60_000, i18n_t("min ago"))
    } else if age_ms < 86_400_000 {
        format!("{} {}", age_ms / 3_600_000, i18n_t("h ago"))
    } else {
        format!("{} {}", age_ms / 86_400_000, i18n_t("d ago"))
    };
    if !s.is_empty() {
        s.push_str(" · ");
    }
    s.push_str(&age);
    match &item.content {
        ClipContent::Text { chars, lines, .. } => s.push_str(&format!(" · {} {} · {} {}", chars, i18n_t("chars"), lines, i18n_t("lines"))),
        ClipContent::Image { width, height, bytes, has_alpha, .. } => {
            s.push_str(&format!(" · {} × {} · {} KB", width, height, bytes / 1024));
            if *has_alpha {
                s.push_str(" · ");
                s.push_str(i18n_t("alpha"));
            }
        }
        ClipContent::Files { paths } => s.push_str(&format!(" · {} {}", paths.len(), i18n_t("file(s)"))),
    }
    if item.sensitivity.is_sensitive() {
        s.push_str(" · ");
        s.push_str(i18n_t("sensitive"));
    }
    s
}

fn measure_wrapped(text: &crate::gfx::text::TextEngine, s: &str, font: Font, w: f32, max_h: f32) -> (f32, f32) {
    match text.layout_once(s, font, w, max_h, true, false) {
        Ok(l) => unsafe {
            let mut m = std::mem::zeroed();
            if l.GetMetrics(&mut m).is_ok() {
                (m.widthIncludingTrailingWhitespace, m.height)
            } else {
                (w, 20.0)
            }
        },
        Err(_) => (w, 20.0),
    }
}

/// Size (DIPs) of the preview popup: (width, height, footer height). Grows up to `max_w` × `max_h`.
pub fn measure(kind: &PreviewKind, footer: &str, text: &crate::gfx::text::TextEngine, thumbs: &mut BitmapCache, max_w: f32, max_h: f32) -> (f32, f32, f32) {
    let inner_w = (max_w - 2.0 * PAD).max(120.0);
    let (_, footer_h) = measure_wrapped(text, footer, Font::Small, inner_w, 200.0);
    let footer_h = footer_h + 4.0;
    let inner_h = (max_h - 2.0 * PAD - footer_h).max(40.0);
    let (w, h) = match kind {
        PreviewKind::Text { text: t, mono } => {
            let font = if *mono { Font::PreviewMono } else { Font::Preview };
            match text.layout_once(t, font, inner_w, inner_h, true, false) {
                Ok(l) => unsafe {
                    let mut m = std::mem::zeroed();
                    if l.GetMetrics(&mut m).is_ok() {
                        ((m.widthIncludingTrailingWhitespace + 2.0).clamp(120.0, inner_w), m.height.clamp(20.0, inner_h))
                    } else {
                        (inner_w, 100.0)
                    }
                },
                Err(_) => (inner_w, 100.0),
            }
        }
        PreviewKind::Image { id, width, height } => {
            let (bw, bh) = match thumbs.get(*id) {
                Some(e) => (e.width as f32, e.height as f32),
                None => (*width as f32, *height as f32),
            };
            let s = (inner_w / bw.max(1.0)).min(inner_h / bh.max(1.0)).min(1.0);
            ((bw * s).max(80.0), (bh * s).max(60.0))
        }
        PreviewKind::Files { lines } => {
            let mut w: f32 = 160.0;
            for l in lines {
                w = w.max(text.measure(l, Font::Body).0 + 4.0);
            }
            (w.min(inner_w), (lines.len() as f32 * text.font_size * 1.5).clamp(20.0, inner_h))
        }
        PreviewKind::Masked => (220.0, 40.0),
    };
    let w = w.max(160.0);
    (w + 2.0 * PAD, h + 2.0 * PAD + footer_h, footer_h)
}

/// Visual style of the preview popup (from `settings.preview`).
#[derive(Clone, Copy)]
pub struct PreviewStyle {
    pub fill: crate::gfx::theme::Color,
    pub radius: f32,
    pub border: bool,
}

pub fn draw(dc: &ID2D1DeviceContext, ctx: &mut DrawCtx, kind: &PreviewKind, footer_text: &str, area: Rect, footer_h: f32, previews: &mut BitmapCache, style: PreviewStyle) {
    let theme = *ctx.theme;
    fill_rr(dc, ctx.gfx, area, style.radius, style.fill);
    if style.border {
        stroke_rr(dc, ctx.gfx, area, style.radius, theme.border, 1.0);
    }
    let inner = Rect::new(area.x + PAD, area.y + PAD, area.w - 2.0 * PAD, area.h - 2.0 * PAD - footer_h);
    match kind {
        PreviewKind::Text { text, mono } => {
            let font = if *mono { Font::PreviewMono } else { Font::Preview };
            draw_text_in(dc, ctx.text, ctx.gfx, text, font, inner, theme.fg, true, false);
        }
        PreviewKind::Image { id, .. } => {
            let entry = previews.get(*id).map(|e| (e.bitmap.clone(), e.width, e.height)).or_else(|| ctx.thumbs.get(*id).map(|e| (e.bitmap.clone(), e.width, e.height)));
            match entry {
                Some((bmp, bw, bh)) => {
                    let (bw, bh) = (bw as f32, bh as f32);
                    let s = (inner.w / bw).min(inner.h / bh);
                    let (dw, dh) = (bw * s, bh * s);
                    let dst = Rect::new(inner.x + (inner.w - dw) / 2.0, inner.y + (inner.h - dh) / 2.0, dw, dh);
                    unsafe { dc.DrawBitmap(&bmp, Some(&dst.d2d()), 1.0, D2D1_INTERPOLATION_MODE_LINEAR, None, None) };
                }
                None => {
                    draw_text_in(dc, ctx.text, ctx.gfx, i18n_t("Loading…"), Font::Body, inner, theme.fg_secondary, false, false);
                }
            }
        }
        PreviewKind::Files { lines } => {
            let lh = ctx.text.font_size * 1.5;
            for (i, l) in lines.iter().enumerate() {
                let r = Rect::new(inner.x, inner.y + i as f32 * lh, inner.w, lh);
                if r.bottom() > inner.bottom() + 1.0 {
                    break;
                }
                draw_text_in(dc, ctx.text, ctx.gfx, l, Font::Body, r, theme.fg, false, true);
            }
        }
        PreviewKind::Masked => {
            draw_text_in(dc, ctx.text, ctx.gfx, &format!("\u{E72E}  {}", i18n_t("Sensitive content hidden")), Font::Body, inner, theme.fg_secondary, false, false);
        }
    }
    let f = Rect::new(area.x + PAD, area.bottom() - PAD - footer_h + 4.0, area.w - 2.0 * PAD, footer_h);
    draw_text_in(dc, ctx.text, ctx.gfx, footer_text, Font::Small, f, theme.fg_secondary, true, false);
}
