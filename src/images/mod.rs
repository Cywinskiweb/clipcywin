pub mod cache;
pub mod worker;

/// Fit `(w, h)` into `(max_w, max_h)` preserving aspect ratio, never upscaling.
pub fn fit_within(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if w == 0 || h == 0 || max_w == 0 || max_h == 0 {
        return (1, 1);
    }
    if w <= max_w && h <= max_h {
        return (w, h);
    }
    let sx = max_w as f64 / w as f64;
    let sy = max_h as f64 / h as f64;
    let s = sx.min(sy);
    (((w as f64 * s).round() as u32).max(1), ((h as f64 * s).round() as u32).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fit() {
        assert_eq!(fit_within(100, 50, 50, 50), (50, 25));
        assert_eq!(fit_within(10, 10, 50, 50), (10, 10));
        assert_eq!(fit_within(1000, 4000, 200, 100), (25, 100));
    }
}
