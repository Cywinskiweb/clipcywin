//! Time-based animation helpers.

use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Easing {
    Linear,
    OutCubic,
    InOutCubic,
    OutBack,
}

pub fn ease(e: Easing, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match e {
        Easing::Linear => t,
        Easing::OutCubic => 1.0 - (1.0 - t).powi(3),
        Easing::InOutCubic => {
            if t < 0.5 {
                4.0 * t * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
            }
        }
        Easing::OutBack => {
            let c1 = 1.20132; // overshoot ≈ 4 %
            let c3 = c1 + 1.0;
            1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
        }
    }
}

/// Animated scalar. When `dur` is zero the value jumps immediately.
#[derive(Clone, Copy, Debug)]
pub struct Anim {
    from: f32,
    to: f32,
    start: Instant,
    dur: Duration,
    easing: Easing,
}

impl Anim {
    pub fn fixed(v: f32) -> Anim {
        Anim { from: v, to: v, start: Instant::now(), dur: Duration::ZERO, easing: Easing::Linear }
    }

    pub fn value(&self, now: Instant) -> f32 {
        if self.dur.is_zero() {
            return self.to;
        }
        let t = now.saturating_duration_since(self.start).as_secs_f32() / self.dur.as_secs_f32();
        if t >= 1.0 {
            self.to
        } else {
            self.from + (self.to - self.from) * ease(self.easing, t)
        }
    }

    pub fn active(&self, now: Instant) -> bool {
        !self.dur.is_zero() && now.saturating_duration_since(self.start) < self.dur
    }

    /// Retarget from the current value (no jump when reversing mid-flight).
    pub fn go(&mut self, to: f32, dur: Duration, easing: Easing, now: Instant) {
        if (self.to - to).abs() < f32::EPSILON && !self.active(now) {
            return;
        }
        let cur = self.value(now);
        self.from = cur;
        self.to = to;
        self.start = now;
        self.dur = if (cur - to).abs() < 0.001 { Duration::ZERO } else { dur };
        self.easing = easing;
    }

    pub fn set(&mut self, v: f32) {
        self.from = v;
        self.to = v;
        self.dur = Duration::ZERO;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_endpoints() {
        for e in [Easing::Linear, Easing::OutCubic, Easing::InOutCubic, Easing::OutBack] {
            assert!((ease(e, 0.0)).abs() < 1e-5);
            assert!((ease(e, 1.0) - 1.0).abs() < 1e-5);
        }
        // overshoot stays small
        let mx = (0..100).map(|i| ease(Easing::OutBack, i as f32 / 100.0)).fold(0f32, f32::max);
        assert!(mx < 1.06);
    }

    #[test]
    fn anim_reverses_without_jump() {
        let t0 = Instant::now();
        let mut a = Anim::fixed(0.0);
        a.go(1.0, Duration::from_millis(100), Easing::Linear, t0);
        let mid = t0 + Duration::from_millis(50);
        let v = a.value(mid);
        assert!((v - 0.5).abs() < 0.01);
        a.go(0.0, Duration::from_millis(100), Easing::Linear, mid);
        assert!((a.value(mid) - v).abs() < 1e-5);
        assert!(!a.active(mid + Duration::from_millis(200)));
    }
}
