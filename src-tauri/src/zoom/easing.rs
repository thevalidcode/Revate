//! Easing functions for zoom transitions.
//!
//! Only one curve is used by the planner — `ease_in_out`, a smoothstep — but the
//! split forms are here because the FFmpeg side needs the *same* curve, spelled
//! as an expression, and having the Rust and the filter string derive from one
//! documented set of shapes is what keeps them in agreement. See
//! [`ease_in_out_expr`].

/// Quadratic ease-in: slow to start, fast to finish.
pub fn ease_in(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t
}

/// Quadratic ease-out: fast to start, slow to finish.
pub fn ease_out(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t) * (1.0 - t)
}

/// Smoothstep: zero slope at both ends, so a zoom neither snaps nor stops dead.
///
/// This is the default for zoom in and out. A linear move is noticeably worse
/// here — the eye reads an abrupt velocity change at the start of a move as a
/// cut, which is exactly what a zoom is meant to avoid.
pub fn ease_in_out(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The same smoothstep as [`ease_in_out`], as an FFmpeg expression in `t`.
///
/// `t` here is the filter's own time variable in seconds and `start`/`dur` are
/// the segment's start and duration, so the expression evaluates to the eased
/// *progress* (0→1) across the transition. Emitted as `clip(...,0,1)` rather
/// than a clamp so the value cannot escape the range even if the caller rounds
/// the bounds into the negatives.
pub fn ease_in_out_expr(var: &str, start: f64, dur: f64) -> String {
    let p = format!("clip(({var}-{start:.4})/{dur:.4},0,1)");
    format!("({p})*({p})*(3-2*({p}))")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_curve_is_pinned_at_both_ends() {
        for f in [ease_in, ease_out, ease_in_out] {
            assert!((f(0.0)).abs() < 1e-9, "f(0) should be 0");
            assert!((f(1.0) - 1.0).abs() < 1e-9, "f(1) should be 1");
        }
    }

    #[test]
    fn curves_clamp_outside_the_unit_interval() {
        for f in [ease_in, ease_out, ease_in_out] {
            assert!((f(-5.0)).abs() < 1e-9);
            assert!((f(9.0) - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn curves_are_monotonic() {
        let mut previous = 0.0;
        for i in 0..=100 {
            let v = ease_in_out(i as f64 / 100.0);
            assert!(v >= previous, "ease_in_out went backwards at {i}");
            previous = v;
        }
    }

    #[test]
    fn smoothstep_has_no_slope_at_the_ends() {
        // The whole point of the default curve: near 0 and near 1 it is flat, so
        // a zoom eases in rather than snapping.
        let a = ease_in_out(0.01);
        let b = ease_in_out(0.1);
        assert!(a < 0.01, "starts flat, got {a}");
        assert!(b < 0.1, "still slow a tenth in, got {b}");

        let c = ease_in_out(0.9);
        let d = ease_in_out(0.99);
        assert!(c > 0.9, "still close to the target near the end, got {c}");
        assert!(d > 0.99, "lands flat, got {d}");
    }

    #[test]
    fn in_and_out_are_mirror_images() {
        for i in 0..=20 {
            let t = i as f64 / 20.0;
            assert!((ease_in(t) + ease_out(1.0 - t) - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn the_ffmpeg_expression_mentions_the_time_variable() {
        let expr = ease_in_out_expr("t", 1.5, 0.4);
        assert!(expr.contains("t"));
        assert!(expr.contains("1.5000"));
        assert!(expr.contains("0.4000"));
        // Progress is clipped, so it can never be evaluated outside 0..1.
        assert!(expr.starts_with("(clip("));
    }
}
