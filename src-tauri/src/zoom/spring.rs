//! Critically-damped spring physics for smooth cursor motion.
//!
//! Used to soften the drawn cursor: the recorded trail is sampled at 60 Hz and
//! lands on integer-ish coordinates, so drawing it verbatim reads as jitter. The
//! spring chases the real position instead, which rounds the corners without
//! ever lagging behind or overshooting.
//!
//! Unlike the zoom transitions, which are a fixed curve over a known window, the
//! spring is stateful and runs forward in time. The cursor renderer therefore
//! steps it in a fixed order — see `input::cursor`, which integrates the whole
//! trail up front so sampling stays order-independent.

/// Default angular frequency, in rad/s. Higher settles faster.
pub const DEFAULT_OMEGA: f64 = 18.0;

/// A one-dimensional critically-damped spring.
///
/// Critically damped means `damping == 2*sqrt(stiffness)`: it converges in the
/// shortest time that does not overshoot. An underdamped spring (the default in
/// most UI toolkits) rings; an overdamped one crawls. Neither belongs in a
/// screencast.
#[derive(Debug, Clone)]
pub struct Spring {
    pub position: f64,
    pub velocity: f64,
    pub target: f64,
    /// 2 × ω, kept explicit so the struct stays inspectable.
    pub damping: f64,
    /// ω².
    pub stiffness: f64,
}

impl Spring {
    /// A spring already at `target`, at rest.
    pub fn new(target: f64) -> Self {
        Self::with_omega(target, DEFAULT_OMEGA)
    }

    /// A spring already at `target`, with an explicit angular frequency.
    pub fn with_omega(target: f64, omega: f64) -> Self {
        let omega = omega.max(f64::EPSILON);
        Self {
            position: target,
            velocity: 0.0,
            target,
            damping: 2.0 * omega,
            stiffness: omega * omega,
        }
    }

    /// A spring at `position` that is heading for `target`.
    pub fn between(position: f64, target: f64) -> Self {
        Self {
            position,
            target,
            ..Self::new(target)
        }
    }

    /// Advance the simulation by `dt` seconds.
    ///
    /// Semi-implicit Euler: velocity is updated first and the new velocity moves
    /// the position. That ordering is what makes it stable — plain (explicit)
    /// Euler gains energy, and a fast-moving cursor would visibly explode.
    ///
    /// Sub-stepped at 240 Hz so a coarse caller (a 30 fps export frame) still
    /// gets the same answer as a fine one.
    pub fn update(&mut self, dt: f64) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        let mut remaining = dt;
        while remaining > 0.0 {
            let h = remaining.min(1.0 / 240.0);
            let accel = self.stiffness * (self.target - self.position) - self.damping * self.velocity;
            self.velocity += accel * h;
            self.position += self.velocity * h;
            remaining -= h;
        }
    }

    /// Retarget without moving. A jump in target is what the spring smooths out.
    pub fn set_target(&mut self, target: f64) {
        self.target = target;
    }

    /// Teleport: no transition, no residual velocity. Used when the trail has a
    /// genuine gap (the pointer left the recorded display and came back).
    pub fn reset(&mut self, position: f64) {
        self.position = position;
        self.target = position;
        self.velocity = 0.0;
    }

    /// How far the spring still is from its target.
    pub fn error(&self) -> f64 {
        (self.position - self.target).abs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_rest_on_its_target() {
        let mut spring = Spring::new(10.0);
        assert_eq!(spring.velocity, 0.0);
        spring.update(1.0 / 60.0);
        // Already at rest, so a step changes nothing.
        assert!((spring.position - 10.0).abs() < 1e-9);
    }

    #[test]
    fn converges_without_overshooting() {
        let mut spring = Spring::between(0.0, 100.0);
        let mut peak: f64 = 0.0;
        for _ in 0..600 {
            spring.update(1.0 / 120.0);
            peak = peak.max(spring.position);
        }
        assert!((spring.position - 100.0).abs() < 0.5, "ended at {}", spring.position);
        assert!(peak <= 100.0 + 1e-6, "overshot to {peak}");
    }

    #[test]
    fn a_higher_omega_settles_sooner() {
        // A short window, because a slow spring still gets there eventually — the
        // difference only exists while it is still travelling.
        //
        // Both start at 0 heading for 100: `with_omega` puts a spring *at rest on*
        // its target, so the position has to be displaced afterwards or there is
        // nothing to settle from.
        let mut slow = Spring::with_omega(100.0, 8.0);
        slow.position = 0.0;
        let mut fast = Spring::with_omega(100.0, 40.0);
        fast.position = 0.0;
        for _ in 0..8 {
            slow.update(1.0 / 60.0);
            fast.update(1.0 / 60.0);
        }
        assert!(
            fast.error() < slow.error(),
            "fast {} should beat slow {}",
            fast.error(),
            slow.error()
        );
    }

    #[test]
    fn frame_rate_does_not_change_the_outcome() {
        // Sub-stepping means a 30 fps caller and a 240 fps caller agree — the
        // export must not depend on how the frames happen to fall.
        let mut coarse = Spring::between(0.0, 50.0);
        let mut fine = Spring::between(0.0, 50.0);
        for _ in 0..30 {
            coarse.update(1.0 / 30.0);
        }
        for _ in 0..240 {
            fine.update(1.0 / 240.0);
        }
        assert!(
            (coarse.position - fine.position).abs() < 0.5,
            "coarse {} vs fine {}",
            coarse.position,
            fine.position
        );
    }

    #[test]
    fn degenerate_timesteps_are_ignored() {
        let mut spring = Spring::between(0.0, 10.0);
        spring.update(0.0);
        spring.update(-1.0);
        spring.update(f64::NAN);
        assert_eq!(spring.position, 0.0);
    }

    #[test]
    fn reset_clears_velocity() {
        let mut spring = Spring::between(0.0, 100.0);
        spring.update(0.05);
        assert!(spring.velocity.abs() > 0.0);
        spring.reset(7.0);
        assert_eq!(spring.position, 7.0);
        assert_eq!(spring.target, 7.0);
        assert_eq!(spring.velocity, 0.0);
    }
}
