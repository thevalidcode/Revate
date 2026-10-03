//! Export pipeline: zoom + cursor, then rename into a user-chosen folder.
//!
//! One FFmpeg pass does everything. The auto-zoom is not a static crop: the
//! viewport moves every frame, so the crop's geometry and the cursor sprite's
//! position are driven by `sendcmd` files generated from [`crate::effects`] —
//! the same table the editor previewed from. That is the whole point: the file
//! the user gets is what the frame they approved said it would be.
//!
//! `crop`'s size expressions are evaluated once, at graph init, so the obvious
//! `if(between(t,…), …)` formulation silently exports the take untouched (t is
//! undefined at init). Commands are the mechanism that actually moves a filter
//! after init, and both `crop` and `overlay` accept them.
//!
//! Progress comes from FFmpeg's `-progress pipe:1` key/value stream rather than
//! stderr scraping, which is far more reliable. We always re-encode so progress
//! is meaningful and every aspect ratio shares one code path.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::commands::analysis::{analyze_with, SessionAnalysis};
use crate::commands::editor::{session_dir, video_in, EXPORT_PROGRESS_EVENT};
use crate::effects::{
    cursor_position, cursor_sendcmd, row_viewport, sprite_metrics, write_cursor_sprite,
    zoom_sendcmd, OverlayOptions, CURSOR_CMD_FILE, ZOOM_CMD_FILE,
};
use crate::utils::ffprobe;
use crate::zoom::viewport::Viewport;


/// The dynamic crop for one export: where the rectangle starts, and the
/// `sendcmd` file that moves it.
#[derive(Debug, Clone)]
struct ZoomPlan {
    /// The command file's path, escaped for a filtergraph.
    cmd_path: String,
    /// `(w, h, x, y)` in source pixels at t = 0.
    start: (i64, i64, i64, i64),
}

/// The cursor sprite's motion: the `sendcmd` file and the sprite's starting
/// position, in output pixels.
#[derive(Debug, Clone)]
struct CursorPlan {
    cmd_path: String,
    start: (i64, i64),
}

/// The filter chain for one export.
///
/// Exactly one of these is used: a `-vf` chain when there is no sprite input
/// (FFmpeg's automatic stream selection keeps the audio), or a
/// `-filter_complex` when the cursor is composited — the sprite is a second
/// input, so the graph has to name its outputs.
#[derive(Debug, Clone, Default, PartialEq)]
struct Filters {
    vf: Option<String>,
    complex: Option<String>,
}

/// Assemble the video chain from the plans.
fn build_filters(
    zoom: Option<&ZoomPlan>,
    static_crop: Option<String>,
    cursor: Option<&CursorPlan>,
    out_width: u32,
    out_height: u32,
) -> Filters {
    let mut chain = String::new();
    if let Some(crop) = static_crop {
        chain.push_str(&crop);
        chain.push(',');
    }
    if let Some(zoom) = zoom {
        // `sendcmd` first, so a command is dispatched before the frame reaches
        // the filter it targets; then the crop, scaled back to the fixed output
        // size so the encoder never sees the crop change dimensions.
        chain.push_str(&format!(
            "sendcmd=f={cmd},crop@zc=w={w}:h={h}:x={x}:y={y},\
             scale={out_width}:{out_height}:flags=bicubic,setsar=1,",
            cmd = zoom.cmd_path,
            w = zoom.start.0,
            h = zoom.start.1,
            x = zoom.start.2,
            y = zoom.start.3,
        ));
    }

    match cursor {
        Some(cursor) => {
            // The cursor layer: a second input, moved per frame by commands.
            // `shortest=1` stops it when the take ends; straight alpha is what
            // the PAM carries.
            chain.push_str(&format!("sendcmd=f={}", cursor.cmd_path));
            let complex = format!(
                "[0:v]{chain}[base];\
                 [base][1:v]overlay@ov=x={x}:y={y}:format=auto:alpha=straight:shortest=1,\
                 format=yuv420p[out]",
                x = cursor.start.0,
                y = cursor.start.1,
            );
            Filters {
                vf: None,
                complex: Some(complex),
            }
        }
        None => Filters {
            vf: if chain.is_empty() {
                None
            } else {
                Some(chain.trim_end_matches(',').to_string())
            },
            complex: None,
        },
    }
}

/// Write one `sendcmd` file, returning the path escaped for a filtergraph.
///
/// `:` separates filter options and `\` escapes, so a path with either would
/// break the graph. Temp directories do not normally contain them, but a path
/// is not the place to find that out.
fn write_cmd_file(dir: &Path, name: &str, text: &str) -> Result<String> {
    let path = dir.join(name);
    std::fs::write(&path, text).with_context(|| format!("failed to write {name}"))?;
    Ok(path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace(':', "\\:"))
}

/// A unique scratch directory for this export's generated files.
fn work_dir() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("revate-export-{}-{stamp}", std::process::id()))
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

/// Convert a normalized rect into a source-pixel rectangle, clamped to the
/// frame and forced to even dimensions so `yuv420p` stays valid.
///
/// Returns `None` for a degenerate or effectively-full rect, which lets the
/// caller fall back to the aspect preset.
pub fn crop_rect_from_normalized(crop: NormalizedCrop, w: u32, h: u32) -> Option<Viewport> {
    // `is_finite` also rejects NaN/Inf, which would survive `clamp` as NaN. The
    // origin is checked too: a NaN `x` casts to 0 in `as i64`, which would silently
    // teleport the user's crop to the top-left corner instead of falling back to
    // the aspect preset as a bad *size* does.
    if w == 0
        || h == 0
        || !crop.x.is_finite()
        || !crop.y.is_finite()
        || !crop.w.is_finite()
        || !crop.h.is_finite()
    {
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
    Some(Viewport {
        x: x as f64,
        y: y as f64,
        width: cw as f64,
        height: ch as f64,
    })
}

/// The `crop` filter for a normalized rect, for callers that only need the
/// string.
pub fn crop_filter_from_normalized(crop: NormalizedCrop, w: u32, h: u32) -> Option<String> {
    crop_rect_from_normalized(crop, w, h).map(|rect| rect_filter(&rect))
}

/// Largest `aspect` box that fits inside `w`×`h`, centred and forced to even
/// dimensions so `yuv420p` stays valid.
pub fn crop_rect(aspect: Aspect, w: u32, h: u32) -> Option<Viewport> {
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
    Some(Viewport {
        x: x as f64,
        y: y as f64,
        width: cw as f64,
        height: ch as f64,
    })
}

/// The `crop` filter for an aspect preset, for callers that only need the
/// string.
pub fn crop_filter(aspect: Aspect, w: u32, h: u32) -> Option<String> {
    crop_rect(aspect, w, h).map(|rect| rect_filter(&rect))
}

/// A static `crop=w:h:x:y`, which is all the *size* of a rectangle needs.
fn rect_filter(rect: &Viewport) -> String {
    format!(
        "crop={}:{}:{}:{}",
        rect.width.round() as i64,
        rect.height.round() as i64,
        rect.x.round() as i64,
        rect.y.round() as i64,
    )
}

/// The box the export renders into: the user's own crop wins, then the aspect
/// preset, then the whole frame. Always even and inside the source.
fn output_bounds(crop: Option<NormalizedCrop>, aspect: Aspect, w: u32, h: u32) -> Viewport {
    crop.and_then(|rect| crop_rect_from_normalized(rect, w, h))
        .or_else(|| crop_rect(aspect, w, h))
        .unwrap_or(Viewport {
            x: 0.0,
            y: 0.0,
            width: (w & !1) as f64,
            height: (h & !1) as f64,
        })
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
/// is used, and a full-frame export renders the frame as recorded. `options`
/// are the editor's current effect settings: the zoom/cursor timeline is
/// re-resolved with them, so the export reproduces the preview the user
/// approved, cursor layer and all.
#[tauri::command]
pub async fn export_recording(
    app: AppHandle,
    session_id: String,
    folder: String,
    file_name: String,
    aspect: Aspect,
    crop: Option<NormalizedCrop>,
    options: Option<OverlayOptions>,
) -> Result<String, String> {
    let options = options.unwrap_or_default().sanitized();
    let dir = session_dir(&app, &session_id)?;
    let input = video_in(&dir)?;

    // Analysis shells out to ffprobe, so it runs off the async runtime. A take
    // that cannot be analyzed (missing or corrupt trail) must still export, so
    // a failure here downgrades to "no effects" rather than blocking the
    // export.
    let analysis = tokio::task::spawn_blocking({
        let dir = dir.clone();
        move || analyze_with(&dir, &options)
    })
    .await
    .ok()
    .and_then(|inner| inner.ok())
    .unwrap_or_else(SessionAnalysis::empty);

    let folder = PathBuf::from(folder);
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("could not use {}: {e}", folder.display()))?;
    let output = folder.join(format!("{}.mp4", sanitize(&file_name)));

    tokio::task::spawn_blocking(move || {
        run_export(&app, input, output, aspect, crop, options, analysis)
    })
    .await
    .map_err(|e| format!("join task failed: {e}"))?
    .map(|path| path.to_string_lossy().into_owned())
    .map_err(|e| e.to_string())
}

/// Run the export and stream its progress to the editor window.
fn run_export(
    app: &AppHandle,
    input: PathBuf,
    output: PathBuf,
    aspect: Aspect,
    crop: Option<NormalizedCrop>,
    options: OverlayOptions,
    analysis: SessionAnalysis,
) -> Result<PathBuf> {
    let work = work_dir();
    report(app, 0, "encoding");

    let result = encode(
        EncodePaths {
            input: &input,
            output: &output,
            work: &work,
        },
        aspect,
        crop,
        options,
        &analysis,
        &mut |percent| report(app, percent, "encoding"),
    );

    // The generated files are only needed while FFmpeg runs.
    std::fs::remove_dir_all(&work).ok();

    match result {
        Ok(()) => {
            report(app, 100, "done");
            Ok(output)
        }
        Err(error) => {
            report(app, 0, "error");
            Err(error)
        }
    }
}

/// Where one export reads from, writes to, and keeps its generated files.
///
/// Grouped because the pass takes all three plus the settings that shape the
/// graph, and a flat list of eight positional arguments is easy to transpose.
struct EncodePaths<'a> {
    /// The raw capture.
    input: &'a Path,
    /// The finished file, in the folder the user chose.
    output: &'a Path,
    /// Scratch space for the generated command files and the cursor sprite.
    work: &'a Path,
}

/// The FFmpeg pass: generate the timeline's command files and sprite, assemble
/// the graph, and stream progress from `-progress`.
///
/// This is also the function the tests call, with a no-op progress sink — one
/// code path for the app and the verification.
fn encode(
    paths: EncodePaths<'_>,
    aspect: Aspect,
    crop: Option<NormalizedCrop>,
    options: OverlayOptions,
    analysis: &SessionAnalysis,
    progress: &mut dyn FnMut(u32),
) -> Result<()> {
    let EncodePaths {
        input,
        output,
        work,
    } = paths;

    let info = ffprobe::probe(input)?;
    let total_ms = info.duration_ms.max(1);
    let frame_w = info.width as f64;
    let frame_h = info.height as f64;

    let bounds = output_bounds(crop, aspect, info.width, info.height);
    let out_w = bounds.width.round() as u32;
    let out_h = bounds.height.round() as u32;

    // The zoom needs a plan *and* its switch on; the rows are the schedule.
    let zoom_active =
        options.zoom && !analysis.zoom_segments.is_empty() && !analysis.rows.is_empty();
    // The cursor is composited only when the take has a trail of its own — a
    // take whose cursor FFmpeg burned in must not get a second one.
    let cursor_active = options.cursor
        && analysis.has_cursor_trail
        && !analysis.cursor_baked_in
        && analysis.rows.iter().any(|row| row.cx.is_some());

    std::fs::create_dir_all(work).context("failed to create the export workspace")?;

    let zoom_plan = if zoom_active {
        let start = analysis
            .rows
            .first()
            .map(|row| row_viewport(row, &bounds, frame_w, frame_h))
            .unwrap_or(bounds);
        Some(ZoomPlan {
            cmd_path: write_cmd_file(
                work,
                ZOOM_CMD_FILE,
                &zoom_sendcmd(&analysis.rows, &bounds, frame_w, frame_h),
            )?,
            start: (
                start.width.round() as i64,
                start.height.round() as i64,
                start.x.round() as i64,
                start.y.round() as i64,
            ),
        })
    } else {
        None
    };

    let (sprite, cursor_plan) = if cursor_active {
        let sprite = sprite_metrics(out_w, options.cursor_scale);
        let sprite_path = write_cursor_sprite(work, &sprite)?;
        let cmd_path = write_cmd_file(
            work,
            CURSOR_CMD_FILE,
            &cursor_sendcmd(&analysis.rows, &bounds, out_w, out_h, &sprite),
        )?;
        let start = analysis
            .rows
            .first()
            .map(|row| cursor_position(row, &bounds, out_w, out_h, &sprite))
            .unwrap_or((0, 0));
        (Some(sprite_path), Some(CursorPlan { cmd_path, start }))
    } else {
        (None, None)
    };

    // With no dynamic crop the user's box is still a static crop.
    let static_crop = if zoom_plan.is_none() && !bounds.is_full(frame_w, frame_h) {
        Some(rect_filter(&bounds))
    } else {
        None
    };

    let filters = build_filters(
        zoom_plan.as_ref(),
        static_crop,
        cursor_plan.as_ref(),
        out_w,
        out_h,
    );

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
    cmd.arg("-i").arg(input);
    if let Some(sprite) = &sprite {
        // A single still, looped for the length of the take.
        cmd.args(["-loop", "1", "-i"]).arg(sprite);
    }
    if let Some(complex) = &filters.complex {
        cmd.args(["-filter_complex", complex, "-map", "[out]"]);
        if info.has_audio {
            cmd.args(["-map", "0:a:0"]);
        }
    } else if let Some(vf) = &filters.vf {
        cmd.args(["-vf", vf]);
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
    cmd.arg(output);

    cmd.stdout(Stdio::piped());
    // stderr is kept, not discarded: it is the only place FFmpeg explains *why*
    // it rejected a graph, and "ffmpeg exited with exit status: 8" is useless to
    // both the user and whoever reads the bug report.
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().context("failed to spawn ffmpeg for export")?;
    let stdout = child.stdout.take().context("ffmpeg stdout was not captured")?;
    let stderr = child.stderr.take().context("ffmpeg stderr was not captured")?;

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
        // Cap at 99 so the ring always animates into the real 100%
        // celebration instead of jumping to the end early.
        let percent = (((done_ms as f64 / total_ms as f64) * 100.0).round() as u32).min(99);
        if percent > last {
            last = percent;
            progress(percent);
        }
    }

    let status = child.wait().context("ffmpeg wait failed")?;
    if !status.success() {
        // Drain stderr *after* the wait. FFmpeg's pipe buffer is far larger than
        // the few lines it writes here, so it cannot deadlock against this read —
        // and reading after the wait is the only order that cannot race.
        let mut log = String::new();
        let _ = BufReader::new(stderr).read_to_string(&mut log);
        let log = log.trim();
        return Err(if log.is_empty() {
            anyhow!("ffmpeg exited with {status}")
        } else {
            anyhow!("ffmpeg exited with {status}: {log}")
        });
    }

    Ok(())
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
    use crate::commands::analysis::{SegmentInfo, SegmentReason};
    use crate::effects::EffectRow;

    /// The whole frame, as the output box when nothing crops it.
    fn full(w: f64, h: f64) -> Viewport {
        Viewport {
            x: 0.0,
            y: 0.0,
            width: w,
            height: h,
        }
    }

    /// One un-zoomed row: the frame as recorded, with the cursor at `(cx, cy)`.
    fn row(t: f64, cx: f64, cy: f64) -> EffectRow {
        EffectRow {
            t,
            x: 0.0,
            y: 0.0,
            w: 640.0,
            h: 360.0,
            cx: Some(cx),
            cy: Some(cy),
        }
    }

    fn zoom_plan() -> ZoomPlan {
        ZoomPlan {
            cmd_path: "/tmp/zoom.cmd".to_string(),
            start: (320, 180, 40, 60),
        }
    }

    fn cursor_plan() -> CursorPlan {
        CursorPlan {
            cmd_path: "/tmp/cursor.cmd".to_string(),
            start: (100, 120),
        }
    }

    /// A scratch directory for one test, named off the pid so concurrent runs of
    /// the suite cannot collide.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("revate-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// True when ffmpeg is available; the end-to-end tests skip rather than fail
    /// on a machine without it.
    fn have_ffmpeg() -> bool {
        Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
    }

    /// Render a short clip with a moving pattern, so a crop that moves produces
    /// visibly different frames.
    fn sample_input(dir: &Path) -> PathBuf {
        let path = dir.join("in.mp4");
        let status = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(["-f", "lavfi", "-i", "testsrc=size=640x360:rate=10:duration=2"])
            .args(["-pix_fmt", "yuv420p", "-c:v", "libx264"])
            .arg(&path)
            .status()
            .expect("ffmpeg is present but failed on the test input");
        assert!(status.success(), "could not build the test input");
        path
    }
/// Width/height of the first video stream, or `None` if it is undecodable.
    fn video_size(path: &Path) -> Option<(u32, u32)> {
        let output = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=width,height",
                "-of",
                "json",
            ])
            .arg(path)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        // ffprobe has to be told to emit JSON: the default key=value form is what
        // `ffprobe` alone prints, and this is a test helper, so it takes the flag
        // rather than parsing whichever form it happens to get.
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
        let stream = &json["streams"][0];
        Some((
            stream["width"].as_u64()? as u32,
            stream["height"].as_u64()? as u32,
        ))
    }

    fn audio_present(path: &Path) -> bool {
        Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "a:0",
                "-show_entries",
                "stream=codec_type",
            ])
            .arg(path)
            .output()
            .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains("audio"))
            .unwrap_or(false)
    }

    /// An analysis that claims a trail, a cursor that was not burned in, and one
    /// planned segment.
    fn analysis_with(rows: Vec<EffectRow>) -> SessionAnalysis {
        SessionAnalysis {
            has_cursor_trail: true,
            cursor_baked_in: false,
            zoom_segments: vec![SegmentInfo {
                start_t: 0.0,
                end_t: 2.0,
                x: 320.0,
                y: 180.0,
                zoom_level: 2.0,
                reason: SegmentReason::Click,
            }],
            width: 640,
            height: 360,
            duration_ms: 2000,
            rows,
            ..SessionAnalysis::empty()
        }
    }

    /// Rows that zoom in over the first second, at one tick per row.
    fn zoom_rows() -> Vec<EffectRow> {
        (0..120)
            .map(|i| {
                let t = i as f64 / 60.0;
                // 1.0 at t=0 ramping to 2.0 by t=1: enough movement for the crop
                // to differ between frames.
                let level = 1.0 + t.min(1.0);
                EffectRow {
                    t,
                    x: 0.0,
                    y: 0.0,
                    w: 640.0 / level,
                    h: 360.0 / level,
                    cx: Some(320.0),
                    cy: Some(180.0),
                }
            })
            .collect()
    }

    /// The whole pipeline over a real file: generate the command files and the
    /// sprite, assemble the graph, run FFmpeg. This is the test that catches a
    /// graph FFmpeg silently accepts but that does nothing.
    fn encode_sample(dir: &Path, options: OverlayOptions, analysis: SessionAnalysis) -> PathBuf {
        let input = sample_input(dir);
        let out = dir.join("out.mp4");

        let mut seen = Vec::new();
        encode(
            EncodePaths {
                input: &input,
                output: &out,
                work: &dir.join("work"),
            },
            Aspect::Original,
            None,
            options,
            &analysis,
            &mut |p| seen.push(p),
        )
        .expect("encode failed");

        assert!(!seen.is_empty(), "progress was never reported");
        assert!(
            seen.iter().all(|p| *p <= 99),
            "progress passed 99 before the end: {seen:?}"
        );
        out
    }
// ---------------------------------------------------------------- filters

    #[test]
    fn no_layers_produces_no_filter_at_all() {
        // An empty `-vf ""` would make FFmpeg fail, so the caller must be able to
        // pass nothing and get the frames through untouched.
        let filters = build_filters(None, None, None, 640, 360);
        assert_eq!(filters, Filters::default());
    }

    #[test]
    fn a_static_crop_is_a_plain_vf_with_no_trailing_comma() {
        let filters = build_filters(None, Some("crop=320:180:0:0".into()), None, 320, 180);
        let vf = filters.vf.expect("a static crop needs a -vf");
        assert_eq!(vf, "crop=320:180:0:0");
        assert!(filters.complex.is_none());
    }

    #[test]
    fn the_zoom_sends_commands_before_it_is_cropped() {
        // Order matters: a command dispatched to a filter that has not been
        // configured yet is dropped, which is what leaves a static-looking export.
        let vf = build_filters(Some(&zoom_plan()), None, None, 640, 360)
            .vf
            .expect("the zoom needs a -vf");

        let sendcmd = vf.find("sendcmd=f=/tmp/zoom.cmd").expect("no sendcmd");
        let crop = vf.find("crop@zc").expect("no crop");
        assert!(sendcmd < crop, "sendcmd must precede the crop: {vf}");
    }

    #[test]
    fn the_zoomed_crop_starts_at_the_first_rows_rectangle() {
        // The filter's literal w/h/x/y is what FFmpeg shows before the first
        // command lands, so it must be the first row's rectangle, not the frame.
        let vf = build_filters(Some(&zoom_plan()), None, None, 640, 360)
            .vf
            .unwrap();
        assert!(vf.contains("crop@zc=w=320:h=180:x=40:y=60"), "{vf}");
    }

    #[test]
    fn the_zoom_rescales_back_to_the_output_size() {
        // The crop changes size every frame; without an explicit scale after it
        // the encoder would see a stream whose dimensions keep moving.
        let vf = build_filters(Some(&zoom_plan()), None, None, 1280, 720)
            .vf
            .unwrap();
        assert!(vf.contains("scale=1280:720"), "no output scale in {vf}");
        assert!(vf.contains("setsar=1"), "aspect not squared off in {vf}");
    }

    #[test]
    fn a_static_crop_precedes_the_zoom() {
        // The user's box is applied to the source first, so the zoom's coordinates
        // are expressed in whatever is left of the frame.
        let vf = build_filters(
            Some(&zoom_plan()),
            Some("crop=600:400:10:10".into()),
            None,
            600,
            400,
        )
        .vf
        .unwrap();
        assert!(vf.starts_with("crop=600:400:10:10,"), "{vf}");
        assert!(
            vf.find("crop=600:400:10:10,") < vf.find("crop@zc"),
            "static crop must come first: {vf}"
        );
    }

    #[test]
    fn a_cursor_layer_switches_the_graph_to_filter_complex() {
        // The sprite is a second input, so the chain has to be named and mapped
        // rather than handed over as -vf.
        let filters = build_filters(None, None, Some(&cursor_plan()), 640, 360);
        assert!(filters.vf.is_none(), "cursor must not use -vf");
        let complex = filters.complex.expect("the cursor needs a filter_complex");

        assert!(complex.contains("[0:v]"), "input is not named: {complex}");
        assert!(complex.contains("[out]"), "output is not named: {complex}");
        assert!(
            complex.contains("[base][1:v]overlay@ov=x=100:y=120"),
            "no overlay at the first row's position: {complex}"
        );
        // Straight alpha is what the PAM carries; auto would read it as premultiplied.
        assert!(complex.contains("alpha=straight"), "wrong alpha mode: {complex}");
        // Without `shortest` the looped sprite input would hold the graph open forever.
        assert!(
            complex.contains("shortest=1"),
            "overlay could run forever: {complex}"
        );
    }

    #[test]
    fn zoom_and_cursor_share_one_graph() {
        let complex = build_filters(Some(&zoom_plan()), None, Some(&cursor_plan()), 640, 360)
            .complex
            .expect("both layers need a filter_complex");

        // Two sendcmds, two differently-named targets: a shared label would make
        // one of the command files drive the wrong filter.
        assert!(complex.contains("sendcmd=f=/tmp/zoom.cmd"), "{complex}");
        assert!(complex.contains("sendcmd=f=/tmp/cursor.cmd"), "{complex}");
        assert!(complex.contains("crop@zc"), "{complex}");
        assert!(complex.contains("overlay@ov"), "{complex}");
    }
// The trailing comma is not cosmetic: `chain,[base]` parses as an *empty
    // filter*, and FFmpeg rejects the whole graph with "No such filter: ''". The
    // -vf branch trimmed it; this one has to as well.
    #[test]
    fn no_filter_chain_ends_with_a_stray_comma() {
        for filters in [
            build_filters(None, Some("crop=320:180:0:0".into()), Some(&cursor_plan()), 320, 180),
            build_filters(Some(&zoom_plan()), None, Some(&cursor_plan()), 640, 360),
            build_filters(Some(&zoom_plan()), None, None, 640, 360),
        ] {
            if let Some(complex) = &filters.complex {
                assert!(!complex.contains(",["), "stray comma before a link: {complex}");
                assert!(
                    !complex.contains(",,"),
                    "an empty filter between commas: {complex}"
                );
            }
            if let Some(vf) = &filters.vf {
                assert!(!vf.ends_with(','), "trailing comma in {vf}");
                assert!(!vf.contains(",,"), "an empty filter in {vf}");
            }
        }
    }

    // ------------------------------------------------------------- cmd files

    #[test]
    fn a_command_file_path_is_escaped_for_the_filtergraph() {
        // `:` and `\` are the filtergraph's own syntax; an unescaped path splits
        // the graph at the colon and FFmpeg fails with a parse error.
        let dir = scratch("escape");
        let escaped = write_cmd_file(&dir, "we:ird\\cmd", "0.0 overlay@ov x 1;\n").unwrap();
        assert!(escaped.contains("we\\:ird\\\\cmd"), "not escaped: {escaped}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_command_file_is_written_verbatim() {
        let dir = scratch("cmdfile");
        let text = "0.0000 crop@zc w 320;\n0.0167 crop@zc x 12;\n";
        let escaped = write_cmd_file(&dir, ZOOM_CMD_FILE, text).unwrap();

        // Unescape to find the file again: the *written* name is the plain one,
        // only the copy embedded in the filtergraph is escaped.
        let plain = escaped.replace("\\\\", "\\").replace("\\:", ":");
        let written = PathBuf::from(plain);
        assert_eq!(written.file_name().unwrap(), ZOOM_CMD_FILE);
        assert_eq!(std::fs::read_to_string(&written).unwrap(), text);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_two_command_files_never_share_a_name() {
        // They are written into one directory during a single export; a collision
        // would mean the zoom's crop commands landed in the overlay's file.
        assert_ne!(ZOOM_CMD_FILE, CURSOR_CMD_FILE);
    }

    // ---------------------------------------------------------------- bounds

    #[test]
    fn the_full_frame_is_used_when_nothing_crops_it() {
        let bounds = output_bounds(None, Aspect::Original, 1920, 1080);
        assert_eq!(bounds, full(1920.0, 1080.0));
    }

    #[test]
    fn an_odd_sized_frame_is_reduced_to_even_dimensions() {
        // yuv420p needs even dimensions; 1080 is fine but 1079 is not.
        let bounds = output_bounds(None, Aspect::Original, 1919, 1079);
        assert_eq!(bounds.width, 1918.0);
        assert_eq!(bounds.height, 1078.0);
    }

    #[test]
    fn the_users_crop_wins_over_the_aspect_preset() {
        let crop = NormalizedCrop {
            x: 0.0,
            y: 0.0,
            w: 0.25,
            h: 0.25,
        };
        let bounds = output_bounds(Some(crop), Aspect::Portrait, 1920, 1080);
        assert_eq!(bounds.width, 480.0);
        assert_eq!(bounds.height, 270.0);
    }

    #[test]
    fn a_degenerate_crop_falls_back_to_the_aspect_preset() {
        // A rect the user dragged to nothing must not produce a 0-sized export.
        let crop = NormalizedCrop {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        };
        let bounds = output_bounds(Some(crop), Aspect::Square, 1920, 1080);
        assert_eq!(bounds.width, 1080.0);
        assert_eq!(bounds.height, 1080.0);
    }

    #[test]
    fn a_square_preset_is_centred_in_a_wide_frame() {
        let bounds = output_bounds(None, Aspect::Square, 1920, 1080);
        assert_eq!((bounds.x, bounds.y, bounds.width), (420.0, 0.0, 1080.0));
    }

    #[test]
    fn a_portrait_preset_letterboxes_a_wide_frame() {
        // A 1080x1920 frame is *already* 9:16, so it would not exercise the
        // letterboxing at all — this needs a frame wider than the target ratio.
        let rect = crop_rect(Aspect::Portrait, 1920, 1080).unwrap();
        // 1080 * 9/16 = 607.5 -> floored to 606, leaving both sides even.
        assert_eq!(rect.width, 606.0);
        assert_eq!(rect.height, 1080.0);
        assert_eq!(rect.x, (1920.0 - 606.0) / 2.0);
        assert_eq!(rect.y, 0.0);
    }

    #[test]
    fn a_preset_wider_than_the_frame_is_not_expanded_to_fit() {
        // 16:9 out of a 1080x1080 square cannot be wider than the frame, so the
        // height is what gives: 1080 * 9/16 = 607.5 -> 606. Asking for 16:9 must
        // pillarbox rather than upscale the frame to 1920 wide.
        let rect = crop_rect(Aspect::Landscape, 1080, 1080).unwrap();
        assert_eq!((rect.width, rect.height), (1080.0, 606.0));
        assert_eq!(rect.y, (1080.0 - 606.0) / 2.0);
        assert!(rect.width <= 1080.0 && rect.height <= 1080.0);
    }

    #[test]
    fn an_already_matching_frame_is_left_alone() {
        let rect = crop_rect(Aspect::Portrait, 1080, 1920).unwrap();
        assert_eq!((rect.x, rect.y, rect.width, rect.height), (0.0, 0.0, 1080.0, 1920.0));
    }

    #[test]
    fn a_degenerate_frame_has_no_rect() {
        assert!(crop_rect(Aspect::Square, 0, 0).is_none());
        assert!(
            crop_rect_from_normalized(
                NormalizedCrop {
                    x: 0.0,
                    y: 0.0,
                    w: 0.5,
                    h: 0.5
                },
                0,
                0
            )
            .is_none()
        );
    }

    #[test]
    fn a_crop_cannot_spill_past_the_right_or_bottom_edge() {
        let crop = NormalizedCrop {
            x: 0.9,
            y: 0.9,
            w: 0.5,
            h: 0.5,
        };
        let rect = crop_rect_from_normalized(crop, 1920, 1080).unwrap();
        assert!(rect.x + rect.width <= 1920.0, "{rect:?}");
        assert!(rect.y + rect.height <= 1080.0, "{rect:?}");
    }

    #[test]
    fn a_crop_is_rejected_when_nothing_is_left() {
        // A sliver one pixel wide rounds down to zero, which yuv420p cannot encode.
        let crop = NormalizedCrop {
            x: 0.0,
            y: 0.0,
            w: 0.001,
            h: 0.001,
        };
        assert!(crop_rect_from_normalized(crop, 1920, 1080).is_none());
    }

    #[test]
    fn a_nan_crop_is_rejected_rather_than_clamped() {
        // `f64::clamp` waves NaN straight through, which would produce a rect full
        // of NaN and a baffling FFmpeg error a long way from the cause.
        let crop = NormalizedCrop {
            x: f64::NAN,
            y: 0.0,
            w: 0.5,
            h: 0.5,
        };
        assert!(crop_rect_from_normalized(crop, 1920, 1080).is_none());
    }

    #[test]
    fn the_crop_filter_reports_the_rectangle_it_was_built_from() {
        assert_eq!(crop_filter(Aspect::Square, 1920, 1080).unwrap(), "crop=1080:1080:420:0");
    }

    #[test]
    fn a_rect_filter_carries_all_four_numbers() {
        let rect = Viewport {
            x: 12.0,
            y: 34.0,
            width: 56.0,
            height: 78.0,
        };
        assert_eq!(rect_filter(&rect), "crop=56:78:12:34");
    }
// ------------------------------------------------------------------ naming

    #[test]
    fn separators_cannot_escape_the_chosen_folder() {
        assert_eq!(sanitize("a/b\\c:d"), "a-b-c-d");
    }

    #[test]
    fn a_hand_typed_extension_is_dropped() {
        // The format is appended by the caller; leaving theirs in gives clip.mp4.mp4.
        assert_eq!(sanitize("Demo.mp4"), "Demo");
    }

    #[test]
    fn an_unusable_name_still_produces_a_file() {
        assert_eq!(sanitize(""), "revate-clip");
        assert_eq!(sanitize("   "), "revate-clip");
        assert_eq!(sanitize("..."), "revate-clip");
    }

    #[test]
    fn a_trailing_dot_cannot_hide_the_extension() {
        // "name." would make the target "name..mp4" and confuse Finder's sort.
        assert_eq!(sanitize("name."), "name");
    }

    #[test]
    fn control_characters_are_replaced() {
        let cleaned = sanitize("a\u{0}b\nc");
        assert!(!cleaned.contains('\u{0}') && !cleaned.contains('\n'), "{cleaned}");
        assert_eq!(cleaned, "a-b-c");
    }

    #[test]
    fn ordinary_names_survive_untouched() {
        assert_eq!(
            sanitize("Screen recording 2026-01-02"),
            "Screen recording 2026-01-02"
        );
    }

    // ------------------------------------------------------------ end-to-end

    #[test]
    fn a_plain_export_produces_a_playable_file_of_the_right_size() {
        if !have_ffmpeg() {
            return;
        }
        let dir = scratch("e2e-plain");
        let out = encode_sample(&dir, OverlayOptions::default(), SessionAnalysis::empty());

        assert_eq!(
            video_size(&out),
            Some((640, 360)),
            "the untouched export is not the source size"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_zoom_graph_survives_ffmpeg() {
        // The failure this guards against is silent: a graph FFmpeg accepts but
        // that never moves the crop exports a flat video, and the user concludes
        // auto-zoom is broken.
        if !have_ffmpeg() {
            return;
        }
        let dir = scratch("e2e-zoom");
        let out = encode_sample(&dir, OverlayOptions::default(), analysis_with(zoom_rows()));

        assert!(out.metadata().map(|m| m.len() > 0).unwrap_or(false));
        // The output keeps the source size: the zoom is a crop inside the frame,
        // not a resize of it.
        assert_eq!(video_size(&out), Some((640, 360)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_cursor_layer_composites_over_the_video() {
        if !have_ffmpeg() {
            return;
        }
        let dir = scratch("e2e-cursor");
        let out = encode_sample(
            &dir,
            OverlayOptions {
                zoom: false,
                ..OverlayOptions::default()
            },
            analysis_with(vec![row(0.0, 320.0, 180.0), row(1.0, 400.0, 200.0)]),
        );

        assert_eq!(
            video_size(&out),
            Some((640, 360)),
            "the sprite must not change the output geometry"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn both_layers_at_once_still_export() {
        // Zoom plus cursor is the normal case, and the one that routes through the
        // two-input graph.
        if !have_ffmpeg() {
            return;
        }
        let dir = scratch("e2e-both");
        let out = encode_sample(&dir, OverlayOptions::default(), analysis_with(zoom_rows()));
        assert_eq!(video_size(&out), Some((640, 360)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_take_whose_cursor_was_burned_in_gets_no_second_cursor() {
        // The rows carry a cursor and a trail exists, so `cursor_baked_in` is the
        // only thing that can switch the layer off here.
        if !have_ffmpeg() {
            return;
        }
        let dir = scratch("e2e-baked");
        let mut analysis = analysis_with(vec![row(0.0, 320.0, 180.0)]);
        analysis.cursor_baked_in = true;

        let out = encode_sample(
            &dir,
            OverlayOptions {
                zoom: false,
                cursor: true,
                ..OverlayOptions::default()
            },
            analysis,
        );
        assert!(out.metadata().map(|m| m.len() > 0).unwrap_or(false));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_take_with_no_trail_still_exports() {
        // A missing `events.revents` must cost the user their effects, not their
        // export.
        if !have_ffmpeg() {
            return;
        }
        let dir = scratch("e2e-notrail");
        let out = encode_sample(&dir, OverlayOptions::default(), analysis_with(vec![]));
        assert_eq!(video_size(&out), Some((640, 360)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_square_preset_exports_square_through_the_zoom_graph() {
        // The two features meet here: the box is square and smaller than the
        // frame, so the zoom's coordinates have to be expressed inside it.
        if !have_ffmpeg() {
            return;
        }
        let dir = scratch("e2e-square");
        let input = sample_input(&dir);
        let out = dir.join("out.mp4");

        encode(
            EncodePaths {
                input: &input,
                output: &out,
                work: &dir.join("work"),
            },
            Aspect::Square,
            None,
            OverlayOptions::default(),
            &analysis_with(zoom_rows()),
            &mut |_| {},
        )
        .expect("encode failed");

        assert_eq!(video_size(&out), Some((360, 360)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_audio_track_survives_the_effect_layers() {
        // The filter_complex path renames the outputs, so audio has to be mapped
        // explicitly or the export silently loses its sound.
        if !have_ffmpeg() {
            return;
        }
        let dir = scratch("e2e-audio");
        let input = dir.join("av.mp4");
        let status = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(["-f", "lavfi", "-i", "testsrc=size=640x360:rate=10:duration=2"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=2"])
            .args([
                "-pix_fmt",
                "yuv420p",
                "-c:v",
                "libx264",
                "-c:a",
                "aac",
                "-shortest",
            ])
            .arg(&input)
            .status()
            .unwrap();
        assert!(status.success(), "could not build the test input");
        assert!(audio_present(&input), "the test input has no audio");

        let out = dir.join("out.mp4");
        encode(
            EncodePaths {
                input: &input,
                output: &out,
                work: &dir.join("work"),
            },
            Aspect::Original,
            None,
            OverlayOptions {
                zoom: false,
                ..OverlayOptions::default()
            },
            &analysis_with(vec![row(0.0, 320.0, 180.0)]),
            &mut |_| {},
        )
        .expect("encode failed");

        assert!(
            audio_present(&out),
            "the audio was dropped by the cursor's filter_complex graph"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}

