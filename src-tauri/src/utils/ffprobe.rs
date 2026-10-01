//! Minimal `ffprobe` wrapper — just enough metadata for the editor preview
//! and for turning FFmpeg's `-progress` output into a percentage.

use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

/// The handful of facts the editor needs about a capture.
#[derive(Debug, Clone, Copy)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub duration_ms: u64,
    pub has_audio: bool,
}

#[derive(Deserialize)]
struct ProbeOutput {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    #[serde(default)]
    format: Option<ProbeFormat>,
}

#[derive(Deserialize)]
struct ProbeStream {
    codec_type: String,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
}

#[derive(Deserialize)]
struct ProbeFormat {
    #[serde(default)]
    duration: Option<String>,
}

/// Read stream/format metadata for `path`.
pub fn probe(path: &Path) -> Result<VideoInfo> {
    let output = Command::new("ffprobe")
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .output()
        .context("failed to run ffprobe — is it on PATH? try `brew install ffmpeg`")?;

    if !output.status.success() {
        return Err(anyhow!(
            "ffprobe failed on {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let parsed: ProbeOutput =
        serde_json::from_slice(&output.stdout).context("could not parse ffprobe output")?;

    let video = parsed
        .streams
        .iter()
        .find(|s| s.codec_type == "video")
        .ok_or_else(|| anyhow!("no video stream in {}", path.display()))?;

    // `format.duration` is seconds as a string; a missing value simply means
    // "unknown", which the caller treats as 0.
    let duration_ms = parsed
        .format
        .and_then(|f| f.duration)
        .and_then(|d| d.trim().parse::<f64>().ok())
        .map(|secs| (secs * 1000.0).round() as u64)
        .unwrap_or(0);

    Ok(VideoInfo {
        width: video.width.unwrap_or(0),
        height: video.height.unwrap_or(0),
        duration_ms,
        has_audio: parsed.streams.iter().any(|s| s.codec_type == "audio"),
    })
}