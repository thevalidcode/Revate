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

use crate::commands::editor::{session_dir, video_in, EXPORT_PROGRESS_EVENT};
use crate::utils::ffprobe;

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

    let folder = PathBuf::from(folder);
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("could not use {}: {e}", folder.display()))?;
    let output = folder.join(format!("{}.mp4", sanitize(&file_name)));

    tokio::task::spawn_blocking(move || run_export(&app, input, output, aspect, crop))
        .await
        .map_err(|e| format!("join task failed: {e}"))?
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|e| e.to_string())
}

fn run_export(
    app: &AppHandle,
    input: PathBuf,
    output: PathBuf,
    aspect: Aspect,
    crop: Option<NormalizedCrop>,
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
    let filter = crop
        .and_then(|rect| crop_filter_from_normalized(rect, info.width, info.height))
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
