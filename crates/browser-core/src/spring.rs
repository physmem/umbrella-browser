use serde::{Deserialize, Serialize};

/// Fixed integration step. Small enough to stay stable for stiff springs at
/// any frame rate; frames just run more substeps.
const SUBSTEP: f32 = 1.0 / 240.0;
/// A stalled frame (debugger, sleep) should not fling the value.
const MAX_FRAME: f32 = 0.1;
const REST_DISTANCE: f32 = 0.001;
const REST_VELOCITY: f32 = 0.01;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SpringParams {
    pub stiffness: f32,
    pub damping: f32,
    #[serde(default = "unit_mass")]
    pub mass: f32,
}

impl SpringParams {
    pub const fn new(stiffness: f32, damping: f32) -> Self {
        Self {
            stiffness,
            damping,
            mass: 1.0,
        }
    }

    /// Below 1.0 overshoots, 1.0 is critically damped, above 1.0 is sluggish.
    pub fn damping_ratio(&self) -> f32 {
        self.damping / (2.0 * (self.stiffness * self.mass).sqrt())
    }

    pub fn is_valid(&self) -> bool {
        self.stiffness > 0.0 && self.damping >= 0.0 && self.mass > 0.0
    }
}

/// User-selectable motion feel. Calm settles with little or no overshoot;
/// Lively keeps visible bounce. Reduced motion still snaps either way.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MotionStyle {
    #[default]
    Calm,
    Lively,
}

/// Calm never overshoots; Lively keeps one small, quick settle.
const CALM_MIN_RATIO: f32 = 1.0;
const LIVELY_RATIO: f32 = 0.78;
const LIVELY_STIFFNESS: f32 = 1.1;

impl MotionStyle {
    pub fn apply(self, base: SpringParams) -> SpringParams {
        let critical = 2.0 * (base.stiffness * base.mass).sqrt();
        match self {
            MotionStyle::Calm => SpringParams {
                damping: base.damping.max(critical * CALM_MIN_RATIO),
                ..base
            },
            MotionStyle::Lively => {
                let stiffness = base.stiffness * LIVELY_STIFFNESS;
                let damping = 2.0 * (stiffness * base.mass).sqrt() * LIVELY_RATIO;
                SpringParams {
                    stiffness,
                    damping,
                    ..base
                }
            }
        }
    }
}

fn unit_mass() -> f32 {
    1.0
}

/// A damped harmonic spring driving one scalar (offset, width, opacity).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    pub params: SpringParams,
    value: f32,
    velocity: f32,
    target: f32,
}

impl Spring {
    pub fn new(params: SpringParams, value: f32) -> Self {
        Self {
            params,
            value,
            velocity: 0.0,
            target: value,
        }
    }

    pub fn value(&self) -> f32 {
        self.value
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    /// Retargets mid-flight; velocity is kept so interrupted motion stays smooth.
    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    pub fn snap_to(&mut self, target: f32) {
        self.target = target;
        self.value = target;
        self.velocity = 0.0;
    }

    pub fn is_settled(&self) -> bool {
        (self.value - self.target).abs() < REST_DISTANCE && self.velocity.abs() < REST_VELOCITY
    }

    /// Advances by `dt` seconds. With reduced motion the value jumps to the
    /// target, matching the Windows "Animation effects" setting.
    pub fn step(&mut self, dt: f32, reduced_motion: bool) -> f32 {
        if reduced_motion {
            self.snap_to(self.target);
            return self.value;
        }
        let mut remaining = dt.clamp(0.0, MAX_FRAME);
        while remaining > 0.0 && !self.is_settled() {
            let h = remaining.min(SUBSTEP);
            self.integrate(h);
            remaining -= h;
        }
        if self.is_settled() {
            self.snap_to(self.target);
        }
        self.value
    }

    // Semi-implicit Euler: velocity first, then position. Stable where explicit
    // Euler would gain energy and explode.
    fn integrate(&mut self, h: f32) {
        let p = self.params;
        let force = -p.stiffness * (self.value - self.target) - p.damping * self.velocity;
        self.velocity += force / p.mass * h;
        self.value += self.velocity * h;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(spring: &mut Spring, seconds: f32) -> f32 {
        let mut peak = spring.value();
        for _ in 0..(seconds * 60.0) as usize {
            peak = peak.max(spring.step(1.0 / 60.0, false));
        }
        peak
    }

    #[test]
    fn settles_on_target() {
        let mut s = Spring::new(SpringParams::new(380.0, 30.0), 0.0);
        s.set_target(100.0);
        run(&mut s, 2.0);
        assert!(s.is_settled());
        assert_eq!(s.value(), 100.0);
    }

    #[test]
    fn underdamped_overshoots_critical_does_not() {
        let mut bouncy = Spring::new(SpringParams::new(380.0, 20.0), 0.0);
        bouncy.set_target(1.0);
        assert!(run(&mut bouncy, 1.0) > 1.0);

        let crit = SpringParams::new(400.0, 40.0);
        assert!((crit.damping_ratio() - 1.0).abs() < 1e-6);
        let mut firm = Spring::new(crit, 0.0);
        firm.set_target(1.0);
        assert!(run(&mut firm, 1.0) <= 1.0 + REST_DISTANCE);
    }

    #[test]
    fn motion_styles_bound_damping_ratio() {
        let base = SpringParams::new(380.0, 20.0);
        assert!(MotionStyle::Calm.apply(base).damping_ratio() >= 1.0 - 1e-4);
        assert!((MotionStyle::Lively.apply(base).damping_ratio() - 0.78).abs() < 1e-4);
        let firm = SpringParams::new(400.0, 45.0);
        assert_eq!(MotionStyle::Calm.apply(firm), firm);
    }

    #[test]
    fn reduced_motion_snaps() {
        let mut s = Spring::new(SpringParams::new(380.0, 30.0), 0.0);
        s.set_target(5.0);
        assert_eq!(s.step(0.001, true), 5.0);
    }

    #[test]
    fn huge_frame_is_clamped() {
        let mut s = Spring::new(SpringParams::new(2000.0, 10.0), 0.0);
        s.set_target(1.0);
        assert!(s.step(1000.0, false).is_finite());
    }
}
