//! Audio mixing and muxing.
//!
//! During a recording we keep three separate files — `raw.mp4` (video, no
//! audio), `mic.wav` (cpal) and `system.wav` (BlackHole via FFmpeg). Keeping
//! them separate means a failed mic never corrupts the video, and re-mixing
//! later is cheap.
//!
//! At export time FFmpeg muxes them back together with `amix`, stream-copying
//! the video so this step is fast.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, Context, Result};

/// Mux `video` with one or more audio tracks into `output`, mixing the tracks
/// together with FFmpeg's `amix` filter. Video is stream-copied, so this only
/// re-encodes audio.
///
/// Returns the video path unchanged when there is nothing to mix.
pub fn mux_tracks(video: &Path, audio: &[&Path], output: &Path) -> Result<PathBuf> {
    if audio.is_empty() {
        return Ok(video.to_path_buf());
    }
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "warning", "-y"]);
    cmd.arg("-i").arg(video);
    for track in audio {
        cmd.arg("-i").arg(track);
    }

    if audio.len() == 1 {
        cmd.args(["-map", "0:v:0", "-map", "1:a:0"]);
    } else {
        // Inputs 1..=n are the audio tracks, in the order they were passed.
        let labels: String = (1..=audio.len()).map(|i| format!("[{i}:a]")).collect();
        let filter = format!(
            "{labels}amix=inputs={}:duration=longest:normalize=0[aout]",
            audio.len()
        );
        cmd.args(["-filter_complex", &filter]);
        cmd.args(["-map", "0:v:0", "-map", "[aout]"]);
    }

    cmd.args(["-c:v", "copy", "-c:a", "aac", "-b:a", "192k"]);
    cmd.args(["-movflags", "+faststart"]);
    cmd.arg(output.to_str().context("non-utf8 mux output path")?);

    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::inherit());

    eprintln!("[audio] muxing {} track(s) → {}", audio.len(), output.display());
    let status = cmd.status().context("failed to run ffmpeg for muxing")?;
    if !status.success() {
        return Err(anyhow!("ffmpeg mux exited with {status}"));
    }
    Ok(output.to_path_buf())
}

