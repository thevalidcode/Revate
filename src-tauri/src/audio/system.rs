//! System audio capture (macOS).
//!
//! macOS exposes no built-in loopback device, so the recommended setup is
//! [BlackHole](https://existential.audio/blackhole/):
//!
//! ```sh
//! brew install --cask blackhole-2ch
//! ```
//!
//! Once installed, BlackHole shows up as an *avfoundation audio input* and we
//! can record it with a second FFmpeg process — the exact same pattern used for
//! the video (spawn ffmpeg, wait for the stop flag, then send `q` to stdin so
//! the WAV header is written). (Windows would use WASAPI loopback and Linux a
//! PulseAudio monitor source.)

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};

/// Substring used to recognize a loopback device in the avfoundation list.
const LOOPBACK_HINT: &str = "blackhole";

/// Handle to a running system-audio capture.
pub struct SystemAudioRecording {
    stop_flag: Arc<AtomicBool>,
    handle: JoinHandle<Result<PathBuf>>,
    pub device_name: String,
}

impl SystemAudioRecording {
    /// Signal FFmpeg to finalize the WAV and wait for it to exit.
    pub fn stop(self) -> Result<PathBuf> {
        self.stop_flag.store(true, Ordering::SeqCst);
        self.handle
            .join()
            .map_err(|e| anyhow!("system audio thread panicked: {e:?}"))?
    }
}

/// Names of the loopback devices we know how to capture (BlackHole et al).
pub fn list_system_audio_devices() -> Result<Vec<String>> {
    let stderr = crate::capture::screen::list_avfoundation_devices()?;
    Ok(parse_avfoundation_audio_devices(&stderr)
        .into_iter()
        .filter(|(_, name)| name.to_lowercase().contains(LOOPBACK_HINT))
        .map(|(_, name)| name)
        .collect())
}

/// Parse `ffmpeg -f avfoundation -list_devices` output and return the index of
/// the first audio device whose name contains `needle` (case-insensitive).
pub fn find_audio_device_index(needle: &str) -> Result<u32> {
    let stderr = crate::capture::screen::list_avfoundation_devices()?;
    let needle = needle.to_lowercase();
    let devices = parse_avfoundation_audio_devices(&stderr);
    devices
        .iter()
        .find(|(_, name)| name.to_lowercase().contains(&needle))
        .map(|(idx, _)| *idx)
        .ok_or_else(|| {
            anyhow!(
                "system audio device `{needle}` not found — install BlackHole with \
                 `brew install --cask blackhole-2ch` and grant microphone permission"
            )
        })
}

/// Devices listed under the `AVFoundation audio devices:` header.
/// Entries look like `[AVFoundation indev @ 0x…] [1] BlackHole 2ch`.
fn parse_avfoundation_audio_devices(stderr: &str) -> Vec<(u32, String)> {
    let mut devices = Vec::new();
    let mut in_audio = false;
    for line in stderr.lines() {
        if line.contains("AVFoundation audio devices") {
            in_audio = true;
            continue;
        }
        if line.contains("AVFoundation video devices") {
            in_audio = false;
            continue;
        }
        if !in_audio {
            continue;
        }
        let Some(after_bracket) = line.split("] [").nth(1) else {
            continue;
        };
        let Some(num) = after_bracket.split(']').next() else {
            continue;
        };
        let Ok(index) = num.trim().parse::<u32>() else {
            continue;
        };
        let name = line.rsplit("] ").next().unwrap_or("").trim().to_string();
        if !name.is_empty() {
            devices.push((index, name));
        }
    }
    devices
}

/// Record `device_name` (e.g. "BlackHole 2ch") to a float WAV at `output`.
/// Returns once FFmpeg is confirmed to be running.
pub fn start_system_audio_capture(
    device_name: &str,
    output: PathBuf,
) -> Result<SystemAudioRecording> {
    let index = find_audio_device_index(device_name)?;
    let stop_flag = Arc::new(AtomicBool::new(false));
    let (init_tx, init_rx) = std::sync::mpsc::channel::<Result<(), String>>();

    let flag = stop_flag.clone();
    let out = output.clone();
    let handle = std::thread::spawn(move || capture_thread(index, out, flag, &init_tx));

    match init_rx.recv() {
        Ok(Ok(())) => {}
        Ok(Err(msg)) => {
            let _ = handle.join();
            return Err(anyhow!(msg));
        }
        Err(_) => return Err(anyhow!("system audio thread exited before ffmpeg started")),
    }

    Ok(SystemAudioRecording {
        stop_flag,
        handle,
        device_name: device_name.to_string(),
    })
}

fn capture_thread(
    index: u32,
    output: PathBuf,
    stop: Arc<AtomicBool>,
    init: &Sender<Result<(), String>>,
) -> Result<PathBuf> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let input = format!(":{index}");
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "warning"]);
    cmd.args(["-f", "avfoundation", "-i", &input]);
    // Float PCM keeps the capture lossless until the final amix encodes AAC.
    cmd.args(["-ac", "2", "-ar", "48000", "-c:a", "pcm_f32le"]);
    cmd.args([
        "-y",
        output.to_str().context("non-utf8 system audio path")?,
    ]);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::inherit());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("failed to spawn ffmpeg for system audio: {e}");
            let _ = init.send(Err(msg.clone()));
            return Err(anyhow!(msg));
        }
    };

    // If ffmpeg dies immediately the device index is wrong (or permission was
    // denied) — surface that instead of silently recording nothing.
    std::thread::sleep(Duration::from_millis(400));
    if let Ok(Some(status)) = child.try_wait() {
        let msg = format!(
            "ffmpeg exited immediately ({status}) — is `{index}` a valid avfoundation \
             audio device, and is microphone access granted?"
        );
        let _ = init.send(Err(msg.clone()));
        return Err(anyhow!(msg));
    }
    let _ = init.send(Ok(()));

    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
    }

    // Graceful shutdown, same as the video path.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"q");
        let _ = stdin.flush();
    }
    let status = child.wait().context("ffmpeg (system audio) wait failed")?;
    if !status.success() {
        return Err(anyhow!("ffmpeg (system audio) exited with {status}"));
    }

    Ok(output)
}

