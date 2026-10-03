//! Cursor sampling in video space, with spring smoothing and click ripples.
//!
//! # Why the spring is pre-integrated
//!
//! A spring is stateful: `position(n+1)` depends on `position(n)`. That is fine
//! for a live overlay, but the export needs to ask for the cursor position at an
//! arbitrary timestamp, possibly out of order (the zoom pass and the cursor pass
//! walk time independently), and it must give the same answer every run or the
//! preview and the export would disagree.
//!
//! So the spring is integrated once, up front, at a fixed 120 Hz and stored as
//! a table. Sampling is then a binary search plus a linear blend between the two
//! neighbouring rows — deterministic, order-independent, and cheap enough to do
//! per output frame.
//!
//! # Dwell
//!
//! Two samples closer together in space *and* time than the dwell thresholds
//! below are treated as "the pointer was resting here". The planner uses that to
//! decide on a gentle zoom; nothing here needs to know about zoom.

use crate::events::schema::MouseButtonName;
use crate::input::event::InputEvent;
use crate::utils::capture_meta::CaptureMeta;

/// Fixed integration rate for the spring table.
const SPRING_HZ: f64 = 120.0;

/// The default spring angular frequency, in rad/s.
///
/// ~18 rad/s settles in roughly 150 ms, which reads as "smooth" rather than
/// "laggy". The editor's smoothing control scales this and bakes the result
/// into the smoothed table, so the preview and the export always share it.
pub const DEFAULT_SMOOTHING: f64 = 18.0;

/// Two cursor samples less than this far apart (in video pixels) count as
/// "barely moved".
const DWELL_RADIUS: f64 = 12.0;

/// …and the gap in time below which that is still a dwell rather than a glide.
const DWELL_TIME: f64 = 0.25;

/// How long a click ripple is visible after the press.
pub const RIPPLE_SECONDS: f64 = 0.45;

/// One cursor position, in video pixels, at one time.
#[derive(Debug, Clone, Copy)]
pub struct CursorSample {
    /// Seconds since the recording started.
    pub t: f64,
    pub x: f64,
    pub y: f64,
}

/// A button press, in video pixels.
#[derive(Debug, Clone, Copy)]
pub struct Click {
    pub t: f64,
    pub x: f64,
    pub y: f64,
    pub button: MouseButtonName,
}

/// A click ripple in progress.
#[derive(Debug, Clone, Copy)]
pub struct Ripple {
    pub x: f64,
    pub y: f64,
    /// 0 at the press, 1 when the ripple has faded out.
    pub progress: f64,
}

/// A stretch of time the pointer spent in one place.
#[derive(Debug, Clone, Copy)]
pub struct Dwell {
    pub start_t: f64,
    pub end_t: f64,
    pub x: f64,
    pub y: f64,
}

/// The cursor trail for one take, resampled and smoothed.
#[derive(Debug, Clone, Default)]
pub struct CursorTrack {
    /// The raw samples, in video pixels, sorted by time.
    samples: Vec<CursorSample>,
    /// The spring-smoothed table, sampled at `SPRING_HZ`.
    smoothed: Vec<CursorSample>,
    clicks: Vec<Click>,
    width: f64,
    height: f64,
}
impl CursorTrack {
    /// Build a track from a session's events.
    ///
    /// `meta` and the video's own dimensions are what make the screen → pixel
    /// conversion possible; a take with no `capture.json` yields an empty track
    /// and the caller skips cursor work entirely.
    pub fn from_events(events: &[InputEvent], meta: &CaptureMeta, width: u32, height: u32) -> Self {
        Self::from_events_with(events, meta, width, height, DEFAULT_SMOOTHING)
    }

    /// Build a track with an explicit spring frequency.
    ///
    /// This is where the editor's smoothing control lands: a higher omega
    /// settles sooner and reads as snappier, a lower one lets the pointer
    /// glide. The value is baked into the smoothed table here, so every
    /// consumer — preview and export alike — inherits the same motion.
    pub fn from_events_with(
        events: &[InputEvent],
        meta: &CaptureMeta,
        width: u32,
        height: u32,
        smoothing: f64,
    ) -> Self {
        let mut samples: Vec<CursorSample> = Vec::new();
        let mut clicks: Vec<Click> = Vec::new();

        for event in events {
            let Some(point) = event.position() else {
                continue;
            };
            // `None` here means the pointer was on another monitor, or outside
            // a region take — either way it is not in this video.
            let Some((x, y)) = meta.to_video_point(point.x, point.y, width, height) else {
                continue;
            };
            let t = event.seconds();
            samples.push(CursorSample { t, x, y });

            if let crate::input::event::InputEventKind::MouseDown { button, .. } = &event.kind {
                clicks.push(Click {
                    t,
                    x,
                    y,
                    button: *button,
                });
            }
        }

        // The reader already sorts, but the planner may hand us a filtered
        // list; sorting here keeps the binary searches honest.
        samples.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
        clicks.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));

        let smoothed = smooth(&samples, smoothing);
        Self {
            samples,
            smoothed,
            clicks,
            width: width as f64,
            height: height as f64,
        }
    }

    /// True when there is nothing to draw — no samples survived the mapping.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn clicks(&self) -> &[Click] {
        &self.clicks
    }

    /// Time span covered, in seconds.
    pub fn duration(&self) -> f64 {
        self.samples.last().map(|s| s.t).unwrap_or(0.0)
    }

    /// Video width this track was built for.
    pub fn width(&self) -> f64 {
        self.width
    }

    pub fn height(&self) -> f64 {
        self.height
    }

    /// The raw (unsmoothed) position at `t`, holding the first sample before
    /// the trail starts and the last one after it ends.
    pub fn raw_at(&self, t: f64) -> Option<CursorSample> {
        if self.samples.is_empty() {
            return None;
        }
        let idx = self.samples.partition_point(|s| s.t <= t);
        Some(match idx {
            0 => self.samples[0],
            n if n >= self.samples.len() => *self.samples.last().unwrap(),
            n => {
                let a = self.samples[n - 1];
                let b = self.samples[n];
                let span = (b.t - a.t).max(f64::EPSILON);
                let f = ((t - a.t) / span).clamp(0.0, 1.0);
                CursorSample {
                    t,
                    x: lerp(a.x, b.x, f),
                    y: lerp(a.y, b.y, f),
                }
            }
        })
    }

    /// The spring-smoothed position at `t`, for drawing.
    pub fn position_at(&self, t: f64) -> Option<(f64, f64)> {
        if self.smoothed.is_empty() {
            return None;
        }
        if t <= self.smoothed[0].t {
            return Some((self.smoothed[0].x, self.smoothed[0].y));
        }
        let last = *self.smoothed.last().unwrap();
        if t >= last.t {
            return Some((last.x, last.y));
        }
        let idx = self.smoothed.partition_point(|s| s.t < t);
        let a = self.smoothed[idx - 1];
        let b = self.smoothed[idx];
        let span = (b.t - a.t).max(f64::EPSILON);
        let f = ((t - a.t) / span).clamp(0.0, 1.0);
        Some((lerp(a.x, b.x, f), lerp(a.y, b.y, f)))
    }

    /// The most recent click at or before `t`, if it is still rippling.
    pub fn ripple_at(&self, t: f64) -> Option<Ripple> {
        let click = self.clicks.iter().rev().find(|c| c.t <= t)?;
        let age = t - click.t;
        if !(0.0..=RIPPLE_SECONDS).contains(&age) {
            return None;
        }
        Some(Ripple {
            x: click.x,
            y: click.y,
            progress: age / RIPPLE_SECONDS,
        })
    }

    /// Spans where the pointer barely moved for longer than `min_seconds`.
    ///
    /// Returned in video pixels. The planner decides which of these are worth a
    /// zoom; it is the planner's job because a dwell inside a click's zoom
    /// window should not produce a second, competing zoom.
    pub fn dwells(&self, min_seconds: f64) -> Vec<Dwell> {
        let mut out: Vec<Dwell> = Vec::new();
        let mut anchor: Option<CursorSample> = None;
        // The last sample still counted as "resting". A run ends when the pointer
        // leaves, and it ends at where it *was* — billing the travel to the new
        // position as time spent resting would turn every glide across the screen
        // into one long dwell at the midpoint between the two places.
        let mut last_still: Option<CursorSample> = None;

        for sample in &self.samples {
            match anchor {
                None => {
                    anchor = Some(*sample);
                    last_still = Some(*sample);
                }
                Some(start) => {
                    let still = (sample.x - start.x).abs() <= DWELL_RADIUS
                        && (sample.y - start.y).abs() <= DWELL_RADIUS;
                    if still {
                        last_still = Some(*sample);
                    } else {
                        if let Some(end) = last_still {
                            push_dwell(&mut out, start, end, min_seconds);
                        }
                        anchor = Some(*sample);
                        last_still = Some(*sample);
                    }
                }
            }
        }
        if let (Some(start), Some(end)) = (anchor, last_still) {
            push_dwell(&mut out, start, end, min_seconds);
        }
        out
    }
}

fn lerp(a: f64, b: f64, f: f64) -> f64 {
    a + (b - a) * f
}

fn push_dwell(out: &mut Vec<Dwell>, start: CursorSample, end: CursorSample, min_seconds: f64) {
    // `DWELL_TIME` is the floor: two samples 20 ms apart in the same place are
    // one moment of stillness, not a pause worth zooming into.
    if end.t - start.t < min_seconds.max(DWELL_TIME) {
        return;
    }
    out.push(Dwell {
        start_t: start.t,
        end_t: end.t,
        x: (start.x + end.x) / 2.0,
        y: (start.y + end.y) / 2.0,
    });
}

/// Integrate a critically-damped spring along the trail, at a fixed rate.
///
/// The spring pulls the drawn cursor toward the true position: stiff when the
/// pointer is far away (so it keeps up with a flick) and soft when it is close
/// (so a hand tremor does not read as jitter). Critically damped means it
/// approaches the target without overshooting or ringing — a springy cursor in a
/// screencast looks like a bug.
fn smooth(samples: &[CursorSample], smoothing: f64) -> Vec<CursorSample> {
    if samples.is_empty() {
        return Vec::new();
    }

    let step = 1.0 / SPRING_HZ;
    let start = samples[0];
    let end_t = samples.last().unwrap().t;
    let count = (((end_t - start.t) / step).ceil().max(1.0) as usize) + 1;

    // Critically damped: stiffness = omega^2, damping = 2*omega. Omega comes
    // from the caller (the editor's smoothing control); a non-finite value
    // falls back to the default rather than poisoning the whole table.
    let omega = if smoothing.is_finite() {
        smoothing.clamp(4.0, 48.0)
    } else {
        DEFAULT_SMOOTHING
    };
    let stiffness = omega * omega;
    let damping = 2.0 * omega;

    let mut out = Vec::with_capacity(count);
    let mut x = start.x;
    let mut y = start.y;
    let mut vx = 0.0f64;
    let mut vy = 0.0f64;

    for i in 0..count {
        let t = start.t + i as f64 * step;
        let target = target_at(samples, t);

        // Semi-implicit Euler, sub-stepped so a coarse output frame rate cannot
        // make the spring explode.
        let mut remaining = step;
        while remaining > 0.0 {
            let h = remaining.min(1.0 / 240.0);
            vx += (stiffness * (target.0 - x) - damping * vx) * h;
            vy += (stiffness * (target.1 - y) - damping * vy) * h;
            x += vx * h;
            y += vy * h;
            remaining -= h;
        }

        out.push(CursorSample { t, x, y });
    }

    // Pin the tail to the last true position, so a clip that ends mid-motion
    // settles where the pointer actually was.
    if let (Some(last), Some(truth)) = (out.last_mut(), samples.last()) {
        last.x = truth.x;
        last.y = truth.y;
    }
    out
}

/// The position the spring is chasing at time `t`.
fn target_at(samples: &[CursorSample], t: f64) -> (f64, f64) {
    let idx = samples.partition_point(|s| s.t <= t);
    match idx {
        0 => (samples[0].x, samples[0].y),
        n if n >= samples.len() => {
            let last = samples.last().unwrap();
            (last.x, last.y)
        }
        n => {
            let a = samples[n - 1];
            let b = samples[n];
            let span = (b.t - a.t).max(f64::EPSILON);
            let f = ((t - a.t) / span).clamp(0.0, 1.0);
            (lerp(a.x, b.x, f), lerp(a.y, b.y, f))
        }
    }
}

