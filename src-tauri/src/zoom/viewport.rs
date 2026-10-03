//! The visible region at any instant — the bridge between the planner's
//! intentions and the two things that act on them: the editor's preview, and
//! FFmpeg's `crop` filter.
//!
//! [`Viewport::at`] is a **pure function of time**. That is the property that
//! matters: the editor can scrub to a timestamp and the export can be re-run,
//! and both get the identical rectangle. Nothing here integrates state, and
//! nothing depends on the order frames are visited.
//!
//! Dimensions are forced even, because `yuv420p` requires it — an odd crop size
//! makes FFmpeg fail rather than round, and a failed export is a much worse
//! outcome than a one-pixel difference in the framing.

use super::easing::ease_in_out;
use super::planner::{segment_at, ZoomSegment};

/// How long the zoom takes to travel in and out, in seconds.
///
/// Applied at both ends of a segment, so a 1.5 s click zoom is really ~1.2 s of
/// hold with a 0.15 s ease each way.
pub const TRANSITION: f64 = 0.15;

/// The visible rectangle, in video pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// Left edge, in source pixels.
    pub x: f64,
    /// Top edge, in source pixels.
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Viewport {
    /// The un-zoomed frame, i.e. the whole video.
    pub fn full(frame_width: f64, frame_height: f64) -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: frame_width,
            height: frame_height,
        }
    }

    /// Centre point, in source pixels.
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    /// Effective zoom relative to the full frame, from the width alone.
    pub fn zoom(&self, frame_width: f64) -> f64 {
        if frame_width <= 0.0 {
            1.0
        } else {
            frame_width / self.width
        }
    }

    /// True when this is the whole frame, so a caller can skip a no-op filter.
    pub fn is_full(&self, frame_width: f64, frame_height: f64) -> bool {
        (self.width - frame_width).abs() < 1.0 && (self.height - frame_height).abs() < 1.0
    }

    /// Round to the dimensions `yuv420p` accepts, then push the origin back
    /// inside the frame — rounding the size moves the far edge, and without this
    /// a crop at the right edge would end one pixel outside.
    pub fn to_even(mut self, frame_width: f64, frame_height: f64) -> Self {
        self.width = (self.width.round() / 2.0).floor() * 2.0;
        self.height = (self.height.round() / 2.0).floor() * 2.0;
        self.width = self.width.clamp(2.0, frame_width);
        self.height = self.height.clamp(2.0, frame_height);
        self.x = self.x.clamp(0.0, (frame_width - self.width).max(0.0));
        self.y = self.y.clamp(0.0, (frame_height - self.height).max(0.0));
        self
    }

    /// The rectangle at time `t`, in seconds.
    ///
    /// Outside every segment this is the full frame; inside one, the crop is
    /// centred on the segment's target at an eased zoom level. The ease is the
    /// same smoothstep the FFmpeg side spells as an expression, so the preview
    /// and the export move identically.
    pub fn at(frame_width: f64, frame_height: f64, segments: &[ZoomSegment], t: f64) -> Self {
        let Some(segment) = segment_at(segments, t) else {
            return Viewport::full(frame_width, frame_height);
        };
        Self::for_segment(frame_width, frame_height, segment, t)
    }

    /// The rectangle for one segment at time `t`, without needing the list.
    pub fn for_segment(frame_width: f64, frame_height: f64, segment: &ZoomSegment, t: f64) -> Self {
        if frame_width <= 0.0 || frame_height <= 0.0 {
            return Viewport::full(frame_width, frame_height);
        }

        let zoom = 1.0 + (segment.zoom_level - 1.0) * ease_in_out(progress_through(segment, t));

        let width = frame_width / zoom;
        let height = frame_height / zoom;
        // The planner already clamped the target, but clamp again: this function
        // is also called with hand-built segments from tests and the UI.
        let half_w = width / 2.0;
        let half_h = height / 2.0;
        let cx = segment.target_x.clamp(half_w, frame_width - half_w);
        let cy = segment.target_y.clamp(half_h, frame_height - half_h);

        Viewport {
            x: cx - half_w,
            y: cy - half_h,
            width,
            height,
        }
    }

    /// The rectangle for one segment at time `t`, keeping `cursor` in view.
    ///
    /// This is the auto-zoom's "auto" half, and it mirrors the decision Cap's
    /// `calculate_zoom_and_center_for_cursor` makes: the camera first aims at
    /// the planned focus point — snapped away from the frame's edges by
    /// `edge_snap_ratio`, so a target near a corner does not slam the viewport
    /// against it and then have to wrench back — and is then nudged the
    /// *smallest* distance that puts the pointer back inside the visible area.
    ///
    /// `cursor` is `None` for takes with no trail: the viewport is then the
    /// plain centred crop, which is also what a hand-built segment gets.
    pub fn following(
        frame_width: f64,
        frame_height: f64,
        segment: &ZoomSegment,
        t: f64,
        cursor: Option<(f64, f64)>,
        edge_snap_ratio: f64,
    ) -> Self {
        if frame_width <= 0.0 || frame_height <= 0.0 {
            return Viewport::full(frame_width, frame_height);
        }

        let zoom = 1.0 + (segment.zoom_level - 1.0) * ease_in_out(progress_through(segment, t));
        let width = frame_width / zoom;
        let height = frame_height / zoom;
        let half_w = width / 2.0;
        let half_h = height / 2.0;

        // Where the camera wants to be, in the frame's own coordinates…
        let focus_x = snap_to_edges(segment.target_x / frame_width, edge_snap_ratio);
        let focus_y = snap_to_edges(segment.target_y / frame_height, edge_snap_ratio);

        // …mapped onto the range the centre may move through, then clamped so
        // the rectangle never leaves the frame.
        let min_cx = half_w;
        let max_cx = frame_width - half_w;
        let min_cy = half_h;
        let max_cy = frame_height - half_h;
        let mut cx = (min_cx + focus_x * (max_cx - min_cx)).clamp(min_cx, max_cx);
        let mut cy = (min_cy + focus_y * (max_cy - min_cy)).clamp(min_cy, max_cy);

        // The follow: a pointer that has wandered off the visible area pulls
        // the camera just far enough to be back on it. When it is already
        // visible nothing happens — that is what keeps the zoom calm during a
        // click instead of gluing the frame to every hand tremor.
        if let Some((ux, uy)) = cursor {
            if ux < cx - half_w {
                cx = (ux + half_w).clamp(min_cx, max_cx);
            } else if ux > cx + half_w {
                cx = (ux - half_w).clamp(min_cx, max_cx);
            }
            if uy < cy - half_h {
                cy = (uy + half_h).clamp(min_cy, max_cy);
            } else if uy > cy + half_h {
                cy = (uy - half_h).clamp(min_cy, max_cy);
            }
        }

        Viewport {
            x: cx - half_w,
            y: cy - half_h,
            width,
            height,
        }
    }
}

/// Keep a normalised coordinate out of the frame's outer `ratio`.
///
/// A focus point 20 px from the left edge would otherwise place the viewport's
/// centre essentially at the frame's edge; snapping it to the inner band means
/// the camera stops short, which reads as deliberate framing rather than as a
/// crop that ran out of room.
pub fn snap_to_edges(value: f64, edge_snap_ratio: f64) -> f64 {
    let ratio = if edge_snap_ratio.is_finite() {
        edge_snap_ratio.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let band = ratio / 2.0;
    value.clamp(band, 1.0 - band)
}

/// Eased progress through a segment: 0 at its start, 1 at its end.
///
/// The ease is applied to the *edges* only — `TRANSITION` seconds to travel in,
/// a flat hold in the middle, `TRANSITION` to travel out. A segment shorter
/// than two transitions (which a merge can produce) has no hold at all, which is
/// correct: it is a quick accent, and stretching it would misrepresent the
/// gesture it came from.
fn progress_through(segment: &ZoomSegment, t: f64) -> f64 {
    let span = segment.end_t - segment.start_t;
    if span <= 0.0 {
        return 1.0;
    }
    let transition = TRANSITION.min(span / 2.0);
    let local = t - segment.start_t;

    if local < transition {
        ease_in_out(local / transition)
    } else if local > span - transition {
        ease_in_out((span - local) / transition)
    } else {
        1.0
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::zoom::planner::ZoomReason;

    fn segment(start: f64, end: f64, x: f64, y: f64, zoom: f64) -> ZoomSegment {
        ZoomSegment {
            start_t: start,
            end_t: end,
            target_x: x,
            target_y: y,
            zoom_level: zoom,
            reason: ZoomReason::Click,
        }
    }

    #[test]
    fn no_segments_means_the_whole_frame() {
        assert_eq!(
            Viewport::at(1920.0, 1080.0, &[], 5.0),
            Viewport::full(1920.0, 1080.0)
        );
    }

    #[test]
    fn time_outside_every_segment_is_the_full_frame() {
        let segments = [segment(2.0, 3.0, 960.0, 540.0, 2.0)];
        assert_eq!(Viewport::at(1920.0, 1080.0, &segments, 1.0), Viewport::full(1920.0, 1080.0));
        assert_eq!(Viewport::at(1920.0, 1080.0, &segments, 9.0), Viewport::full(1920.0, 1080.0));
    }

    #[test]
    fn the_hold_reaches_the_full_zoom_level() {
        let segments = [segment(1.0, 3.0, 960.0, 540.0, 2.0)];
        let view = Viewport::at(1920.0, 1080.0, &segments, 2.0);
        assert!((view.width - 960.0).abs() < 1e-6, "width was {}", view.width);
        assert!((view.height - 540.0).abs() < 1e-6);
        assert!((view.zoom(1920.0) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn the_crop_follows_the_target() {
        let segments = [segment(1.0, 3.0, 480.0, 270.0, 2.0)];
        let view = Viewport::at(1920.0, 1080.0, &segments, 2.0);
        let (cx, cy) = view.center();
        assert!((cx - 480.0).abs() < 1e-6, "cx was {cx}");
        assert!((cy - 270.0).abs() < 1e-6, "cy was {cy}");
    }

    #[test]
    fn the_zoom_eases_in_rather_than_snapping() {
        let segments = [segment(1.0, 3.0, 960.0, 540.0, 2.0)];
        let at_start = Viewport::at(1920.0, 1080.0, &segments, 1.0);
        let just_in = Viewport::at(1920.0, 1080.0, &segments, 1.0 + TRANSITION / 2.0);
        let full = Viewport::at(1920.0, 1080.0, &segments, 1.0 + TRANSITION);

        assert!((at_start.width - 1920.0).abs() < 1e-6, "starts unzoomed");
        // Halfway through the ease the zoom is partway — not already there.
        let mid_zoom = just_in.zoom(1920.0);
        assert!(mid_zoom > 1.0 && mid_zoom < 2.0, "mid zoom was {mid_zoom}");
        assert!((full.zoom(1920.0) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn it_eases_back_out_at_the_end() {
        let segments = [segment(1.0, 3.0, 960.0, 540.0, 2.0)];
        let full = Viewport::at(1920.0, 1080.0, &segments, 2.0);
        let leaving = Viewport::at(1920.0, 1080.0, &segments, 3.0 - TRANSITION / 2.0);
        let gone = Viewport::at(1920.0, 1080.0, &segments, 3.0);
        assert!(leaving.zoom(1920.0) < full.zoom(1920.0));
        assert!((gone.width - 1920.0).abs() < 1e-6);
    }

    #[test]
    fn the_crop_never_leaves_the_frame() {
        // Target hard against the corner, before the planner's clamp.
        let segments = [segment(1.0, 3.0, 1920.0, 1080.0, 2.0)];
        for step in 0..=20 {
            let t = 1.0 + step as f64 * 0.1;
            let view = Viewport::at(1920.0, 1080.0, &segments, t);
            assert!(view.x >= -1e-6, "x was {} at t={t}", view.x);
            assert!(view.y >= -1e-6, "y was {} at t={t}", view.y);
            assert!(view.x + view.width <= 1920.0 + 1e-6, "past the right edge");
            assert!(view.y + view.height <= 1080.0 + 1e-6, "past the bottom");
        }
    }

    #[test]
    fn even_dimensions_are_guaranteed() {
        let segments = [segment(1.0, 3.0, 333.0, 277.0, 1.37)];
        let view = Viewport::at(1919.0, 1079.0, &segments, 2.0).to_even(1919.0, 1079.0);
        assert_eq!(view.width % 2.0, 0.0, "width {} is odd", view.width);
        assert_eq!(view.height % 2.0, 0.0, "height {} is odd", view.height);
    }

    #[test]
    fn rounding_the_size_keeps_the_crop_inside_the_frame() {
        let view = Viewport {
            x: 1910.0,
            y: 0.0,
            width: 11.0,
            height: 8.0,
        }
        .to_even(1920.0, 1080.0);
        assert!(view.x + view.width <= 1920.0, "crop spilled past the edge");
        assert!(view.x >= 0.0);
    }

    #[test]
    fn a_degenerate_frame_does_not_divide_by_zero() {
        let segments = [segment(1.0, 3.0, 10.0, 10.0, 2.0)];
        let view = Viewport::at(0.0, 0.0, &segments, 2.0);
        assert_eq!(view.width, 0.0);
    }

    #[test]
    fn a_zero_length_segment_is_handled() {
        let view = Viewport::for_segment(
            1920.0,
            1080.0,
            &ZoomSegment {
                start_t: 1.0,
                end_t: 1.0,
                target_x: 960.0,
                target_y: 540.0,
                zoom_level: 2.0,
                reason: ZoomReason::Click,
            },
            1.0,
        );
        assert!(view.width.is_finite() && view.width > 0.0);
    }

    #[test]
    fn a_short_segment_still_moves_without_a_hold() {
        // Shorter than two transitions: no plateau, but it must still resolve.
        let segments = [segment(1.0, 1.1, 960.0, 540.0, 2.0)];
        let mid = Viewport::at(1920.0, 1080.0, &segments, 1.05);
        assert!(mid.zoom(1920.0) > 1.0, "should be partway in: {mid:?}");
        assert!(mid.width.is_finite());
    }

    #[test]
    fn following_keeps_the_cursor_on_screen() {
        // The zoom aims at the middle, but the pointer is far off to the right.
        let segment = segment(1.0, 3.0, 960.0, 540.0, 2.0);
        let view = Viewport::following(1920.0, 1080.0, &segment, 2.0, Some((1700.0, 900.0)), 0.0);

        assert!((view.width - 960.0).abs() < 1e-6, "zoom itself is unchanged");
        assert!(
            view.x <= 1700.0 && 1700.0 <= view.x + view.width,
            "cursor x must be visible: {view:?}"
        );
        assert!(view.y <= 900.0 && 900.0 <= view.y + view.height);
        // …and it must not have overshot: the pointer sits on the right edge.
        assert!((view.x + view.width - 1700.0).abs() < 1e-6);
        assert!((view.y + view.height - 900.0).abs() < 1e-6);
    }

    #[test]
    fn following_leaves_a_visible_cursor_alone() {
        let segment = segment(1.0, 3.0, 960.0, 540.0, 2.0);
        let centred = Viewport::following(1920.0, 1080.0, &segment, 2.0, None, 0.0);
        let with_cursor =
            Viewport::following(1920.0, 1080.0, &segment, 2.0, Some((960.0, 540.0)), 0.0);
        assert_eq!(centred, with_cursor, "a centred pointer needs no follow");
    }

    #[test]
    fn following_falls_back_to_the_plain_crop_without_a_cursor() {
        let segment = segment(1.0, 3.0, 480.0, 270.0, 2.0);
        let follow = Viewport::following(1920.0, 1080.0, &segment, 2.0, None, 0.3);
        let plain = Viewport::for_segment(1920.0, 1080.0, &segment, 2.0);
        // No cursor: only the edge snap can differ, and the rect stays legal.
        assert!((follow.width - plain.width).abs() < 1e-9);
        assert!(follow.x >= 0.0 && follow.x + follow.width <= 1920.0);
    }

    #[test]
    fn edge_snap_pulls_the_camera_away_from_the_frame_edge() {
        // A target hard against the left edge, with no cursor to follow.
        let segment = segment(1.0, 3.0, 0.0, 540.0, 2.0);
        let unsnapped = Viewport::following(1920.0, 1080.0, &segment, 2.0, None, 0.0);
        let snapped = Viewport::following(1920.0, 1080.0, &segment, 2.0, None, 0.3);
        assert!((unsnapped.x - 0.0).abs() < 1e-6, "no snap means the left edge");
        assert!(snapped.x > unsnapped.x, "the snap should hold the camera back");
        // 15% of the movable range (1920 - 960) is 144 px.
        assert!((snapped.x - 144.0).abs() < 1e-6, "x was {}", snapped.x);

        assert!((snap_to_edges(0.0, 0.3) - 0.15).abs() < 1e-9);
        assert!((snap_to_edges(0.5, 0.3) - 0.5).abs() < 1e-9);
        assert!((snap_to_edges(1.0, 0.3) - 0.85).abs() < 1e-9);
        // A non-finite ratio must not poison the clamp.
        assert!((snap_to_edges(0.5, f64::NAN) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_degenerate_frame_makes_following_a_no_op() {
        let segment = segment(1.0, 3.0, 0.0, 0.0, 2.0);
        let view = Viewport::following(0.0, 0.0, &segment, 2.0, Some((10.0, 10.0)), 0.3);
        assert_eq!(view, Viewport::full(0.0, 0.0));
    }
}
