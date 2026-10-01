//! Screen capture via FFmpeg's avfoundation input (macOS).
//!
//! We shell out to `ffmpeg` and let it capture + encode in one process.
//! To stop, we send 'q' to ffmpeg's stdin so it finalizes the MP4 cleanly.
//! On Windows this module would use `gdigrab`, on Linux `x11grab` — the
//! command construction is the only part that changes.

use anyhow::{anyhow, Context, Result};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// A rectangular region (in captured-frame pixels) to crop to.
#[derive(Debug, Clone, Copy)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

pub struct CaptureConfig {
    pub output: PathBuf,
    /// avfoundation device index for the screen (see `list_avfoundation_devices`)
    pub screen_index: u32,
    /// avfoundation audio device index; None = no audio. Audio is captured
    /// separately (cpal + BlackHole) in `audio::*`, so this stays `None` for
    /// the video process.
    pub mic_index: Option<u32>,
    pub fps: u32,
    pub capture_cursor: bool,
    /// Video bitrate in bits per second (e.g. 8_000_000 for 8 Mbps)
    pub bitrate: u32,
    /// Optional crop applied with FFmpeg's `crop` filter.
    pub region: Option<Region>,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            output: PathBuf::new(),
            screen_index: 3,      // macOS default when 3 cameras are plugged in
            mic_index: None,
            fps: 30,
            capture_cursor: true,
            bitrate: 8_000_000,
            region: None,
        }
    }
}

/// Run `ffmpeg -list_devices` and return the raw stderr.
/// FFmpeg prints the device list to stderr, not stdout.
pub fn list_avfoundation_devices() -> Result<String> {
    let out = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-f", "avfoundation",
            "-list_devices", "true",
            "-i", "",
        ])
        .output()
        .context("failed to run ffmpeg — is it installed and on PATH? try `brew install ffmpeg`")?;

    // This call always "fails" with a non-zero exit because there's no input.
    // We only care about the stderr text.
    Ok(String::from_utf8_lossy(&out.stderr).into_owned())
}

/// Parse `list_avfoundation_devices` output and find the avfoundation index of
/// a given screen. `screen` is the screen ordinal (0 = primary).
pub fn find_screen_index_for(screen: u32) -> Result<u32> {
    let stderr = list_avfoundation_devices()?;
    let wanted = format!("Capture screen {screen}");
    let mut fallback: Option<u32> = None;

    for line in stderr.lines() {
        // Lines look like: "[AVFoundation indev @ 0x...] [3] Capture screen 0"
        let Some(after_bracket) = line.split("] [").nth(1) else {
            continue;
        };
        let Some(num_str) = after_bracket.split(']').next() else {
            continue;
        };
        let Ok(idx) = num_str.trim().parse::<u32>() else {
            continue;
        };

        if line.contains(&wanted) {
            return Ok(idx);
        }
        if line.contains("Capture screen") && fallback.is_none() {
            fallback = Some(idx);
        }
    }

    fallback.ok_or_else(|| anyhow!("no screen capture device found — is Screen Recording permission granted?"))
}

/// Blocking capture loop. Spawns ffmpeg, waits for `stop_flag`, then asks
/// ffmpeg to finalize the file gracefully.
pub fn record_loop(cfg: CaptureConfig, stop_flag: Arc<AtomicBool>) -> Result<PathBuf> {
    // Make sure the output directory exists
    if let Some(parent) = cfg.output.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let audio_input = match cfg.mic_index {
        Some(i) => i.to_string(),
        None => "none".to_string(),
    };
    let input = format!("{}:{}", cfg.screen_index, audio_input);

    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "info"]);
    cmd.args(["-f", "avfoundation"]);
    cmd.args([
        "-capture_cursor",
        if cfg.capture_cursor { "1" } else { "0" },
    ]);
    cmd.args(["-framerate", &cfg.fps.to_string()]);
    cmd.args(["-i", &input]);

    // Optional crop for "region" recordings. Coordinates are relative to the
    // captured screen's top-left corner.
    if let Some(r) = cfg.region {
        cmd.args([
            "-vf",
            &format!("crop={}:{}:{}:{}", r.width, r.height, r.x, r.y),
        ]);
    }

    // Hardware encoder on Apple Silicon. Note: modern ffmpeg dropped -q:v
    // for h264_videotoolbox, so we use -b:v.
    cmd.args(["-c:v", "h264_videotoolbox"]);
    cmd.args(["-b:v", &cfg.bitrate.to_string()]);
    cmd.args(["-realtime", "1"]);

    if cfg.mic_index.is_some() {
        cmd.args(["-c:a", "aac", "-b:a", "128k"]);
    } else {
        cmd.args(["-an"]);
    }

    cmd.args(["-pix_fmt", "yuv420p"]);
    cmd.args(["-movflags", "+faststart"]);
    cmd.args([
        "-y",
        cfg.output
            .to_str()
            .context("non-utf8 output path")?,
    ]);

    cmd.stdin(Stdio::piped());     // we write 'q' here to stop
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::inherit());  // ffmpeg progress prints to the dev terminal

    eprintln!(
        "[capture] spawning ffmpeg: input={input} output={}",
        cfg.output.display()
    );

    let mut child = cmd
        .spawn()
        .context("failed to spawn ffmpeg — is it on PATH? try `brew install ffmpeg`")?;

    // Poll the stop flag every 100ms. Cheap, simple, no async plumbing.
    while !stop_flag.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
    }

    // Graceful shutdown: send 'q' to ffmpeg. This makes it write the
    // moov atom and produce a valid MP4. SIGKILL here would corrupt it.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"q");
        let _ = stdin.flush();
    }

    let status = child.wait().context("ffmpeg wait failed")?;
    if !status.success() {
        return Err(anyhow!("ffmpeg exited with status: {status}"));
    }

    eprintln!("[capture] finalized {}", cfg.output.display());
    Ok(cfg.output)
}