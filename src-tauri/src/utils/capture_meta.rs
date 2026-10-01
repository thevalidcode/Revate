//! What was on screen when the take was recorded — written to `capture.json`.
//!
//! Cursor coordinates arrive in **screen space**; the video is in **its own
//! pixels**, and the two are only trivially related when the recording covers a
//! whole monitor. Region recordings crop, retina displays scale, and a secondary
//! monitor sits at a non-zero origin. This file records the pieces needed to
//! convert between them exactly, and is written at record time (when we still
//! know them) and read back at export time.
//!
//! The conversion itself is [`CaptureMeta::to_video_point`], which is
//! deliberately the *only* place that knows about the space mismatch.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// The file a session's capture geometry lives in.
pub const CAPTURE_FILE: &str = "capture.json";

/// A rectangle in physical screen pixels, top-left origin.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PixelRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl PixelRect {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// The geometry of one recorded take.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureMeta {
    /// The captured monitor, in physical pixels.
    pub display: PixelRect,
    /**
     * Scale factor of the *primary* monitor.
     *
     * Cursor events arrive in logical points whose global origin is the primary
     * monitor's top-left corner, so this is the factor that turns them back into
     * the single physical space `display` also lives in. It is deliberately not
     * the captured monitor's own factor: on a mixed-DPI desk those differ, and
     * using the wrong one shifts every cursor position.
     */
    pub primary_scale_factor: f64,
    /// The sub-rectangle of `display` that was recorded, if any.
    pub region: Option<PixelRect>,
    /// Capture frame rate.
    pub fps: u32,
    /**
     * True when FFmpeg drew the system cursor into the frames.
     *
     * When the input tracker is running we ask FFmpeg to hide the cursor and
     * draw our own, so the two never appear on top of each other. When the
     * tracker could not start (no Accessibility permission) the system cursor
     * stays, and the editor must not draw a second one over it.
     */
    pub cursor_baked_in: bool,
}

impl CaptureMeta {
    /// The rectangle the video actually covers, in physical pixels: the whole
    /// monitor, or the recorded region within it.
    pub fn source_rect(&self) -> PixelRect {
        self.region.unwrap_or(self.display)
    }

    /**
     * Map a cursor position from logical screen points into video pixels.
     *
     * Returns `None` when the point falls outside the recorded area, which is
     * the common case on a multi-monitor desk — the caller uses that to decide
     * whether to draw the cursor at all.
     *
     * Both bounds are inclusive: a point sitting exactly on the far edge of the
     * recorded rectangle is the last cursor position still on screen, so it is
     * mapped rather than discarded. The result may therefore land exactly on
     * `video_w`/`video_h`, which the renderer clips like any other draw that
     * falls outside the frame.
     */
    pub fn to_video_point(&self, x: f64, y: f64, video_w: u32, video_h: u32) -> Option<(f64, f64)> {
        let source = self.source_rect();
        if source.width == 0 || source.height == 0 || video_w == 0 || video_h == 0 {
            return None;
        }

        // Points → physical pixels, through the primary monitor's scale.
        let px = x * self.primary_scale_factor;
        let py = y * self.primary_scale_factor;

        // Physical screen → the recorded rectangle → the video's own pixels.
        let vx = (px - source.x as f64) * (video_w as f64 / source.width as f64);
        let vy = (py - source.y as f64) * (video_h as f64 / source.height as f64);

        let inside = vx >= 0.0 && vy >= 0.0 && vx <= video_w as f64 && vy <= video_h as f64;
        inside.then_some((vx, vy))
    }
}

/// Write `capture.json` into a session folder.
pub fn write_capture_meta(dir: &Path, meta: &CaptureMeta) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let path = dir.join(CAPTURE_FILE);
    let json = serde_json::to_string_pretty(meta)?;
    std::fs::write(&path, json).with_context(|| format!("failed to write {}", path.display()))
}

/// Read `capture.json`, or `None` for a take recorded before it existed (or with
/// input tracking unavailable). Callers treat `None` as "no cursor, no zoom".
pub fn read_capture_meta(dir: &Path) -> Option<CaptureMeta> {
    let bytes = std::fs::read(dir.join(CAPTURE_FILE)).ok()?;
    // A truncated or hand-edited file should degrade to "no metadata", not fail
    // the whole export.
    serde_json::from_slice(&bytes).ok()
}


#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> CaptureMeta {
        CaptureMeta {
            display: PixelRect::new(0, 0, 2560, 1440),
            primary_scale_factor: 2.0,
            region: None,
            fps: 30,
            cursor_baked_in: false,
        }
    }

    #[test]
    fn maps_logical_points_into_video_pixels() {
        let meta = meta();
        // 1280×720 is the centre of a 2560×1440 physical screen at 2x.
        let (x, y) = meta.to_video_point(640.0, 360.0, 1920, 1080).unwrap();
        assert!((x - 960.0).abs() < 0.001, "x was {x}");
        assert!((y - 540.0).abs() < 0.001, "y was {y}");
    }

    #[test]
    fn points_off_the_captured_monitor_are_rejected() {
        let meta = meta();
        assert!(meta.to_video_point(-50.0, 100.0, 1920, 1080).is_none());
        assert!(meta.to_video_point(2000.0, 100.0, 1920, 1080).is_none());
    }

    #[test]
    fn a_region_recording_maps_relative_to_the_region() {
        let mut meta = meta();
        // Top-left quarter of the monitor.
        meta.region = Some(PixelRect::new(0, 0, 1280, 720));
        // The region's own top-left is video (0,0)…
        let (x, y) = meta.to_video_point(0.0, 0.0, 1280, 720).unwrap();
        assert!(x.abs() < 0.001 && y.abs() < 0.001);
        // …and its bottom-right is the far corner.
        let (x, y) = meta.to_video_point(640.0, 360.0, 1280, 720).unwrap();
        assert!((x - 1280.0).abs() < 0.5, "x was {x}");
        assert!((y - 720.0).abs() < 0.5, "y was {y}");
        // Anything outside the region is not in the video.
        assert!(meta.to_video_point(700.0, 100.0, 1280, 720).is_none());
    }

    #[test]
    fn a_monitor_at_a_non_zero_origin_is_handled() {
        let mut meta = meta();
        meta.display = PixelRect::new(2560, 0, 1920, 1080);
        meta.primary_scale_factor = 1.0;
        // Cursor at the top-left of the *second* monitor.
        let (x, y) = meta.to_video_point(2560.0, 0.0, 1920, 1080).unwrap();
        assert!(x.abs() < 0.001 && y.abs() < 0.001);
        // A point on the first monitor is outside this take.
        assert!(meta.to_video_point(100.0, 100.0, 1920, 1080).is_none());
    }

    #[test]
    fn degenerate_rectangles_never_divide_by_zero() {
        let mut meta = meta();
        meta.display = PixelRect::new(0, 0, 0, 0);
        assert!(meta.to_video_point(1.0, 1.0, 1920, 1080).is_none());
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("revate-capture-{}", std::process::id()));
        let meta = meta();
        write_capture_meta(&dir, &meta).unwrap();
        assert_eq!(read_capture_meta(&dir).unwrap().display, meta.display);
        std::fs::remove_dir_all(&dir).ok();

        assert!(read_capture_meta(Path::new("/definitely/not/here")).is_none());
    }
}
