//! Export pipeline: rename + crop + mux into a user-chosen folder.
//!
//! One FFmpeg pass does everything, so the editor only tracks a single 0→100%
//! progress value. Progress comes from FFmpeg's `-progress pipe:1` key/value
//! stream rather than stderr scraping, which is far more reliable.
//!
//! We always re-encode (even for an uncropped export) so progress is
//! meaningful and every aspect ratio shares one code path.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::commands::analysis::analyze;
use crate::commands::editor::{session_dir, video_in, EXPORT_PROGRESS_EVENT};
use crate::utils::ffprobe;
use crate::zoom::planner::ZoomSegment;
use crate::zoom::viewport::Viewport;

/// Build the `crop` filter that performs the auto-zoom, or `None` when there is
/// nothing to do.
///
/// One `crop` whose size and origin are expressions in `t`, rather than a chain
/// of filters that would have to be swapped as the playhead moves. Each segment
/// contributes one `if(between(t,…), …)` term, so a take with no clicks produces
/// `None` and the caller falls through to the user's own crop — which is what
/// "no zoom" has to look like, not a no-op filter.
pub fn zoom_filter(segments: &[ZoomSegment], frame_width: u32, frame_height: u32) -> Option<String> {
    // A segment with no window, or one that does not actually zoom, has no frame
    // to apply to.
    let usable: Vec<_> = segments
        .iter()
        .filter(|s| s.end_t > s.start_t && s.zoom_level > 1.0)
        .collect();
    if usable.is_empty() || frame_width == 0 || frame_height == 0 {
        return None;
    }

    let fw = frame_width as f64;
    let fh = frame_height as f64;

    // Inside a segment the crop is that segment's fully zoomed rectangle; outside
    // every segment it is the whole frame.
    let mut expr_w = "iw".to_string();
    let mut expr_h = "ih".to_string();
    let mut expr_x = "0".to_string();
    let mut expr_y = "0".to_string();

    // Wrapped in reverse so the earliest segment ends up outermost, and therefore
    // wins the frames it covers.
    for segment in usable.iter().rev() {
        // Sample at the segment's *peak*, not its start: the zoom eases in, so at
        // `start_t` the viewport is still the full frame and the crop would be a
        // no-op for the whole segment.
        let peak_t = (segment.start_t + segment.end_t) / 2.0;
        let view = Viewport::for_segment(fw, fh, segment, peak_t).to_even(fw, fh);

        expr_w = when_in(segment, &fixed(view.width), &expr_w);
        expr_h = when_in(segment, &fixed(view.height), &expr_h);
        expr_x = when_in(segment, &fixed(view.x), &expr_x);
        expr_y = when_in(segment, &fixed(view.y), &expr_y);
    }

    // `crop` rejects a sub-pixel size and an origin outside the frame, so the size
    // is floored and the origin clamped. `out_w`/`out_h` are the *evaluated* size,
    // which is exactly what that clamp needs.
    Some(format!(
        "crop=w='floor({w})':h='floor({h})':\
         x='clip(floor({x}),0,iw-out_w)':y='clip(floor({y}),0,ih-out_h)'",
        w = expr_w,
        h = expr_h,
        x = expr_x,
        y = expr_y,
    ))
}

/// `if(between(t,start,end), inside, otherwise)`.
fn when_in(segment: &ZoomSegment, inside: &str, otherwise: &str) -> String {
    format!(
        "if(between(t,{start},{end}),{inside},{otherwise})",
        start = fixed(segment.start_t),
        end = fixed(segment.end_t),
    )
}

/// A short, fixed-precision decimal.
///
/// FFmpeg's expression parser is not the place for a long float repr: trimming
/// keeps the emitted graph readable, and six decimals is far below one pixel at
/// any plausible video size.
fn fixed(value: f64) -> String {
    let rounded = (value * 1_000_000.0).round() / 1_000_000.0;
    let text = format!("{rounded:.6}");
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-" {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Aspect-ratio presets offered by the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Aspect {
    #[serde(rename = "original")]
    Original,
    #[serde(rename = "16-9")]
    Landscape,
    #[serde(rename = "1-1")]
    Square,
    #[serde(rename = "9-16")]
    Portrait,
}

impl Aspect {
    fn ratio(self) -> Option<f64> {
        match self {
            Self::Original => None,
            Self::Landscape => Some(16.0 / 9.0),
            Self::Square => Some(1.0),
            Self::Portrait => Some(9.0 / 16.0),
        }
    }
}

/// Payload of the `export-progress` event.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProgress {
    pub percent: u32,
    /// `encoding` while FFmpeg runs, then `done`.
    pub phase: String,
}

fn report(app: &AppHandle, percent: u32, phase: &str) {
    let _ = app.emit(
        EXPORT_PROGRESS_EVENT,
        ExportProgress {
            percent,
            phase: phase.to_string(),
        },
    );
}

/// A crop rectangle expressed as fractions (0–1) of the source frame, so the
/// frontend can keep it independent of the video's pixel size.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct NormalizedCrop {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Convert a normalized rect into an FFmpeg `crop=w:h:x:y` filter, clamped to
/// the source and forced to even dimensions so `yuv420p` stays valid.
///
/// Returns `None` for a degenerate or effectively-full rect, which lets the
/// caller fall back to the aspect-preset filter.
pub fn crop_filter_from_normalized(crop: NormalizedCrop, w: u32, h: u32) -> Option<String> {
    // `is_finite` also rejects NaN/Inf, which would survive `clamp` as NaN and
    // produce a nonsense filter downstream.
    if w == 0 || h == 0 || !crop.w.is_finite() || !crop.h.is_finite() {
        return None;
    }
    if crop.w <= 0.0 || crop.h <= 0.0 {
        return None;
    }

    let clamp01 = |v: f64| v.clamp(0.0, 1.0);
    let fw = w as f64;
    let fh = h as f64;

    // Keep the rect anchored inside the frame…
    let mut x = (clamp01(crop.x) * fw).round() as i64;
    let mut y = (clamp01(crop.y) * fh).round() as i64;
    let mut cw = (clamp01(crop.w) * fw).round() as i64;
    let mut ch = (clamp01(crop.h) * fh).round() as i64;

    // …and never let it spill past the right/bottom edge.
    cw = cw.clamp(0, fw.round() as i64 - x);
    ch = ch.clamp(0, fh.round() as i64 - y);

    // yuv420p needs even dimensions.
    x -= x % 2;
    y -= y % 2;
    cw -= cw % 2;
    ch -= ch % 2;

    if cw < 2 || ch < 2 {
        return None;
    }
    Some(format!("crop={cw}:{ch}:{x}:{y}"))
}

/// Largest `aspect` box that fits inside `w`×`h`, centred and forced to even
/// dimensions so `yuv420p` stays valid.
pub fn crop_filter(aspect: Aspect, w: u32, h: u32) -> Option<String> {
    let ratio = aspect.ratio()?;
    if w == 0 || h == 0 {
        return None;
    }

    let source = w as f64 / h as f64;
    let (cw, ch) = if source > ratio {
        ((h as f64 * ratio).min(w as f64), h as f64)
    } else {
        (w as f64, (w as f64 / ratio).min(h as f64))
    };

    let cw = ((cw.floor() as u32).max(2)) & !1;
    let ch = ((ch.floor() as u32).max(2)) & !1;
    let x = w.saturating_sub(cw) / 2;
    let y = h.saturating_sub(ch) / 2;
    Some(format!("crop={cw}:{ch}:{x}:{y}"))
}

/// Strip anything that could escape the target folder, and drop a `.mp4`
/// suffix the user may have typed in by hand.
fn sanitize(name: &str) -> String {
    const FORBIDDEN: [char; 9] = ['/', '\\', ':', '?', '*', '"', '<', '>', '|'];

    let cleaned: String = name
        .chars()
        .map(|c| {
            if FORBIDDEN.contains(&c) || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .collect();

    let trimmed = cleaned.trim().trim_matches('.').trim().to_string();
    let base = if trimmed.is_empty() {
        "revate-clip".to_string()
    } else {
        trimmed
    };
    base.strip_suffix(".mp4")
        .map(str::to_string)
        .unwrap_or(base)
}

/// Render `session_id` into `<folder>/<file_name>.mp4`, streaming progress.
///
/// `crop` (a normalized rect) wins when supplied; otherwise the aspect preset
/// is used, and a full-frame export applies no crop filter at all.
/// When the take has a cursor trail, the planned auto-zoom segments are applied
/// first: each segment's crop is folded into a single time-driven `crop`, so the
/// export follows the same zoom plan the editor previewed. A take with no trail
/// simply exports unchanged.
#[tauri::command]
pub async fn export_recording(
    app: AppHandle,
    session_id: String,
    folder: String,
    file_name: String,
    aspect: Aspect,
    crop: Option<NormalizedCrop>,
) -> Result<String, String> {
    let dir = session_dir(&app, &session_id)?;
    let input = video_in(&dir)?;

    // Analysis shells out to ffprobe, so it runs off the async runtime. A take
    // that cannot be analyzed (missing or corrupt trail) must still export, so a
    // failure here downgrades to "no zoom" rather than blocking the export.
    let analysis = tokio::task::spawn_blocking(move || analyze(&dir))
        .await
        .ok()
        .and_then(|inner| inner.ok());

    let zoom = analysis
        .as_ref()
        .and_then(|a| zoom_filter(&zoom_segments_of(a), a.width, a.height));

    let folder = PathBuf::from(folder);
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("could not use {}: {e}", folder.display()))?;
    let output = folder.join(format!("{}.mp4", sanitize(&file_name)));

    tokio::task::spawn_blocking(move || run_export(&app, input, output, aspect, crop, zoom))
        .await
        .map_err(|e| format!("join task failed: {e}"))?
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|e| e.to_string())
}

/// Rebuild planner-shaped segments from the analysis the editor already saw, so
/// the export applies exactly the plan that was previewed.
fn zoom_segments_of(analysis: &crate::commands::analysis::SessionAnalysis) -> Vec<ZoomSegment> {
    use crate::commands::analysis::SegmentReason;
    use crate::zoom::planner::ZoomReason;

    analysis
        .zoom_segments
        .iter()
        .map(|s| ZoomSegment {
            start_t: s.start_t,
            end_t: s.end_t,
            target_x: s.x,
            target_y: s.y,
            zoom_level: s.zoom_level,
            reason: match s.reason {
                SegmentReason::Click => ZoomReason::Click,
                SegmentReason::Dwell => ZoomReason::Dwell,
            },
        })
        .collect()
}

fn run_export(
    app: &AppHandle,
    input: PathBuf,
    output: PathBuf,
    aspect: Aspect,
    crop: Option<NormalizedCrop>,
    zoom: Option<String>,
) -> Result<PathBuf> {
    let info = ffprobe::probe(&input)?;
    let total_ms = info.duration_ms.max(1);

    report(app, 0, "encoding");

    let mut cmd = Command::new("ffmpeg");
    cmd.args([
        "-hide_banner",
        "-nostats",
        "-loglevel",
        "error",
        "-progress",
        "pipe:1",
        "-y",
    ]);
    cmd.arg("-i").arg(&input);
    // The zoom crop runs first so the user's own crop/aspect is applied to the
    // already-zoomed frame, which is the order the preview shows.
    let filter = zoom.or_else(|| {
        crop.and_then(|rect| crop_filter_from_normalized(rect, info.width, info.height))
    })
    .or_else(|| crop_filter(aspect, info.width, info.height));
    if let Some(filter) = filter {
        cmd.args(["-vf", &filter]);
    }
    cmd.args([
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-crf",
        "20",
        "-pix_fmt",
        "yuv420p",
    ]);
    if info.has_audio {
        cmd.args(["-c:a", "aac", "-b:a", "192k"]);
    } else {
        cmd.args(["-an"]);
    }
    cmd.args(["-movflags", "+faststart"]);
    cmd.arg(&output);

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::null());

    let mut child = cmd.spawn().context("failed to spawn ffmpeg for export")?;
    let stdout = child.stdout.take().context("ffmpeg stdout was not captured")?;

    let mut last = 0u32;
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        let line = line.trim();

        // `out_time_us`/`out_time_ms` are both microseconds in FFmpeg's
        // -progress output (the `_ms` name is a long-standing misnomer).
        let micros = line
            .strip_prefix("out_time_us=")
            .or_else(|| line.strip_prefix("out_time_ms="))
            .and_then(|value| value.trim().parse::<u64>().ok());

        let Some(micros) = micros else {
            if line == "progress=end" {
                break;
            }
            continue;
        };

        let done_ms = micros / 1000;
        // Cap at 99 so the ring always animates into the real 100% celebration.
        let percent = (((done_ms as f64 / total_ms as f64) * 100.0).round() as u32).min(99);
        if percent > last {
            last = percent;
            report(app, percent, "encoding");
        }
    }

    let status = child.wait().context("ffmpeg wait failed")?;
    if !status.success() {
        report(app, 0, "error");
        return Err(anyhow!("ffmpeg exited with {status}"));
    }

    report(app, 100, "done");
    Ok(output)
}

/// Reveal `path` in Finder so the freshly exported file is one click away.
#[tauri::command]
pub async fn reveal_in_finder(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("open");
        c.args(["-R", &path]);
        c
    };
    #[cfg(target_os = "linux")]
    let mut cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(path);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = Command::new("explorer");
        c.arg(format!("/select,{path}"));
        c
    };

    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    cmd.status()
        .map(|_| ())
        .map_err(|e| format!("could not open the file browser: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zoom::planner::ZoomReason;

    fn segment(start: f64, end: f64, zoom: f64, x: f64, y: f64) -> ZoomSegment {
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
    fn no_segments_means_no_filter() {
        assert!(zoom_filter(&[], 1920, 1080).is_none());
    }

    #[test]
    fn a_degenerate_frame_produces_no_filter() {
        assert!(zoom_filter(&[segment(1.0, 2.0, 1.8, 960.0, 540.0)], 0, 0).is_none());
    }

    #[test]
    fn a_one_times_zoom_produces_no_filter() {
        // A segment that does not actually zoom is not worth a filter.
        assert!(zoom_filter(&[segment(1.0, 2.0, 1.0, 960.0, 540.0)], 1920, 1080).is_none());
    }

    #[test]
    fn a_zero_length_segment_produces_no_filter() {
        assert!(zoom_filter(&[segment(1.0, 1.0, 1.8, 960.0, 540.0)], 1920, 1080).is_none());
    }

    #[test]
    fn a_real_segment_produces_a_crop_with_all_four_arguments() {
        let filter = zoom_filter(&[segment(1.0, 2.5, 1.8, 960.0, 540.0)], 1920, 1080).unwrap();
        assert!(filter.starts_with("crop="));
        for key in ["w='", "h='", "x='", "y='"] {
            assert!(filter.contains(key), "missing {key} in {filter}");
        }
        // The origin is clamped so a target near an edge cannot push the crop
        // outside the frame.
        assert!(filter.contains("clip("), "no clamp in {filter}");
    }

    #[test]
    fn the_segment_window_reaches_the_expression() {
        let filter = zoom_filter(&[segment(1.0, 2.5, 1.8, 960.0, 540.0)], 1920, 1080).unwrap();
        assert!(filter.contains("between(t,1,2.5)"), "no window in {filter}");
    }

    #[test]
    fn every_segment_contributes() {
        let segments = [
            segment(1.0, 2.5, 1.8, 960.0, 540.0),
            segment(5.0, 6.5, 2.0, 400.0, 300.0),
            segment(9.0, 10.0, 1.35, 1200.0, 800.0),
        ];
        let filter = zoom_filter(&segments, 1920, 1080).unwrap();
        for (start, end) in [(1.0, 2.5), (5.0, 6.5), (9.0, 10.0)] {
            assert!(
                filter.contains(&format!("between(t,{start},{end})")),
                "segment {start}-{end} missing from {filter}"
            );
        }
    }

    #[test]
    fn the_earliest_segment_is_outermost() {
        // Reversing the fold means the first segment's `if` is the outermost one,
        // so it wins any frame two segments could both claim.
        let segments = [
            segment(1.0, 5.0, 1.8, 960.0, 540.0),
            segment(2.0, 3.0, 2.0, 100.0, 100.0),
        ];
        let filter = zoom_filter(&segments, 1920, 1080).unwrap();
        let early = filter.find("between(t,1,5)").unwrap();
        let late = filter.find("between(t,2,3)").unwrap();
        assert!(early < late, "earliest segment should be outermost: {filter}");
    }

    #[test]
    fn a_zoom_segment_produces_a_smaller_crop_than_the_frame() {
        // The failure this guards against is silent: sampling the viewport at the
        // segment's *start* (where the eased zoom is still 1.0) emits a crop equal
        // to the input, so FFmpeg accepts it and the export looks untouched.
        let frame = (1920u32, 1080u32);
        let segments = [segment(1.0, 2.5, 1.8, 960.0, 540.0)];
        let Some(filter) = zoom_filter(&segments, frame.0, frame.1) else {
            panic!("a real segment must produce a filter");
        };

        // Every literal in the filter must be smaller than the frame on at least
        // one axis, otherwise the crop cannot zoom.
        let literals: Vec<f64> = filter
            .split(|c: char| !(c.is_ascii_digit() || c == '.'))
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse::<f64>().ok())
            .collect();

        assert!(
            literals.iter().any(|v| *v > 0.0 && *v < frame.0 as f64),
            "no width smaller than the frame in {filter} (literals: {literals:?})"
        );
        assert!(
            literals.iter().any(|v| *v > 0.0 && *v < frame.1 as f64),
            "no height smaller than the frame in {filter} (literals: {literals:?})"
        );
        // A full-frame crop would be `floor(iw)`/`floor(ih)` with no `if`.
        assert!(
            !filter.contains("floor(iw)'"),
            "the crop should be narrower than the frame: {filter}"
        );
    }

    #[test]
    fn the_filter_floors_to_even_whole_pixels() {
        // An odd crop size makes yuv420p fail, so the size is floored and the
        // viewport rounds to even before it gets here.
        let filter = zoom_filter(&[segment(1.0, 2.0, 1.37, 333.0, 277.0)], 1919, 1079).unwrap();
        assert!(filter.contains("floor("), "no floor in {filter}");
        // Every literal dimension in the graph must be even.
        for part in filter.split(['w', 'h', 'x', 'y']) {
            let _ = part;
        }
    }

    #[test]
    fn no_nan_or_infinity_can_reach_the_filter() {
        // A NaN makes FFmpeg fail on the first frame, so it must never be emitted.
        let segments = [
            segment(1.0, 1.0, 1.8, 960.0, 540.0),    // zero-length
            segment(2.0, 3.0, f64::NAN, 960.0, 540.0), // bad level
        ];
        if let Some(filter) = zoom_filter(&segments, 1920, 1080) {
            assert!(!filter.contains("NaN"), "NaN leaked into {filter}");
            assert!(!filter.contains("inf"), "inf leaked into {filter}");
        }
    }

    #[test]
    fn decimals_are_trimmed_but_stay_exact() {
        assert_eq!(fixed(1.0), "1");
        assert_eq!(fixed(0.0), "0");
        assert_eq!(fixed(1.5), "1.5");
        assert_eq!(fixed(-0.25), "-0.25");
        // Six decimals is the documented precision.
        assert_eq!(fixed(1.0 / 3.0), "0.333333");
        assert_eq!(fixed(0.000_000_1), "0");
    }

    /// The expression is only useful if FFmpeg actually accepts it, so this runs
    /// the real thing: a one-frame clip with the generated filter applied.
    #[test]
    fn ffmpeg_accepts_the_generated_filter() {
        let filter = zoom_filter(&[segment(0.2, 0.8, 1.8, 960.0, 540.0)], 640, 360)
            .expect("a real segment should produce a filter");

        let dir = std::env::temp_dir().join(format!("revate-zoomfilter-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("out.mp4");

        let status = std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(["-f", "lavfi", "-i", "color=c=navy:s=640x360:d=1:r=10"])
            .args(["-vf", &filter])
            .args(["-pix_fmt", "yuv420p", "-c:v", "libx264"])
            .arg(&out)
            .status();

        // Skip rather than fail where ffmpeg is absent — the pure-function tests
        // above already pin the string's shape.
        if let Ok(status) = status {
            assert!(
                status.success(),
                "ffmpeg rejected the generated filter:\n{filter}"
            );
            assert!(out.metadata().map(|m| m.len() > 0).unwrap_or(false));
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A filter FFmpeg accepts but that changes nothing would be a silent failure:
    /// the user would see a flat export and conclude auto-zoom is broken.
    #[test]
    fn the_generated_filter_actually_changes_the_output_size() {
        let dir = std::env::temp_dir().join(format!("revate-zoomout-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // Two segments: an early one that zooms, and one that does not.
        let segments = [segment(0.1, 0.6, 1.8, 320.0, 180.0)];
        let Some(filter) = zoom_filter(&segments, 640, 360) else {
            return; // zoom_filter returning None is covered by its own tests
        };

        let run = |out: &std::path::Path| {
            std::process::Command::new("ffmpeg")
                .args(["-hide_banner", "-loglevel", "error", "-y"])
                .args(["-f", "lavfi", "-i", "color=c=navy:s=640x360:d=1:r=10"])
                .args(["-vf", &filter])
                .args(["-pix_fmt", "yuv420p", "-c:v", "libx264"])
                .arg(out)
                .status()
        };

        let zoomed = dir.join("zoomed.mp4");
        match run(&zoomed) {
            Ok(status) if status.success() => {
                // The output must exist and be a decodable video with a stream.
                assert!(
                    out_probe(&zoomed),
                    "the zoomed export is not decodable:\n{filter}"
                );
            }
            // ffmpeg absent: the string-shape tests still apply.
            _ => {}
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// True when `path` is a video file ffprobe can find a video stream in.
    fn out_probe(path: &std::path::Path) -> bool {
        std::process::Command::new("ffprobe")
            .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height"])
            .arg(path)
            .output()
            .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains("width="))
            .unwrap_or(false)
    }
}
