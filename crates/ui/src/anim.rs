//! Shared animation presets. Every animated element in the app goes through
//! this module so timing feels consistent and "reduce motion" is honoured in
//! one place (`Context::style().animation_time == 0`).

use egui::{Color32, Context, Id};

pub const FAST: f32 = 0.12;
pub const NORMAL: f32 = 0.2;
pub const SLOW: f32 = 0.32;

pub fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

pub fn ease_in_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

/// Overshooting ease ("back"), good for pop-ups.
pub fn ease_out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let (c1, c3) = (1.70158_f32, 2.70158_f32);
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

/// Closed-form damped spring from 0 to 1 over normalised time.
pub fn spring(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (-7.0 * t).exp() * (12.0 * t).cos()
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| lerp(f32::from(x), f32::from(y), t).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

/// Boolean animation with an ease-out curve. Returns 0..=1.
pub fn ease_bool(ctx: &Context, id: Id, target: bool, seconds: f32) -> f32 {
    ease_out_cubic(ctx.animate_bool_with_time(id, target, seconds))
}

/// A critically-damped-ish spring for values that chase a target (drop
/// indicators, drag ghosts). Integrates with fixed sub-steps so it is stable
/// at any frame rate.
#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub pos: f32,
    pub vel: f32,
    pub target: f32,
    pub stiffness: f32,
    pub damping: f32,
}

impl Spring {
    pub fn new(value: f32) -> Self {
        Self {
            pos: value,
            vel: 0.0,
            target: value,
            stiffness: 420.0,
            damping: 34.0,
        }
    }

    pub fn snap(&mut self, value: f32) {
        self.pos = value;
        self.target = value;
        self.vel = 0.0;
    }

    pub fn settled(&self) -> bool {
        (self.pos - self.target).abs() < 0.05 && self.vel.abs() < 0.5
    }

    /// Advance by `dt` seconds. Returns true while still moving.
    pub fn step(&mut self, dt: f32) -> bool {
        let dt = dt.min(0.05);
        let steps = ((dt / 0.004).ceil() as usize).max(1);
        let h = dt / steps as f32;
        for _ in 0..steps {
            let force = self.stiffness * (self.target - self.pos) - self.damping * self.vel;
            self.vel += force * h;
            self.pos += self.vel * h;
        }
        if self.settled() {
            self.pos = self.target;
            self.vel = 0.0;
            false
        } else {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easings_hit_endpoints() {
        for f in [ease_out_cubic, ease_in_out_cubic, ease_out_back] {
            assert!((f(0.0)).abs() < 1e-5);
            assert!((f(1.0) - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn spring_settles() {
        let mut s = Spring::new(0.0);
        s.target = 100.0;
        let mut frames = 0;
        while s.step(1.0 / 60.0) {
            frames += 1;
            assert!(frames < 600, "spring never settled");
        }
        assert_eq!(s.pos, 100.0);
    }
}
