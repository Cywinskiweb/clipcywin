//! DirectWrite text formats and a per-item layout cache.

use crate::model::item::ItemId;
use crate::settings::Appearance;
use crate::util::wide::WStr;
use std::collections::HashMap;
use windows::core::Result;
use windows::Win32::Graphics::DirectWrite::{
    IDWriteFactory, IDWriteInlineObject, IDWriteTextFormat, IDWriteTextLayout, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWRITE_WORD_WRAPPING_WRAP,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Font {
    Body,
    BodyBold,
    Small,
    Mono,
    Icon,
    Badge,
    Preview,
    PreviewMono,
}

pub struct TextEngine {
    dwrite: IDWriteFactory,
    body: IDWriteTextFormat,
    body_bold: IDWriteTextFormat,
    small: IDWriteTextFormat,
    mono: IDWriteTextFormat,
    icon: IDWriteTextFormat,
    badge: IDWriteTextFormat,
    preview: IDWriteTextFormat,
    preview_mono: IDWriteTextFormat,
    ellipsis: IDWriteInlineObject,
    cache: HashMap<(ItemId, u8), CachedLayout>,
    pub generation: u32,
    pub font_size: f32,
}

struct CachedLayout {
    generation: u32,
    width: f32,
    height: f32,
    layout: IDWriteTextLayout,
}

fn make_format(dw: &IDWriteFactory, family: &str, size: f32, weight: windows::Win32::Graphics::DirectWrite::DWRITE_FONT_WEIGHT) -> Result<IDWriteTextFormat> {
    unsafe {
        let f = dw.CreateTextFormat(WStr::new(family).pcwstr(), None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, WStr::new("").pcwstr())?;
        f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        Ok(f)
    }
}

impl TextEngine {
    /// `preview_size` = 0 keeps the bar font size for previews.
    pub fn with_preview(dwrite: &IDWriteFactory, a: &Appearance, preview_size: f32) -> Result<TextEngine> {
        let ps = if preview_size > 0.0 { preview_size } else { a.font_size };
        let preview = make_format(dwrite, &a.font_family, ps, DWRITE_FONT_WEIGHT_NORMAL)?;
        let preview_mono = make_format(dwrite, &a.mono_font, (ps * 0.92).max(8.0), DWRITE_FONT_WEIGHT_NORMAL)?;
        let body = make_format(dwrite, &a.font_family, a.font_size, DWRITE_FONT_WEIGHT_NORMAL)?;
        let body_bold = make_format(dwrite, &a.font_family, a.font_size, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
        let small = make_format(dwrite, &a.font_family, (a.font_size * 0.85).max(8.0), DWRITE_FONT_WEIGHT_NORMAL)?;
        let mono = make_format(dwrite, &a.mono_font, (a.font_size * 0.92).max(8.0), DWRITE_FONT_WEIGHT_NORMAL)?;
        let icon = make_format(dwrite, "Segoe Fluent Icons", a.font_size, DWRITE_FONT_WEIGHT_NORMAL)?;
        let badge = make_format(dwrite, &a.font_family, (a.font_size * 0.8).max(8.0), DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
        unsafe {
            icon.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            badge.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            body.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING)?;
        }
        let ellipsis = unsafe { dwrite.CreateEllipsisTrimmingSign(&body)? };
        Ok(TextEngine {
            dwrite: dwrite.clone(),
            body,
            body_bold,
            small,
            mono,
            icon,
            badge,
            preview,
            preview_mono,
            ellipsis,
            cache: HashMap::new(),
            generation: 1,
            font_size: a.font_size,
        })
    }

    /// Recreate formats after font settings changed; invalidates the cache.
    pub fn rebuild(&mut self, a: &Appearance, preview_size: f32) -> Result<()> {
        let fresh = TextEngine::with_preview(&self.dwrite, a, preview_size)?;
        let gen = self.generation.wrapping_add(1);
        *self = fresh;
        self.generation = gen;
        Ok(())
    }

    pub fn format(&self, f: Font) -> &IDWriteTextFormat {
        match f {
            Font::Body => &self.body,
            Font::BodyBold => &self.body_bold,
            Font::Small => &self.small,
            Font::Mono => &self.mono,
            Font::Icon => &self.icon,
            Font::Badge => &self.badge,
            Font::Preview => &self.preview,
            Font::PreviewMono => &self.preview_mono,
        }
    }

    /// Single-use layout (no caching). `wrap` enables multi-line wrapping.
    pub fn layout_once(&self, text: &str, font: Font, w: f32, h: f32, wrap: bool, trim: bool) -> Result<IDWriteTextLayout> {
        let w16: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            let l = self.dwrite.CreateTextLayout(&w16, self.format(font), w.max(1.0), h.max(1.0))?;
            if wrap {
                l.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
                l.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
            }
            if trim {
                let t = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
                l.SetTrimming(&t, &self.ellipsis)?;
            }
            Ok(l)
        }
    }

    /// Cached per (item, slot) layout, re-created when width/height/generation change.
    pub fn layout_cached(&mut self, id: ItemId, slot: u8, item_gen: u32, text: &str, font: Font, w: f32, h: f32, wrap: bool) -> Option<IDWriteTextLayout> {
        let key = (id, slot);
        let gen = self.generation ^ item_gen.wrapping_mul(0x9E3779B1);
        if let Some(c) = self.cache.get(&key) {
            if c.generation == gen && (c.width - w).abs() < 0.5 && (c.height - h).abs() < 0.5 {
                return Some(c.layout.clone());
            }
        }
        let layout = self.layout_once(text, font, w, h, wrap, true).ok()?;
        self.cache.insert(key, CachedLayout { generation: gen, width: w, height: h, layout: layout.clone() });
        if self.cache.len() > 512 {
            // Simple pressure valve: drop everything and rebuild lazily.
            self.cache.clear();
        }
        Some(layout)
    }

    pub fn invalidate_item(&mut self, id: ItemId) {
        self.cache.retain(|k, _| k.0 != id);
    }

    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Measure text width in DIPs (unbounded).
    pub fn measure(&self, text: &str, font: Font) -> (f32, f32) {
        match self.layout_once(text, font, 4096.0, 4096.0, false, false) {
            Ok(l) => unsafe {
                let mut m = std::mem::zeroed();
                if l.GetMetrics(&mut m).is_ok() {
                    (m.widthIncludingTrailingWhitespace, m.height)
                } else {
                    (0.0, 0.0)
                }
            },
            Err(_) => (0.0, 0.0),
        }
    }
}
