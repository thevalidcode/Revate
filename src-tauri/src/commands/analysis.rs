//! Reading a finished take's input trail back and turning it into the two
//! things the editor and the exporter need: a list of zoom segments, and a
//! cursor track sampled in video-pixel space.
//!
//! # The whole pipeline in one place
//!
//! ```text
//! events.revents ─┐
//! capture.json   ─┼─▶ CursorTrack ─▶ ZoomSegment[]   (auto-zoom)
//! ffprobe        ─┘            └──▶ sampled positions (cursor overlay)
//! ```
//!
//! Everything the editor shows about a take comes from [`analyze`], so the
//! preview and the export cannot disagree about where a zoom is or where the
//! cursor was: they are handed the *same* objects.
//!
//! # Degrading
//!
//! A take recorded before input tracking existed, or without Accessibility
//! permission, has no `events.revents` or no `capture.json`. That is not an
//! error — it means "no cursor work, no zoom", and [`analyze`] returns empty
//! segments with `has_cursor_trail == false` so the UI can say so plainly
//! instead of silently doing nothing.

use std::path::Path;

use serde::Serialize;
use tauri::AppHandle;

use crate::commands::editor::{session_dir, video_in};
use crate::events::reader::read_session_events;
use crate::input::cursor::CursorTrack;
use crate::utils::capture_meta::read_capture_meta;
use crate::utils::ffprobe;
use crate::zoom::planner::{plan_zoom_segments, ZoomReason, ZoomSegment};

/// Why a zoom segment exists, in a form the UI can switch on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SegmentReason {
    /// The user clicked here.
    Click,
    /// The pointer rested here.
    Dwell,
}

impl From<&ZoomSegment> for SegmentReason {
    fn from(segment: &ZoomSegment) -> Self {
        match segment.reason {
            ZoomReason::Click => Self::Click,
            ZoomReason::Dwell => Self::Dwell,
        }
    }
}

/// One planned zoom, flattened for the editor timeline.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentInfo {
    pub start_t: f64,
    pub end_t: f64,
    /// Focus point in video pixels.
    pub x: f64,
    pub y: f64,
    pub zoom_level: f64,
    pub reason: SegmentReason,
}

/// Everything the editor needs to draw the auto-zoom overlay and decide whether
/// the cursor layer is worth compositing.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionAnalysis {
    /// False when the take has no usable trail: no `events.revents`, no
    /// `capture.json`, or input tracking was refused at record time.
    pub has_cursor_trail: bool,
    /// True when FFmpeg burned the system cursor into the video, which means
    /// drawing our own would double it up.
    pub cursor_baked_in: bool,
    /// Events read back from `events.revents`.
    pub event_count: usize,
    /// Lines that could not be parsed.
    pub skipped: usize,
    /// Video dimensions, in pixels.
    pub width: u32,
    pub height: u32,
    pub duration_ms: u64,
    /// Clicks found in the trail — the raw material for the zoom segments.
    pub click_count: usize,
    /// Planned, merged, non-overlapping zoom segments, in time order.
    pub zoom_segments: Vec<SegmentInfo>,
}

/// Analyze a session folder from disk.
///
/// Falls back to a graceful "no cursor work" result rather than erroring when
/// the trail or the geometry sidecar is missing.
pub fn analyze(dir: &Path) -> anyhow::Result<SessionAnalysis> {
    let video = video_in(dir).map_err(anyhow::Error::msg)?;
    let info = ffprobe::probe(&video)?;
    let meta = read_capture_meta(dir);

    let log = read_session_events(dir);
    let event_count = log.events.len();

    // Without the geometry sidecar there is no honest way to map screen
    // coordinates into this video's pixels, so the trail is not usable.
    let track = meta.as_ref().map(|meta| {
        CursorTrack::from_events(&log.events, meta, info.width, info.height)
    });
    let click_count = track.as_ref().map(|t| t.clicks().len()).unwrap_or(0);

    // A track that lost every sample to the mapping (pointer on another monitor,
    // or a region take) is not a trail.
    let has_cursor_trail = track.as_ref().is_some_and(|t| !t.is_empty());

    let zoom_segments = match (&track, meta.as_ref()) {
        (Some(track), Some(_)) => plan_zoom_segments(&log.events, track),
        // No metadata means no frame to clamp the crop to; planning against a
        // guessed frame would produce segments that fall off the video.
        _ => Vec::new(),
    }
    .into_iter()
    .map(|s| SegmentInfo {
        start_t: s.start_t,
        end_t: s.end_t,
        x: s.target_x,
        y: s.target_y,
        zoom_level: s.zoom_level,
        reason: SegmentReason::from(&s),
    })
    .collect();

    Ok(SessionAnalysis {
        has_cursor_trail,
        cursor_baked_in: meta.as_ref().is_none_or(|m| m.cursor_baked_in),
        event_count,
        skipped: log.skipped,
        width: info.width,
        height: info.height,
        duration_ms: info.duration_ms,
        click_count,
        zoom_segments,
    })
}

/// Frontend entry point for [`analyze`].
#[tauri::command]
pub async fn session_analysis(app: AppHandle, session_id: String) -> Result<SessionAnalysis, String> {
    let dir = session_dir(&app, &session_id)?;
    analyze(&dir).map_err(|e| e.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::schema::{EventKind, MouseButtonName, RawEvent};
    use crate::input::cursor::CursorTrack;
    use crate::utils::capture_meta::{write_capture_meta, CaptureMeta, PixelRect};

    /// The metadata the recorder writes for a full-screen 1920×1080 at 1×.
    fn meta() -> CaptureMeta {
        CaptureMeta {
            display: PixelRect::new(0, 0, 1920, 1080),
            primary_scale_factor: 1.0,
            region: None,
            fps: 30,
            cursor_baked_in: false,
        }
    }

    /// Round-trip wire events through the reader, the way a real take does.
    fn log_of(lines: &[&str]) -> crate::events::reader::EventLog {
        let log = crate::events::reader::parse_lines(&lines.join("\n"));
        assert_eq!(log.skipped, 0, "test fixture should be fully parseable");
        log
    }

    #[test]
    fn cursor_samples_survive_the_round_trip_through_the_track() {
        let raw = [
            RawEvent::cursor_move(0.0, 900.0, 500.0),
            RawEvent::button(
                1000.0,
                EventKind::MouseDown,
                MouseButtonName::Left,
                900.0,
                500.0,
            ),
        ];
        let lines: Vec<String> = raw.iter().map(|e| serde_json::to_string(e).unwrap()).collect();
        let log = crate::events::reader::parse_lines(&lines.join("\n"));
        assert_eq!(log.events.len(), 2);

        let track = CursorTrack::from_events(&log.events, &meta(), 1920, 1080);
        assert!(!track.is_empty(), "the trail should survive mapping");
        assert_eq!(track.clicks().len(), 1);
        assert_eq!(track.width(), 1920.0);
    }

    #[test]
    fn a_click_on_its_own_produces_exactly_one_segment() {
        let log = log_of(&[
            r#"{"t":0.0,"type":"cursor_move","data":{"x":900.0,"y":500.0}}"#,
            r#"{"t":1000.0,"type":"mouse_down","data":{"x":900.0,"y":500.0,"button":"left"}}"#,
        ]);
        let track = CursorTrack::from_events(&log.events, &meta(), 1920, 1080);

        let plan = plan_zoom_segments(&log.events, &track);
        assert_eq!(plan.len(), 1);
        // `plan_zoom_segments` yields the planner's own `ZoomReason`; the
        // editor-facing `SegmentReason` is derived from it.
        assert!(matches!(plan[0].reason, ZoomReason::Click));
        assert_eq!(SegmentReason::from(&plan[0]), SegmentReason::Click);
        assert!((plan[0].start_t - 1.0).abs() < 1e-6, "start was {}", plan[0].start_t);
        assert!((plan[0].target_x - 900.0).abs() < 1e-6);
    }

    #[test]
    fn a_key_event_alone_never_produces_a_zoom() {
        // Keys carry no position, so a take that is only typing has nothing to
        // aim a zoom at.
        let log = log_of(&[r#"{"t":5.0,"type":"key_down","data":{"key":"char:R"}}"#]);
        let track = CursorTrack::from_events(&log.events, &meta(), 1920, 1080);
        assert!(track.is_empty());
        assert!(plan_zoom_segments(&log.events, &track).is_empty(), "typing must not zoom");
    }

    #[test]
    fn events_outside_the_recorded_area_are_dropped() {
        // Pointer at x=5000 is on another monitor; it must not enter the track.
        let log = log_of(&[r#"{"t":0.0,"type":"cursor_move","data":{"x":5000.0,"y":500.0}}"#]);
        let track = CursorTrack::from_events(&log.events, &meta(), 1920, 1080);
        assert!(track.is_empty());
    }

    #[test]
    fn an_empty_trail_plans_nothing() {
        let track = CursorTrack::from_events(&[], &meta(), 1920, 1080);
        assert!(plan_zoom_segments(&[], &track).is_empty());
    }

    /// The analysis struct is what the UI switches on, so its serde shape is part
    /// of the contract: camelCase, and `zoomSegments` present even when empty.
    #[test]
    fn analysis_serializes_to_the_shape_the_editor_expects() {
        let a = SessionAnalysis {
            has_cursor_trail: true,
            cursor_baked_in: false,
            event_count: 12,
            skipped: 0,
            width: 1920,
            height: 1080,
            duration_ms: 4200,
            click_count: 2,
            zoom_segments: vec![SegmentInfo {
                start_t: 1.0,
                end_t: 2.5,
                x: 640.0,
                y: 360.0,
                zoom_level: 1.8,
                reason: SegmentReason::Click,
            }],
        };
        let json = serde_json::to_value(&a).unwrap();
        assert_eq!(json["hasCursorTrail"], true);
        assert_eq!(json["cursorBakedIn"], false);
        assert_eq!(json["zoomSegments"][0]["reason"], "click");
        assert_eq!(json["zoomSegments"][0]["startT"], 1.0);
    }

    /// A session with no `events.revents` must still analyze cleanly — that is
    /// the "recorded before tracking existed" case the editor has to survive.
    #[test]
    fn a_session_with_no_trail_still_analyses() {
        let dir = std::env::temp_dir().join(format!("revate-notrail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A real, probe-able clip so ffprobe succeeds.
        let made = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i",
                "color=c=black:s=320x240:d=0.1", "-y",
            ])
            .arg(dir.join("raw.mp4"))
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if made {
            let a = analyze(&dir).expect("a take with no events should still analyse");
            assert!(!a.has_cursor_trail);
            assert!(a.zoom_segments.is_empty());
            assert_eq!(a.event_count, 0);
            // No `capture.json` means we cannot place the cursor, so the safe
            // assumption is that the system cursor is already in the video.
            assert!(a.cursor_baked_in);
            assert_eq!(a.width, 320);
            assert_eq!(a.height, 240);
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The full chain, on disk: what the recorder writes is what the analysis
    /// reads. This is the test that would have caught the feature being "not
    /// applying" — every piece worked in isolation, but nothing connected them.
    #[test]
    fn the_recorded_trail_drives_the_zoom_plan_end_to_end() {
        let dir = std::env::temp_dir().join(format!("revate-e2e-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // 1. The video the recorder would have produced.
        let made = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i",
                "color=c=black:s=1920x1080:d=2:r=10", "-y",
            ])
            .arg(dir.join("raw.mp4"))
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        // 2. The geometry sidecar, written before the take started.
        write_capture_meta(&dir, &meta()).unwrap();

        // 3. The trail: a cursor settling, then two clicks a little apart in time
        //    but far apart in space (which must NOT merge into one zoom).
        let raw = vec![
            RawEvent::cursor_move(0.0, 300.0, 300.0),
            RawEvent::button(500.0, EventKind::MouseDown, MouseButtonName::Left, 300.0, 300.0),
            RawEvent::cursor_move(1000.0, 1500.0, 700.0),
            RawEvent::button(1500.0, EventKind::MouseDown, MouseButtonName::Left, 1500.0, 700.0),
        ];
        let mut writer =
            crate::events::log::EventWriter::create(&dir.join(crate::events::reader::EVENTS_FILE))
                .unwrap();
        for event in &raw {
            writer.write(event).unwrap();
        }
        writer.flush().unwrap();
        drop(writer);

        if made {
            let a = analyze(&dir).expect("a complete take should analyze");

            assert!(a.has_cursor_trail, "the trail should be usable");
            assert!(!a.cursor_baked_in, "we drew our own cursor, so none is baked in");
            assert_eq!(a.event_count, 4);
            assert_eq!(a.click_count, 2);
            assert_eq!(a.width, 1920);
            assert_eq!(a.height, 1080);

            // Two far-apart clicks are two separate zooms.
            assert_eq!(
                a.zoom_segments.len(),
                2,
                "far-apart clicks must not merge: {:?}",
                a.zoom_segments
            );

            // Each segment targets its own click, and both stay inside the frame.
            for segment in &a.zoom_segments {
                assert!(segment.start_t >= segment.end_t - 10.0, "segment window is inverted");
                assert!(segment.zoom_level > 1.0, "a click should zoom in");
                assert!(
                    segment.x >= 0.0 && segment.x <= 1920.0,
                    "target {} escaped the frame",
                    segment.x
                );
                assert!(
                    segment.y >= 0.0 && segment.y <= 1080.0,
                    "target {} escaped the frame",
                    segment.y
                );
            }
            // Ordered in time, so the export's piecewise filter is well formed.
            assert!(a.zoom_segments[0].start_t <= a.zoom_segments[1].start_t);
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The cursor track must be sampled in the *video's* pixels, which only works
    /// because `capture.json` carries the scale factor.
    #[test]
    fn a_retina_recording_maps_logical_points_into_video_pixels() {
        // A 2× display: the recorded area is 1920×1080 *physical* pixels, so it
        // covers 960×540 logical points. The cursor reports logical points, so a
        // correct mapping has to divide that 2× back out.
        let mut retina = meta();
        retina.primary_scale_factor = 2.0;

        // Logical centre of that area.
        let log = log_of(&[r#"{"t":0.0,"type":"cursor_move","data":{"x":480.0,"y":270.0}}"#]);
        let track = CursorTrack::from_events(&log.events, &retina, 1920, 1080);
        assert!(!track.is_empty(), "a centred pointer is inside a 2× screen");

        // Logical (480,270) → physical (960,540) → the centre of the 1920×1080
        // video. Without the scale factor this would land at (480,270) instead.
        let (x, y) = track.position_at(0.0).unwrap();
        assert!((x - 960.0).abs() < 0.5, "x was {x}");
        assert!((y - 540.0).abs() < 0.5, "y was {y}");

        // A logical point beyond the recorded area maps outside and is dropped.
        let beyond = log_of(&[r#"{"t":0.0,"type":"cursor_move","data":{"x":2000.0,"y":270.0}}"#]);
        let off = CursorTrack::from_events(&beyond.events, &retina, 1920, 1080);
        assert!(off.is_empty(), "a pointer past the edge must not enter the track");
    }
}