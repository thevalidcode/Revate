//! Zoom planning: input events in, a clean list of zoom segments out.
//!
//! # The rules
//!
//! * A **click** zooms in on the point that was clicked — that is the whole
//!   point of the feature. A short, punchy segment, not a long one: a click is
//!   an accent, and holding the zoom for four seconds would turn a gesture into
//!   a scene.
//! * Clicks **close together merge** into one segment. Someone double-clicking a
//!   toolbar, or clicking through a menu, should read as one action, not three
//!   separate lurches.
//! * A **dwell** (the pointer resting in one spot) earns a gentler, slightly
//!   longer zoom, because attention is on that spot even though nothing was
//!   clicked. It is a weaker signal, so it gets a weaker zoom — and it is
//!   suppressed inside a click's own window, where it is just the click's tail.
//! * Segments that would **overlap are resolved, not stacked**: the later one
//!   wins the contested time, because the most recent thing the user did is what
//!   they are looking at.
//!
//! Everything is clamped to the video's own bounds, and the output is sorted,
//! non-overlapping, and never longer than the recording.

use crate::input::cursor::CursorTrack;
use crate::input::event::InputEvent;

/// How long a click's zoom holds before easing back out, in seconds.
pub const CLICK_HOLD: f64 = 1.5;
/// How long a dwell's zoom holds, in seconds.
pub const DWELL_HOLD: f64 = 2.0;
/// Zoom multiplier for a click.
pub const CLICK_ZOOM: f64 = 1.8;
/// Zoom multiplier for a dwell — deliberately gentler than a click's.
pub const DWELL_ZOOM: f64 = 1.35;
/// Longest gap between two clicks that still counts as "consecutive".
pub const MERGE_GAP: f64 = 0.6;
/// …and the distance (in video pixels) within which that still counts.
pub const MERGE_RADIUS: f64 = 120.0;
/// A dwell must last at least this long to be worth a zoom, in seconds.
pub const MIN_DWELL: f64 = 0.8;
/// Never zoom past this, whatever the events say.
pub const MAX_ZOOM: f64 = 4.0;

/// Why a segment exists. Used by the UI to label and colour it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomReason {
    /// The user clicked here.
    Click,
    /// The pointer rested here.
    Dwell,
}

/// One continuous stretch of zoom.
///
/// Times are seconds from the start of the recording, matching the video's own
/// `t = 0`. The target is in video pixels and is already clamped so the whole
/// crop stays inside the frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoomSegment {
    pub start_t: f64,
    pub end_t: f64,
    /// Focus point, in video pixels.
    pub target_x: f64,
    pub target_y: f64,
    /// 1.0 is the full frame; larger is more zoomed in.
    pub zoom_level: f64,
    pub reason: ZoomReason,
}

impl ZoomSegment {
    /// Length of the segment in seconds.
    pub fn duration(&self) -> f64 {
        (self.end_t - self.start_t).max(0.0)
    }

    /// True when `t` falls inside the segment.
    pub fn contains(&self, t: f64) -> bool {
        t >= self.start_t && t < self.end_t
    }
}

/// Plan the zoom for one take.
///
/// Returns an empty vector when there are no events — the caller treats that as
/// "export the video untouched" rather than as an error.
///
/// `events` is accepted alongside `track` so a caller can plan from a filtered
/// subset (say, clicks from one monitor) while the track still supplies the
/// geometry the clamping needs.
pub fn plan_zoom_segments(_events: &[InputEvent], track: &CursorTrack) -> Vec<ZoomSegment> {
    let (width, height) = (track.width(), track.height());
    if width <= 0.0 || height <= 0.0 {
        return Vec::new();
    }

    let mut raw: Vec<ZoomSegment> = Vec::new();

    // ---- Clicks ----
    for click in track.clicks() {
        raw.push(ZoomSegment {
            start_t: click.t,
            end_t: click.t + CLICK_HOLD,
            target_x: click.x,
            target_y: click.y,
            zoom_level: CLICK_ZOOM,
            reason: ZoomReason::Click,
        });
    }

    // ---- Dwells ----
    // Suppressed when they collide with a click: the pointer resting before or
    // after a click is that click's lead-in and tail, not separate attention.
    //
    // The time test *touches* rather than strictly overlaps, and the position
    // test is included, because the common case is a dwell that ends at the
    // exact instant of a click — same point, zero gap. A strict overlap test
    // lets that through and produces a dwell segment a frame long of the
    // click's own lead-in, which shows up as a zoom that starts too early.
    for dwell in track.dwells(MIN_DWELL) {
        let claimed_by_click = raw.iter().any(|s| {
            dwell.start_t <= s.end_t
                && dwell.end_t >= s.start_t
                && within_radius(dwell.x, dwell.y, s.target_x, s.target_y, MERGE_RADIUS)
        });
        if claimed_by_click {
            continue;
        }
        raw.push(ZoomSegment {
            start_t: dwell.start_t,
            end_t: dwell.start_t + DWELL_HOLD,
            target_x: dwell.x,
            target_y: dwell.y,
            zoom_level: DWELL_ZOOM,
            reason: ZoomReason::Dwell,
        });
    }

    if raw.is_empty() {
        return Vec::new();
    }

    raw.sort_by(|a, b| {
        a.start_t
            .partial_cmp(&b.start_t)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let merged = merge_nearby(raw);
    let resolved = resolve_overlaps(merged);
    clamp_all(resolved, width, height)
}

fn within_radius(x1: f64, y1: f64, x2: f64, y2: f64, radius: f64) -> bool {
    let dx = x2 - x1;
    let dy = y2 - y1;
    dx * dx + dy * dy <= radius * radius
}

/// Merge segments that are close in both time and space.
///
/// Consecutive clicks land in one segment whose zoom is the strongest of the
/// group and whose focus is the *mean* of its clicks — the point the user was
/// working at, not whichever click happened to be last.
fn merge_nearby(mut segments: Vec<ZoomSegment>) -> Vec<ZoomSegment> {
    let mut out: Vec<ZoomSegment> = Vec::with_capacity(segments.len());

    while !segments.is_empty() {
        let mut segment = segments.remove(0);
        let mut count = 1.0f64;
        let mut reason = segment.reason;

        // Absorb anything that starts within MERGE_GAP of where this group ends
        // and lands within MERGE_RADIUS of its focus.
        while let Some(candidate) = segments.first().copied() {
            if candidate.start_t > segment.end_t + MERGE_GAP {
                break;
            }
            if !within_radius(
                segment.target_x,
                segment.target_y,
                candidate.target_x,
                candidate.target_y,
                MERGE_RADIUS,
            ) {
                break;
            }
            segments.remove(0);

            // A click inside a dwell group promotes it: the stronger signal wins.
            if candidate.reason == ZoomReason::Click {
                reason = ZoomReason::Click;
            }
            // Running mean of the focus points.
            segment.target_x += (candidate.target_x - segment.target_x) / (count + 1.0);
            segment.target_y += (candidate.target_y - segment.target_y) / (count + 1.0);
            count += 1.0;
            segment.zoom_level = segment.zoom_level.max(candidate.zoom_level);
            segment.end_t = segment.end_t.max(candidate.end_t);
        }
        segment.reason = reason;
        out.push(segment);
    }

    out
}

/// Resolve overlaps so the output is strictly non-overlapping.
///
/// The later segment wins the contested time: it is the most recent intent, and
/// cutting the earlier one short is far less jarring than stacking two focus
/// points on top of each other.
fn resolve_overlaps(segments: Vec<ZoomSegment>) -> Vec<ZoomSegment> {
    let mut out: Vec<ZoomSegment> = Vec::with_capacity(segments.len());

    for segment in segments {
        if let Some(last) = out.last_mut() {
            if segment.start_t < last.end_t {
                // Trim the earlier segment back to where this one takes over.
                last.end_t = segment.start_t;
            }
        }
        match out.last() {
            // The later segment swallowed the previous one entirely.
            Some(last) if last.end_t <= last.start_t => {
                out.pop();
                out.push(segment);
            }
            _ => out.push(segment),
        }
    }

    out
}

/// Clamp the zoom level and pull the focus point in so the crop cannot fall off
/// the edge of the frame.
fn clamp_all(mut segments: Vec<ZoomSegment>, width: f64, height: f64) -> Vec<ZoomSegment> {
    for segment in &mut segments {
        segment.zoom_level = segment.zoom_level.clamp(1.0, MAX_ZOOM);

        // At zoom z the visible box is (width/z, height/z), so the centre has to
        // stay within half a box of the edges.
        let half_w = width / (2.0 * segment.zoom_level);
        let half_h = height / (2.0 * segment.zoom_level);
        segment.target_x = segment.target_x.clamp(half_w, width - half_w);
        segment.target_y = segment.target_y.clamp(half_h, height - half_h);
    }
    segments
}

/// The segment active at `t`, if any. Segments are non-overlapping, so there is
/// at most one.
pub fn segment_at(segments: &[ZoomSegment], t: f64) -> Option<&ZoomSegment> {
    segments.iter().find(|s| s.contains(t))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::schema::MouseButtonName;
    use crate::input::event::ScreenPoint;
    use crate::utils::capture_meta::{CaptureMeta, PixelRect};

    /// A track over a full-frame 1920×1080 recording, built from raw events.
    fn track_from(events: &[InputEvent]) -> CursorTrack {
        let meta = CaptureMeta {
            display: PixelRect::new(0, 0, 1920, 1080),
            primary_scale_factor: 1.0,
            region: None,
            fps: 30,
            cursor_baked_in: false,
        };
        CursorTrack::from_events(events, &meta, 1920, 1080)
    }

    fn click_at(t: f64, x: f64, y: f64) -> InputEvent {
        InputEvent::mouse_down(t * 1000.0, MouseButtonName::Left, ScreenPoint { x, y })
    }

    fn move_to(t: f64, x: f64, y: f64) -> InputEvent {
        InputEvent::cursor_move(t * 1000.0, ScreenPoint { x, y })
    }

    #[test]
    fn no_events_means_no_zoom() {
        let track = track_from(&[]);
        assert!(plan_zoom_segments(&[], &track).is_empty());
    }

    #[test]
    fn a_click_produces_one_segment_at_that_point() {
        let events = vec![move_to(0.0, 900.0, 500.0), click_at(1.0, 900.0, 500.0)];
        let track = track_from(&events);
        let plan = plan_zoom_segments(&events, &track);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].reason, ZoomReason::Click);
        assert!((plan[0].start_t - 1.0).abs() < 1e-6);
        assert!((plan[0].zoom_level - CLICK_ZOOM).abs() < 1e-9);
        assert!((plan[0].target_x - 900.0).abs() < 1e-6);
    }

    #[test]
    fn nearby_clicks_merge_into_one_segment() {
        let events = vec![
            move_to(0.0, 500.0, 500.0),
            click_at(1.0, 500.0, 500.0),
            click_at(1.2, 505.0, 502.0),
            click_at(1.4, 502.0, 498.0),
        ];
        let track = track_from(&events);
        let plan = plan_zoom_segments(&events, &track);
        assert_eq!(plan.len(), 1, "three close clicks should read as one");
        // Extended to cover the last of them.
        assert!(plan[0].end_t >= 1.4 + CLICK_HOLD - 1e-6);
    }

    #[test]
    fn clicks_far_apart_stay_separate() {
        let events = vec![
            move_to(0.0, 200.0, 500.0),
            click_at(1.0, 200.0, 500.0),
            move_to(5.0, 1600.0, 500.0),
            click_at(6.0, 1600.0, 500.0),
        ];
        let track = track_from(&events);
        let plan = plan_zoom_segments(&events, &track);
        assert_eq!(plan.len(), 2);
        assert!(plan[1].start_t > plan[0].end_t, "segments must not overlap");
    }

    #[test]
    fn clicks_close_in_time_but_far_in_space_do_not_merge() {
        let events = vec![
            move_to(0.0, 100.0, 500.0),
            click_at(1.0, 100.0, 500.0),
            move_to(1.1, 1800.0, 500.0),
            click_at(1.2, 1800.0, 500.0),
        ];
        let track = track_from(&events);
        let plan = plan_zoom_segments(&events, &track);
        assert_eq!(plan.len(), 2, "same moment, different place");
    }

    #[test]
    fn a_long_dwell_earns_a_gentler_zoom() {
        let mut events = vec![move_to(0.0, 800.0, 400.0)];
        // 1.5 s of stillness.
        for i in 1..=90 {
            events.push(move_to(i as f64 * 0.016, 801.0, 401.0));
        }
        let track = track_from(&events);
        let plan = plan_zoom_segments(&events, &track);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].reason, ZoomReason::Dwell);
        assert!(
            plan[0].zoom_level < CLICK_ZOOM,
            "a dwell should be gentler than a click"
        );
    }

    #[test]
    fn a_brief_pause_is_not_a_dwell() {
        let mut events = vec![move_to(0.0, 800.0, 400.0)];
        for i in 1..=20 {
            events.push(move_to(i as f64 * 0.016, 801.0, 401.0));
        }
        let track = track_from(&events);
        assert!(plan_zoom_segments(&events, &track).is_empty());
    }

    #[test]
    fn a_dwell_after_a_click_does_not_double_up() {
        let mut events = vec![move_to(0.0, 800.0, 400.0), click_at(1.0, 800.0, 400.0)];
        for i in 1..=60 {
            events.push(move_to(1.0 + i as f64 * 0.016, 801.0, 401.0));
        }
        let track = track_from(&events);
        let plan = plan_zoom_segments(&events, &track);
        assert_eq!(plan.len(), 1, "the rest after a click is the click's tail");
        assert_eq!(plan[0].reason, ZoomReason::Click);
    }

    #[test]
    fn the_focus_point_stays_inside_the_frame() {
        // A click right in the corner: at 1.8x the visible box is smaller than
        // the distance to the edge, so the centre has to be pulled in.
        let events = vec![move_to(0.0, 5.0, 5.0), click_at(1.0, 5.0, 5.0)];
        let track = track_from(&events);
        let plan = plan_zoom_segments(&events, &track);
        let segment = plan[0];
        let half_w = 1920.0 / (2.0 * segment.zoom_level);
        let half_h = 1080.0 / (2.0 * segment.zoom_level);
        assert!(segment.target_x >= half_w - 1e-6);
        assert!(segment.target_y >= half_h - 1e-6);
    }

    #[test]
    fn the_plan_is_sorted_and_never_overlapping() {
        let mut events = Vec::new();
        for i in 0..8 {
            let t = 1.0 + i as f64 * 0.9;
            events.push(move_to(t - 0.1, 300.0 + i as f64, 300.0));
            events.push(click_at(t, 300.0 + i as f64, 300.0));
        }
        let track = track_from(&events);
        let plan = plan_zoom_segments(&events, &track);
        for pair in plan.windows(2) {
            assert!(
                pair[0].end_t <= pair[1].start_t + 1e-9,
                "{} overlaps {}",
                pair[0].end_t,
                pair[1].start_t
            );
        }
    }

    #[test]
    fn segment_at_finds_the_active_segment() {
        let segment = ZoomSegment {
            start_t: 1.0,
            end_t: 2.0,
            target_x: 0.0,
            target_y: 0.0,
            zoom_level: 2.0,
            reason: ZoomReason::Click,
        };
        let segments = vec![segment];
        assert!(segment_at(&segments, 1.5).is_some());
        assert!(segment_at(&segments, 0.9).is_none());
        assert!(segment_at(&segments, 2.5).is_none());
        assert!((segment.duration() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn zoom_never_exceeds_the_cap() {
        let mut events = vec![move_to(0.0, 500.0, 500.0)];
        for i in 0..50 {
            events.push(click_at(1.0 + i as f64 * 0.05, 500.0, 500.0));
        }
        let track = track_from(&events);
        let plan = plan_zoom_segments(&events, &track);
        for segment in plan {
            assert!(segment.zoom_level >= 1.0 && segment.zoom_level <= MAX_ZOOM);
        }
    }
}
