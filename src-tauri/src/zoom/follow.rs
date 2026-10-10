//! Shared building blocks for cursor-following motion.
//!
//! The zoom timeline and the cursor smoothing both chase step-function targets
//! with springs — retargeted at fixed steps, velocity always carried across
//! retargets. There are no animation durations and no boundary special-cases:
//! segment starts/ends/re-aims are just target changes the spring smooths
//! through into continuous motion.
//!
//! The two integrators below look redundant. They are not:
//! [`SpringMassDamper`] advances with the closed-form solution, so the answer
//! is exact for any step size and identical for 30 fps and 240 fps callers.
//! Euler integration (`Spring::update`) accumulates a step-size-dependent error
//! that compounds over a 36 000-row timeline.

/// Physically-driven spring parameters, in the tension/mass/friction form.
#[derive(Debug, Clone, Copy)]
pub struct SpringConfig {
    /// Spring stiffness. Higher pulls harder toward the target.
    pub tension: f64,
    /// Mass the spring moves. Higher is slower and heavier.
    pub mass: f64,
    /// Damping. At exactly `2 * sqrt(tension * mass)` the spring is critically
    /// damped: the fastest approach that never overshoots.
    pub friction: f64,
}

impl SpringConfig {
    /// The damping ratio: 1.0 is critical, below rings.
    pub fn zeta(&self) -> f64 {
        let stiffness = self.tension.max(f64::EPSILON);
        let mass = self.mass.max(f64::EPSILON);
        self.friction / (2.0 * (stiffness * mass).sqrt())
    }

    /// The undamped natural frequency, in rad/s.
    pub fn omega0(&self) -> f64 {
        (self.tension.max(f64::EPSILON) / self.mass.max(f64::EPSILON)).sqrt()
    }
}


/// A one-dimensional spring advanced by the analytic solution.
///
/// Holds the spring's *state* (position, velocity, target); each
/// [`run`](Self::run) call moves it forward chasing the current target.
/// Velocity survives retargets, which is what makes a re-aim mid-flight glide
/// instead of snapping: the spring is already moving and bends toward the goal.
#[derive(Debug, Clone)]
pub struct SpringMassDamper {
    config: SpringConfig,
    position: f64,
    velocity: f64,
    target: f64,
}

impl SpringMassDamper {
    pub fn new(config: SpringConfig, start: f64) -> Self {
        Self {
            config,
            position: start,
            velocity: 0.0,
            target: start,
        }
    }

    pub fn set_target(&mut self, target: f64) {
        self.target = target;
    }

    pub fn position(&self) -> f64 {
        self.position
    }

    pub fn velocity(&self) -> f64 {
        self.velocity
    }

    /// Advance by `dt` seconds toward the current target, exactly.
    ///
    /// The closed form is evaluated per step, so a 30 fps caller and a 240 fps
    /// caller agree to floating-point noise — the export must not depend on how
    /// the frames happen to fall. Non-finite or non-positive steps are ignored.
    pub fn run(&mut self, dt: f64) -> f64 {
        if !dt.is_finite() || dt <= 0.0 {
            return self.position;
        }
        let (disp, vel) = solve_spring_1d(
            self.position - self.target,
            self.velocity,
            dt,
            self.config.omega0(),
            self.config.zeta(),
        );
        self.position = self.target + disp;
        self.velocity = vel;

        // Snap to rest: below threshold the spring would crawl for seconds
        // chasing the last sub-pixel, and a crop one pixel off full-frame is
        // visually identical but keeps the command files growing.
        let disp_mag = (self.position - self.target).abs();
        if disp_mag < 1e-5 && self.velocity.abs() < 1e-4 {
            self.position = self.target;
            self.velocity = 0.0;
        }
        self.position
    }
}

/// The exact displacement/velocity after `t` seconds for a spring starting
/// `displacement` away from its target with `velocity`, given ω₀ and ζ.
///
/// Three regimes: underdamped rings (decaying sine), overdamped crawls (two
/// exponentials), and critically damped or near it converges fastest without
/// overshooting — the regime every zoom and cursor channel here lives in.
fn solve_spring_1d(
    displacement: f64,
    velocity: f64,
    t: f64,
    omega0: f64,
    zeta: f64,
) -> (f64, f64) {
    const CRITICAL_EPSILON: f64 = 0.01;

    if zeta < 1.0 - CRITICAL_EPSILON {
        let omega_d = omega0 * (1.0 - zeta * zeta).sqrt();
        let decay = (-zeta * omega0 * t).exp();
        let cos_term = (omega_d * t).cos();
        let sin_term = (omega_d * t).sin();

        let a = displacement;
        let b = (velocity + displacement * zeta * omega0) / omega_d.max(1e-4);

        let new_disp = decay * (a * cos_term + b * sin_term);
        let new_vel = decay
            * ((b * omega_d - a * zeta * omega0) * cos_term
                - (a * omega_d + b * zeta * omega0) * sin_term);
        (new_disp, new_vel)
    } else if zeta > 1.0 + CRITICAL_EPSILON {
        let sqrt_term = (zeta * zeta - 1.0).sqrt();
        let s1 = -omega0 * (zeta - sqrt_term);
        let s2 = -omega0 * (zeta + sqrt_term);
        let denom = s1 - s2;

        if denom.abs() < 1e-10 {
            let s_avg = 0.5 * (s1 + s2);
            let decay = (s_avg * t).exp();
            let new_disp = decay * (displacement + (velocity - displacement * s_avg) * t);
            let new_vel = decay
                * ((velocity - displacement * s_avg)
                    + s_avg * (displacement + (velocity - displacement * s_avg) * t));
            (new_disp, new_vel)
        } else {
            let c1 = (velocity - displacement * s2) / denom;
            let c2 = displacement - c1;
            let e1 = (s1 * t).exp();


            let e2 = (s2 * t).exp();
            (c1 * e1 + c2 * e2, c1 * s1 * e1 + c2 * s2 * e2)
        }
    } else {
        let decay = (-omega0 * t).exp();
        let a = displacement;
        let b = velocity + displacement * omega0;
        let new_disp = decay * (a + b * t);
        let new_vel = decay * (b - omega0 * (a + b * t));
        (new_disp, new_vel)
    }
}

use super::viewport::snap_to_edges;

/// How often the follow state is integrated, in seconds.
///
/// 8 ms (125 Hz) matches the sampling density the cursor smoothing already
/// uses, so the zoom and the pointer it chases move on the same clock and the
/// cursor never visibly lags its own frame.
pub const FOLLOW_STEP: f64 = 0.008;

/// Default spring for the zoom *amount* channel: critically damped, settling in
/// roughly half a second — slow enough to read as a glide, fast enough that a
/// 1.5 s click segment actually arrives.
pub const AMOUNT_SPRING: SpringConfig = SpringConfig {
    tension: 90.0,
    mass: 1.0,
    friction: 19.0,
};

/// Default spring for the viewport *center* channels: critically damped and a
/// touch slower than the amount, so the frame pans into place a beat after the
/// zoom starts rather than sliding and scaling in lockstep.
pub const CENTER_SPRING: SpringConfig = SpringConfig {
    tension: 65.0,
    mass: 1.0,
    friction: 16.2,
};

/// The zoom transform at one instant: how far in, and where aimed.
///
/// The center is in Cap's normalized "travel space": 0 pins the visible
/// viewport flush to the left/top edge, 1 flush to the right/bottom, 0.5 dead
/// centre. The full [0, 1] range is exactly the set of in-bounds framings, so
/// edge content stays reachable — unlike centring the viewport on the focus,
/// which strands a band around every edge outside the frame.
#[derive(Debug, Clone, Copy)]
pub struct FollowState {
    /// Smoothed zoom level: 1.0 is the full frame.
    pub amount: f64,
    /// Smoothed travel-space center, each axis in [0, 1].
    pub center_x: f64,
    pub center_y: f64,
}

/// Step-function targets for one instant: the zoom level to chase, and where to
/// aim it.
///
/// Outside every segment the target is "no zoom"; the center target is kept
/// sensible anyway so re-entry starts from somewhere sane rather than a corner.
#[derive(Debug, Clone, Copy)]
pub struct FollowTargets {
    pub amount: f64,
    pub center_x: f64,
    pub center_y: f64,
}

/// The follow state advanced one fixed step toward its targets.
///
/// Each channel is its own spring, retargeted every step with velocity carried
/// across — the same construction as the cursor smoothing, so the camera and
/// the pointer it follows share one feel.
pub fn step_follow(
    amount: &mut SpringMassDamper,
    center_x: &mut SpringMassDamper,
    center_y: &mut SpringMassDamper,
    targets: FollowTargets,
    dt: f64,
) -> FollowState {
    amount.set_target(targets.amount);
    center_x.set_target(targets.center_x);
    center_y.set_target(targets.center_y);
    FollowState {
        amount: amount.run(dt).max(1.0),
        center_x: center_x.run(dt).clamp(0.0, 1.0),
        center_y: center_y.run(dt).clamp(0.0, 1.0),
    }
}

/// Map a focus point (each axis a 0..1 fraction across the frame) to a
/// travel-space center for an auto zoom: edge-snap each axis, then clamp.
///
/// A focus at 0 or 1 reaches the corner-flush framing, so content hard against
/// an edge is actually reachable.
pub fn follow_center(focus_x: f64, focus_y: f64, edge_snap_ratio: f64) -> (f64, f64) {
    (
        snap_to_edges(focus_x, edge_snap_ratio).clamp(0.0, 1.0),
        snap_to_edges(focus_y, edge_snap_ratio).clamp(0.0, 1.0),
    )
}
