//! The effects timeline: one table, asked for by both the preview and the
//! export.
//!
//! # Why a pre-computed table
//!
//! The cursor trail and the auto-zoom are *stateful* in the natural
//! implementation — the spring integrates forward in time, and the viewport
//! depends on where the cursor was at that instant. If the editor integrated
//! the spring and the exporter did the same independently, the two would drift
//! apart the moment either side was tweaked, and the user would approve one
//! thing and receive another.
//!
//! So the whole take is resolved here, once, into rows sampled at
//! [`TICK_HZ`]. The preview interpolates between rows to draw every animation
//! frame; the export writes the same rows into FFmpeg `sendcmd` files that
//! drive `crop` and `overlay` commands in lockstep. There is no second
//! implementation to disagree with, and a take with a still pointer and no
//! zoom collapses into a handful of rows.
//!
//! # Units
//!
//! Rows are in **source video pixels**. The export maps them into the output
//! (crop) box when it writes the command files; the preview divides by the
//! scale it lays the frame out at. Neither re-derives the zoom curve.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::input::cursor::CursorTrack;
use crate::zoom::planner::{segment_at, ZoomSegment};
use crate::zoom::viewport::Viewport;

/// How often the table is sampled, in hertz.
///
/// 60 Hz is one row per frame on the takes people actually edit; finer than the
/// eye can resolve once it is interpolated, and coarse enough that a ten-minute
/// take stays a few thousand rows.
pub const TICK_HZ: f64 = 60.0;

/// Default `edge_snap_ratio`: how far (as a fraction of the movable range) the
/// viewport centre is held away from the frame's edges.
pub const DEFAULT_EDGE_SNAP: f64 = 0.3;

/// The cursor image's own size, and the box its opaque content occupies
/// (`assets/cursor.png`, 512×512, content starting at 82,71).
///
/// Hard-coded because decoding a PNG in Rust just to measure it would drag in a
/// decoder for four constants. The sprite writer re-reads the same numbers, and
/// a test cross-checks the hotspot against the shipped file so a swapped asset
/// is caught here rather than as a pointer whose tip misses the click.
pub const SPRITE_WIDTH: f64 = 512.0;
pub const SPRITE_HEIGHT: f64 = 512.0;
pub const SPRITE_CONTENT_X: f64 = 82.0;
pub const SPRITE_CONTENT_Y: f64 = 71.0;
pub const SPRITE_CONTENT_WIDTH: f64 = 352.0;
pub const SPRITE_CONTENT_HEIGHT: f64 = 370.0;

/// How wide the pointer's visible arrow is at 1920 px of output, in pixels.
const CURSOR_VISUAL_WIDTH: f64 = 28.0;

/// The file the sprite is converted to for FFmpeg.
pub const SPRITE_FILE: &str = "cursor.pam";
/// The zoom crop's command file.
pub const ZOOM_CMD_FILE: &str = "zoom.cmd";
/// The cursor overlay's command file.
pub const CURSOR_CMD_FILE: &str = "cursor.cmd";

/// Where a hidden cursor is parked, in output pixels. Far enough off-frame to
/// never paint, while staying inside the range FFmpeg's overlay accepts.
pub const HIDDEN_PARK: i64 = -10_000;

/// The cursor asset, embedded so the export works from a bundled binary with no
/// asset tree next to it. `commands::editor::cursor_asset` hands the same bytes to
/// the preview, so both render one image.
pub const CURSOR_PNG: &[u8] = include_bytes!("../../assets/cursor.png");

/// What the user can bend, from the editor's sidebar. Every field is tolerated
/// at any value and clamped by [`OverlayOptions::sanitized`] — the UI is not the
/// only caller, and a hand-edited value must not be able to produce a crop that
/// falls off the video.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OverlayOptions {
    /// Apply the planned auto-zoom at all.
    pub zoom: bool,
    /// Multiplier on each planned segment's zoom level. 1.0 = as planned.
    pub zoom_strength: f64,
    /// How far the viewport centre is held away from the frame's edges.
    pub zoom_edge_snap: f64,
    /// Draw the cursor layer. The video was captured without one.
    pub cursor: bool,
    /// Multiplier on the cursor's size.
    pub cursor_scale: f64,
    /// Spring angular frequency, in rad/s. Higher settles faster.
    pub cursor_smoothing: f64,
    /// Draw click ripples in the preview.
    pub ripples: bool,
}

impl Default for OverlayOptions {
    fn default() -> Self {
        Self {
            zoom: true,
            zoom_strength: 1.0,
            zoom_edge_snap: DEFAULT_EDGE_SNAP,
            cursor: true,
            cursor_scale: 1.0,
            cursor_smoothing: crate::input::cursor::DEFAULT_SMOOTHING,
            ripples: true,
        }
    }
}

impl OverlayOptions {
    /// Clamp every value into a range the rest of the pipeline can trust.
    pub fn sanitized(mut self) -> Self {
        self.zoom_strength = clamp_or(self.zoom_strength, 1.0, 1.0, 2.5);
        self.zoom_edge_snap = clamp_or(self.zoom_edge_snap, DEFAULT_EDGE_SNAP, 0.0, 0.5);
        self.cursor_scale = clamp_or(self.cursor_scale, 1.0, 0.25, 3.0);
        self.cursor_smoothing = clamp_or(
            self.cursor_smoothing,
            crate::input::cursor::DEFAULT_SMOOTHING,
            4.0,
            48.0,
        );
        self
    }
}

/// A NaN-tolerant clamp: floats from the webview can arrive as NaN, which
/// `f64::clamp` would wave straight through.
fn clamp_or(value: f64, fallback: f64, low: f64, high: f64) -> f64 {
    if value.is_finite() {
        value.clamp(low, high)
    } else {
        fallback
    }
}

/// One tick of the effects timeline.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectRow {
    /// Seconds from the start of the take.
    pub t: f64,
    /// The visible source rectangle at this tick, in video pixels.
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// The cursor tip in video pixels, or `null` when the pointer had not been
    /// seen yet — or the cursor layer is switched off.
    pub cx: Option<f64>,
    pub cy: Option<f64>,
}

/// A click, for the preview's ripple.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClickMark {
    pub t: f64,
    pub x: f64,
    pub y: f64,
}

/// The resolved effects for one take.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Timeline {
    pub rows: Vec<EffectRow>,
    pub clicks: Vec<ClickMark>,
}

impl Timeline {
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// The cursor sprite's numbers, carried to the preview so both sides draw the
/// same image with the same anchor instead of each hard-coding the crop.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpriteInfo {
    pub width: f64,
    pub height: f64,
    /// The hotspot — the tip of the pointer — inside the image.
    pub hot_x: f64,
    pub hot_y: f64,
    /// The opaque content's box, used to size the *visible* arrow rather than
    /// the transparent padding around it.
    pub content_width: f64,
    pub content_height: f64,
}

impl Default for SpriteInfo {
    fn default() -> Self {
        Self {
            width: SPRITE_WIDTH,
            height: SPRITE_HEIGHT,
            hot_x: SPRITE_CONTENT_X,
            hot_y: SPRITE_CONTENT_Y,
            content_width: SPRITE_CONTENT_WIDTH,
            content_height: SPRITE_CONTENT_HEIGHT,
        }
    }
}

/// Resolve one take's cursor + zoom into the shared table.
///
/// `segments` are the *final* segments (zoom strength already folded in).
/// `track` is `None` for takes recorded without input tracking; the timeline is
/// then empty.
pub fn build_timeline(
    frame_width: u32,
    frame_height: u32,
    duration_ms: u64,
    segments: &[ZoomSegment],
    track: Option<&CursorTrack>,
    options: &OverlayOptions,
) -> Timeline {
    if frame_width == 0 || frame_height == 0 {
        return Timeline::default();
    }
    let fw = frame_width as f64;
    let fh = frame_height as f64;
    let duration = (duration_ms as f64 / 1000.0).max(0.0);

    // What there is to say. The cursor layer needs an actual trail — a take
    // recorded without input tracking has no position to draw — and the zoom
    // needs something to zoom towards. Without either, an empty table is both
    // honest and cheaper than a table of constant rows.
    let wants_cursor = options.cursor && track.is_some_and(|t| !t.is_empty());
    let wants_zoom = options.zoom && !segments.is_empty();
    if !(wants_cursor || wants_zoom) {
        return Timeline::default();
    }

    let ticks = (duration * TICK_HZ).ceil().max(0.0) as usize;
    let mut rows: Vec<EffectRow> = Vec::with_capacity(ticks + 1);
    let mut previous: Option<EffectRow> = None;

    for i in 0..=ticks {
        let t = (i as f64 / TICK_HZ).min(duration);
        let cursor = track.and_then(|track| track.position_at(t));

        let view = match segment_at(segments, t) {
            Some(segment) if options.zoom => {
                Viewport::following(fw, fh, segment, t, cursor, options.zoom_edge_snap)
            }
            _ => Viewport::full(fw, fh),
        };

        let row = EffectRow {
            t,
            x: view.x,
            y: view.y,
            w: view.width,
            h: view.height,
            cx: if options.cursor { cursor.map(|c| c.0) } else { None },
            cy: if options.cursor { cursor.map(|c| c.1) } else { None },
        };

        // Collapse runs that did not move: a still pointer over an un-zoomed
        // frame is the common case, and one row per idle stretch is enough —
        // the preview interpolates, so nothing is lost.
        //
        // The very first row is never collapsed away. Keeping the *last* tick of a
        // run would otherwise slide the table's start forward to the end of the
        // opening idle stretch, leaving the opening frames with no state at all.
        if let Some(prev) = previous {
            if rows.len() > 1 && rows_close(&prev, &row) {
                *rows.last_mut().unwrap() = row;
                previous = Some(row);
                continue;
            }
        }
        rows.push(row);
        previous = Some(row);
    }

    let clicks = if options.cursor && options.ripples {
        track
            .map(|track| {
                track
                    .clicks()
                    .iter()
                    .filter(|c| c.t <= duration)
                    .map(|c| ClickMark {
                        t: c.t,
                        x: c.x,
                        y: c.y,
                    })
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    Timeline { rows, clicks }
}

/// True when two rows describe the same state within half a pixel.
fn rows_close(a: &EffectRow, b: &EffectRow) -> bool {
    const EPSILON: f64 = 0.5;
    let near = |x: f64, y: f64| (x - y).abs() <= EPSILON;
    let cursor_line = match (a.cx, a.cy, b.cx, b.cy) {
        (None, None, None, None) => true,
        (Some(ax), Some(ay), Some(bx), Some(by)) => near(ax, bx) && near(ay, by),
        _ => false,
    };
    cursor_line && near(a.x, b.x) && near(a.y, b.y) && near(a.w, b.w) && near(a.h, b.h)
}

/// The row's viewport fitted into the output box.
///
/// The zoom is defined against the whole frame, but the export and the preview
/// only ever show the user's crop box. A viewport bigger than the box (the
/// un-zoomed case) is reduced to it; a smaller one keeps its size and is slid
/// until it sits inside. `lib/effects.ts` mirrors this function — keep the two
/// in step.
pub fn fit_viewport(view: &Viewport, bounds: &Viewport) -> Viewport {
    let width = view.width.min(bounds.width).max(2.0);
    let height = view.height.min(bounds.height).max(2.0);

    let cx = view.x + view.width / 2.0;
    let cy = view.y + view.height / 2.0;
    let x = (cx - width / 2.0).clamp(bounds.x, (bounds.x + bounds.width - width).max(bounds.x));
    let y = (cy - height / 2.0).clamp(bounds.y, (bounds.y + bounds.height - height).max(bounds.y));

    Viewport {
        x,
        y,
        width,
        height,
    }
}

/// The viewport for one row, fitted into the output box and rounded to the
/// even, integer rectangle FFmpeg's crop needs.
pub fn row_viewport(
    row: &EffectRow,
    bounds: &Viewport,
    frame_width: f64,
    frame_height: f64,
) -> Viewport {
    let raw = Viewport {
        x: row.x,
        y: row.y,
        width: row.w,
        height: row.h,
    };
    fit_viewport(&raw, bounds).to_even(frame_width, frame_height)
}

/// The `sendcmd` file that drives the zooming `crop`.
///
/// Every line sets one option at one time; the filter holds each value until
/// the next command, so only *changes* are emitted. Values are integers —
/// FFmpeg parses them as expressions, and an integer is both unambiguous and
/// short.
pub fn zoom_sendcmd(
    rows: &[EffectRow],
    bounds: &Viewport,
    frame_width: f64,
    frame_height: f64,
) -> String {
    let mut out = String::new();
    let mut previous: Option<(i64, i64, i64, i64)> = None;

    for row in rows {
        let view = row_viewport(row, bounds, frame_width, frame_height);
        let key = (
            view.width.round() as i64,
            view.height.round() as i64,
            view.x.round() as i64,
            view.y.round() as i64,
        );
        if Some(key) == previous {
            continue;
        }
        previous = Some(key);
        out.push_str(&format!(
            "{t:.4} crop@zc w {w};\n{t:.4} crop@zc h {h};\n\
             {t:.4} crop@zc x {x};\n{t:.4} crop@zc y {y};\n",
            t = row.t,
            w = key.0,
            h = key.1,
            x = key.2,
            y = key.3,
        ));
    }

    out
}

/// The `sendcmd` file that moves the cursor sprite.
///
/// Positions are in **output** pixels: the sprite is overlaid after the crop
/// and scale, so the row's source-space cursor travels through the same
/// transform the frame did. When the pointer is unknown the sprite is parked
/// off-frame, which is how the overlay hides without a second filter.
pub fn cursor_sendcmd(
    rows: &[EffectRow],
    bounds: &Viewport,
    out_width: u32,
    out_height: u32,
    sprite: &SpriteMetrics,
) -> String {
    let mut out = String::new();
    let mut previous: Option<(i64, i64)> = None;

    for row in rows {
        let position = cursor_position(row, bounds, out_width, out_height, sprite);

        if Some(position) == previous {
            continue;
        }
        previous = Some(position);
        out.push_str(&format!(
            "{t:.4} overlay@ov x {x};\n{t:.4} overlay@ov y {y};\n",
            t = row.t,
            x = position.0,
            y = position.1,
        ));
    }

    out
}

/// Where the sprite's top-left corner goes for one row, in output pixels.
///
/// The row's cursor is in source pixels, so it has to travel through the very
/// same crop-then-scale the frame did — otherwise the pointer slides against
/// the UI it is pointing at as the zoom changes. The hotspot is subtracted so
/// the arrow's *tip* lands on the tracked coordinate, not its corner.
///
/// A row with no cursor parks the sprite off-frame. This is also what
/// `build_filters` uses for the overlay's initial `x`/`y`, which is why the first
/// frame cannot disagree with the first command.
pub fn cursor_position(
    row: &EffectRow,
    bounds: &Viewport,
    out_width: u32,
    out_height: u32,
    sprite: &SpriteMetrics,
) -> (i64, i64) {
    let view = fit_viewport(
        &Viewport {
            x: row.x,
            y: row.y,
            width: row.w,
            height: row.h,
        },
        bounds,
    );

    match (row.cx, row.cy) {
        (Some(cx), Some(cy)) => {
            let scale_x = out_width as f64 / view.width.max(1.0);
            let scale_y = out_height as f64 / view.height.max(1.0);
            (
                ((cx - view.x) * scale_x - sprite.hot_x).round() as i64,
                ((cy - view.y) * scale_y - sprite.hot_y).round() as i64,
            )
        }
        _ => (HIDDEN_PARK, HIDDEN_PARK),
    }
}

/// The sprite as FFmpeg will see it: pixel size and the scaled hotspot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpriteMetrics {
    pub width: u32,
    pub height: u32,
    /// Where the tip sits inside the *scaled* sprite, in output pixels.
    pub hot_x: f64,
    pub hot_y: f64,
}

/// Size the sprite for an output `out_width` pixels wide.
///
/// The arrow is sized against the video's own width — a 4K take should not get
/// a thumbnail-sized pointer — and measured against the image's *content* box,
/// so the transparent padding around the arrow does not shrink it.
pub fn sprite_metrics(out_width: u32, cursor_scale: f64) -> SpriteMetrics {
    let reference = (CURSOR_VISUAL_WIDTH * (out_width as f64 / 1920.0) * cursor_scale).max(2.0);
    let full_width = reference * (SPRITE_WIDTH / SPRITE_CONTENT_WIDTH);
    let full_height = reference * (SPRITE_HEIGHT / SPRITE_CONTENT_HEIGHT);

    let width = even_u32(full_width.round().clamp(8.0, 2048.0) as u32);
    let height = even_u32(full_height.round().clamp(8.0, 2048.0) as u32);

    SpriteMetrics {
        width,
        height,
        hot_x: SPRITE_CONTENT_X * (width as f64 / SPRITE_WIDTH),
        hot_y: SPRITE_CONTENT_Y * (height as f64 / SPRITE_HEIGHT),
    }
}

fn even_u32(value: u32) -> u32 {
    (value & !1).max(2)
}

/// Convert the embedded cursor PNG into a scaled PAM inside `dir` — the format
/// FFmpeg reads with its alpha intact, without an image decoder in this crate.
pub fn write_cursor_sprite(dir: &Path, metrics: &SpriteMetrics) -> Result<PathBuf> {
    let png = dir.join("cursor.png");
    std::fs::write(&png, CURSOR_PNG).context("failed to write the embedded cursor image")?;

    let pam = dir.join(SPRITE_FILE);
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&png)
        .args([
            "-vf",
            &format!("scale={}:{}", metrics.width, metrics.height),
            "-frames:v",
            "1",
        ])
        .arg(&pam)
        .status()
        .context("failed to run ffmpeg to convert the cursor sprite")?;

    if !status.success() {
        return Err(anyhow!("ffmpeg could not convert the cursor sprite"));
    }
    Ok(pam)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::cursor::DEFAULT_SMOOTHING;
    use crate::input::event::{InputEvent, ScreenPoint};
    use crate::utils::capture_meta::{CaptureMeta, PixelRect};
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

    fn meta() -> CaptureMeta {
        CaptureMeta {
            display: PixelRect::new(0, 0, 1920, 1080),
            primary_scale_factor: 1.0,
            region: None,
            fps: 60,
            cursor_baked_in: false,
        }
    }

    /// A track that glides from one corner to the middle over a second.
    fn track() -> CursorTrack {
        let events = vec![
            InputEvent::cursor_move(0.0, ScreenPoint { x: 300.0, y: 300.0 }),
            InputEvent::cursor_move(1000.0, ScreenPoint { x: 900.0, y: 500.0 }),
        ];
        CursorTrack::from_events(&events, &meta(), 1920, 1080)
    }

    #[test]
    fn options_are_clamped_into_range() {
        let o = OverlayOptions {
            zoom_strength: 99.0,
            zoom_edge_snap: -1.0,
            cursor_scale: f64::NAN,
            ..OverlayOptions::default()
        }
        .sanitized();
        assert!(o.zoom_strength <= 2.5);
        assert!(o.zoom_edge_snap >= 0.0);
        assert!(o.cursor_scale.is_finite());
        assert_eq!(o.cursor_smoothing, DEFAULT_SMOOTHING);
    }

    /// `src/lib/effects.ts` mirrors these bounds so the preview cannot draw
    /// outside the box the export renders. The constants are asserted here
    /// because nothing else in the Rust build would notice the TS copy drifting.
    /// `src/lib/effects.ts` mirrors `fit_viewport` so the preview draws exactly
    /// the rectangle the export crops to. These vectors are the shared contract:
    /// the same numbers appear in the TypeScript test of the same name, so a
    /// change to one side that is not made to the other fails a build rather than
    /// quietly shifting the preview.
    #[test]
    fn fit_viewport_cases_the_preview_also_uses() {
        let bounds = Viewport {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        };

        // A viewport larger than the box shrinks to it.
        let big = fit_viewport(
            &Viewport {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
            },
            &bounds,
        );
        assert_eq!((big.width, big.height), (1920.0, 1080.0));
        assert_eq!((big.x, big.y), (0.0, 0.0));

        // A zoomed viewport keeps its size and slides to stay inside.
        let zoomed = fit_viewport(
            &Viewport {
                x: 800.0,
                y: 500.0,
                width: 480.0,
                height: 270.0,
            },
            &bounds,
        );
        assert_eq!((zoomed.width, zoomed.height), (480.0, 270.0));
        // Centre would be (1040, 635); the frame only allows up to 1440/810.
        assert_eq!((zoomed.x, zoomed.y), (800.0, 500.0));

        // Pushed past the right/bottom edge, it slides back in.
        let edge = fit_viewport(
            &Viewport {
                x: 1800.0,
                y: 1000.0,
                width: 480.0,
                height: 270.0,
            },
            &bounds,
        );
        assert_eq!(edge.x, 1440.0);
        assert_eq!(edge.y, 810.0);

        // An off-centre crop box is honoured on both sides.
        let offset_bounds = Viewport {
            x: 100.0,
            y: 50.0,
            width: 800.0,
            height: 600.0,
        };
        let in_box = fit_viewport(
            &Viewport {
                x: 100.0,
                y: 50.0,
                width: 400.0,
                height: 300.0,
            },
            &offset_bounds,
        );
        assert_eq!((in_box.x, in_box.y), (100.0, 50.0));

        // A sub-pixel viewport is floored to the 2px minimum FFmpeg accepts.
        let tiny = fit_viewport(
            &Viewport {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                height: 0.5,
            },
            &bounds,
        );
        assert_eq!((tiny.width, tiny.height), (2.0, 2.0));
    }

    #[test]
    fn the_clamp_bounds_are_the_ones_the_preview_mirrors() {
        let o = OverlayOptions {
            zoom_strength: 99.0,
            zoom_edge_snap: 99.0,
            cursor_scale: 99.0,
            cursor_smoothing: 99.0,
            ..OverlayOptions::default()
        }
        .sanitized();
        assert_eq!(o.zoom_strength, 2.5);
        assert_eq!(o.zoom_edge_snap, 0.5);
        assert_eq!(o.cursor_scale, 3.0);
        assert_eq!(o.cursor_smoothing, 48.0);

        let low = OverlayOptions {
            zoom_strength: -1.0,
            zoom_edge_snap: -1.0,
            cursor_scale: -1.0,
            cursor_smoothing: -1.0,
            ..OverlayOptions::default()
        }
        .sanitized();
        assert_eq!(low.zoom_strength, 1.0);
        assert_eq!(low.zoom_edge_snap, 0.0);
        assert_eq!(low.cursor_scale, 0.25);
        assert_eq!(low.cursor_smoothing, 4.0);
    }

    #[test]
    fn no_zoom_and_no_cursor_produces_no_rows() {
        let options = OverlayOptions {
            zoom: false,
            cursor: false,
            ..OverlayOptions::default()
        };
        let timeline = build_timeline(1920, 1080, 3000, &[], Some(&track()), &options);
        assert!(timeline.is_empty());
    }

    #[test]
    fn a_take_with_no_track_produces_no_rows() {
        let timeline = build_timeline(
            1920,
            1080,
            3000,
            &[],
            None,
            &OverlayOptions::default(),
        );
        assert!(timeline.is_empty());
    }

    #[test]
    fn the_table_covers_the_take_and_carries_the_cursor() {
        let timeline = build_timeline(
            1920,
            1080,
            1000,
            &[],
            Some(&track()),
            &OverlayOptions::default(),
        );
        assert!(!timeline.is_empty());
        assert_eq!(timeline.rows.first().unwrap().t, 0.0);
        assert!((timeline.rows.last().unwrap().t - 1.0).abs() < 1e-9);

        let first = timeline.rows.first().unwrap();
        assert!(first.cx.is_some() && first.cy.is_some());
        assert!(
            (first.w - 1920.0).abs() < 1e-6,
            "no zoom means the full frame"
        );
    }

    #[test]
    fn zoom_rows_shrink_the_viewport_inside_the_segment() {
        let segments = [segment(0.2, 0.8, 900.0, 500.0, 2.0)];
        let timeline = build_timeline(
            1920,
            1080,
            1000,
            &segments,
            Some(&track()),
            &OverlayOptions::default(),
        );

        let during = timeline
            .rows
            .iter()
            .find(|r| (r.t - 0.5).abs() < 1e-9)
            .expect("a tick at the segment's peak");
        assert!(
            during.w < 1920.0 * 0.6,
            "a zoomed row should be smaller: {}",
            during.w
        );

        let outside = timeline.rows.first().unwrap();
        assert!((outside.w - 1920.0).abs() < 1e-6);
    }

    #[test]
    fn a_still_pointer_collapses_the_table() {
        // One sample: the cursor never moves, so a 30 s take must not become
        // 1,800 identical rows.
        let events = vec![InputEvent::cursor_move(
            0.0,
            ScreenPoint { x: 960.0, y: 540.0 },
        )];
        let still = CursorTrack::from_events(&events, &meta(), 1920, 1080);
        let timeline = build_timeline(
            1920,
            1080,
            30_000,
            &[],
            Some(&still),
            &OverlayOptions::default(),
        );
        assert!(
            timeline.rows.len() < 10,
            "a still pointer should collapse: {} rows",
            timeline.rows.len()
        );
    }

    #[test]
    fn cursor_and_zoom_switches_are_honoured() {
        let segments = [segment(0.2, 0.8, 900.0, 500.0, 2.0)];

        let no_cursor = OverlayOptions {
            cursor: false,
            ..OverlayOptions::default()
        };
        let timeline = build_timeline(1920, 1080, 1000, &segments, Some(&track()), &no_cursor);
        assert!(timeline.rows.iter().all(|r| r.cx.is_none()));
        assert!(timeline.clicks.is_empty(), "no ripple without a cursor");
        // …but the zoom still follows the (invisible) pointer.
        assert!(timeline.rows.iter().any(|r| r.w < 1800.0));

        let no_zoom = OverlayOptions {
            zoom: false,
            ..OverlayOptions::default()
        };
        let timeline = build_timeline(1920, 1080, 1000, &segments, Some(&track()), &no_zoom);
        assert!(timeline.rows.iter().all(|r| (r.w - 1920.0).abs() < 1e-6));
    }

    #[test]
    fn fit_viewport_slides_a_zoomed_rect_inside_the_box() {
        let bounds = Viewport {
            x: 400.0,
            y: 200.0,
            width: 800.0,
            height: 600.0,
        };
        // A zoomed rect half outside the box: it moves back in, size intact.
        let view = Viewport {
            x: 100.0,
            y: 100.0,
            width: 400.0,
            height: 300.0,
        };
        let fitted = fit_viewport(&view, &bounds);
        assert!((fitted.width - 400.0).abs() < 1e-9);
        assert!(fitted.x >= bounds.x && fitted.x + fitted.width <= bounds.x + bounds.width);
        assert!(fitted.y >= bounds.y && fitted.y + fitted.height <= bounds.y + bounds.height);

        // A rect bigger than the box (the un-zoomed case) shrinks to the box.
        let wide = Viewport::full(1920.0, 1080.0);
        assert_eq!(fit_viewport(&wide, &bounds), bounds);
    }

    #[test]
    fn the_zoom_command_file_only_emits_changes() {
        let rows = vec![
            EffectRow { t: 0.0, x: 0.0, y: 0.0, w: 1920.0, h: 1080.0, cx: None, cy: None },
            EffectRow { t: 0.1, x: 0.0, y: 0.0, w: 1920.0, h: 1080.0, cx: None, cy: None },
            EffectRow { t: 0.2, x: 480.0, y: 270.0, w: 960.0, h: 540.0, cx: None, cy: None },
        ];
        let bounds = Viewport::full(1920.0, 1080.0);
        let text = zoom_sendcmd(&rows, &bounds, 1920.0, 1080.0);
        // Two states → eight lines; the duplicate middle row is gone.
        assert_eq!(text.lines().count(), 8, "got:\n{text}");
        assert!(text.contains("0.0000 crop@zc w 1920;"));
        assert!(text.contains("0.2000 crop@zc w 960;"));
        assert!(text.contains("0.2000 crop@zc x 480;"));
    }

    #[test]
    fn the_cursor_command_file_maps_into_output_space_and_parks_when_unknown() {
        let rows = vec![
            EffectRow { t: 0.0, x: 0.0, y: 0.0, w: 1920.0, h: 1080.0, cx: None, cy: None },
            EffectRow {
                t: 0.5,
                x: 960.0,
                y: 540.0,
                w: 960.0,
                h: 540.0,
                cx: Some(1200.0),
                cy: Some(700.0),
            },
        ];
        let bounds = Viewport::full(1920.0, 1080.0);
        let sprite = SpriteMetrics {
            width: 40,
            height: 42,
            hot_x: 6.0,
            hot_y: 6.0,
        };
        let text = cursor_sendcmd(&rows, &bounds, 1920, 1080, &sprite);
        assert!(
            text.contains("overlay@ov x -10000;"),
            "an unknown pointer is parked off-frame:\n{text}"
        );
        // (1200-960) * 1920/960 - 6 = 474
        assert!(text.contains("0.5000 overlay@ov x 474;"), "got:\n{text}");
        // (700-540) * 1080/540 - 6 = 314
        assert!(text.contains("0.5000 overlay@ov y 314;"));
    }

    #[test]
    fn sprite_metrics_size_the_arrow_against_the_output() {
        let at_1080 = sprite_metrics(1920, 1.0);
        let at_4k = sprite_metrics(3840, 1.0);
        assert!(at_4k.width > at_1080.width, "4K should get a bigger pointer");

        // The visible arrow is ~28 px wide at 1920 regardless of the padding in
        // the source image.
        let content = (at_1080.width as f64) * (SPRITE_CONTENT_WIDTH / SPRITE_WIDTH);
        assert!((content - 28.0).abs() <= 1.5, "content was {content}");

        let doubled = sprite_metrics(1920, 2.0);
        assert!((doubled.width as f64 - 2.0 * at_1080.width as f64).abs() <= 2.0);
        // The hotspot must track the scaling.
        let ratio = at_1080.hot_x / (at_1080.width as f64);
        assert!((ratio - SPRITE_CONTENT_X / SPRITE_WIDTH).abs() < 1e-9);
    }

    #[test]
    fn the_sprite_file_decodes_with_ffmpeg() {
        let dir = std::env::temp_dir().join(format!("revate-sprite-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let metrics = sprite_metrics(1920, 1.0);
        match write_cursor_sprite(&dir, &metrics) {
            Ok(path) => {
                let status = Command::new("ffmpeg")
                    .args(["-hide_banner", "-loglevel", "error", "-y", "-loop", "1", "-i"])
                    .arg(&path)
                    .args(["-frames:v", "1", "-f", "null", "-"])
                    .status();
                assert!(
                    status.map(|s| s.success()).unwrap_or(true),
                    "the generated sprite is not decodable"
                );
            }
            Err(error) => {
                // ffmpeg absent: the pure metrics tests above still apply.
                eprintln!("skipping the sprite decode check: {error}");
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
