//! Overflow list popup: sizing helpers (drawing reuses items_view in vertical mode).

use crate::settings::Settings;

pub const MAX_ROWS: usize = 8;

#[derive(Default)]
pub struct ListState {
    pub scroll: usize,
}

/// Popup size in DIPs for `n` items.
pub fn measure(n: usize, s: &Settings) -> (f32, f32) {
    let pad = s.layout.padding_dip as f32;
    let gap = s.layout.gap_dip as f32;
    let rows = n.clamp(1, MAX_ROWS) as f32;
    let item_h = s.bar.height_dip as f32 - 2.0 * pad;
    let w = s.layout.item_width_dip as f32 * 1.4 + 2.0 * pad;
    let overflow = if n > MAX_ROWS { 28.0 + gap } else { 0.0 };
    let h = rows * item_h + (rows - 1.0) * gap + 2.0 * pad + overflow;
    (w, h)
}

impl ListState {
    pub fn scroll_by(&mut self, delta_rows: i32, n: usize) {
        let max = n.saturating_sub(MAX_ROWS);
        let cur = self.scroll as i32 + delta_rows;
        self.scroll = cur.clamp(0, max as i32) as usize;
    }
}
