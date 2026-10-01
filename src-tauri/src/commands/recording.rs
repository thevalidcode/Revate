use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::audio::{mic, mixer, system};
use crate::capture::screen::{find_screen_index_for, list_avfoundation_devices, record_loop, CaptureConfig, Region};
use crate::state::{RecordingSession, RecordingState};
use crate::utils::paths;

/// A selectable audio device. `id` and `name` are the same OS-provided string
/// today, but are kept separate so the frontend shape stays stable if we later
/// key on CoreAudio UIDs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
}

impl AudioDevice {
    fn from_names(names: Vec<String>) -> Vec<Self> {
        names
            .into_iter()
            .map(|name| Self {
                id: name.clone(),
                name,
            })
            .collect()
    }
}

/// Microphones available through cpal (CoreAudio on macOS).
#[tauri::command]
pub fn list_audio_inputs() -> Result<Vec<AudioDevice>, String> {
    mic::list_input_devices()
        .map(AudioDevice::from_names)
        .map_err(|e| e.to_string())
}

/// Loopback devices we can record with FFmpeg (BlackHole on macOS).
#[tauri::command]
pub fn list_system_audio_devices() -> Result<Vec<AudioDevice>, String> {
    system::list_system_audio_devices()
        .map(AudioDevice::from_names)
        .map_err(|e| e.to_string())
}

/// Raw avfoundation device dump — handy for the settings/debug screen.
#[tauri::command]
pub async fn list_capture_devices() -> Result<String, String> {
    list_avfoundation_devices().map_err(|e| e.to_string())
}

/// Crop rectangle for a "region" recording, relative to the captured screen.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionInput {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Start a new recording: one FFmpeg process for the screen, plus independent
/// audio captures for the microphone (cpal) and/or system audio (BlackHole via
/// a second FFmpeg process). Returns the project directory.
#[tauri::command]
// The argument count mirrors the `invoke` payload from the recorder UI one-for-one.
// Tauri maps JS object keys onto positional command parameters, so folding these
// into a request struct would change the IPC contract rather than tidy the code.
#[allow(clippy::too_many_arguments)]
pub async fn start_recording(
    app: AppHandle,
    state: State<'_, RecordingState>,
    display_index: Option<u32>,
    mic: Option<String>,
    system_audio: Option<String>,
    region: Option<RegionInput>,
    fps: Option<u32>,
    capture_cursor: Option<bool>,
) -> Result<String, String> {
    if state.session.lock().unwrap().is_some() {
        return Err("a recording is already in progress".into());
    }

    let project_dir = paths::new_project_dir(&app).map_err(|e| e.to_string())?;
    let video_path = project_dir.join("raw.mp4");
    let mic_path = project_dir.join("mic.wav");
    let system_path = project_dir.join("system.wav");

    let screen_index =
        find_screen_index_for(display_index.unwrap_or(0)).map_err(|e| e.to_string())?;
    let fps = fps.unwrap_or(30).clamp(1, 60);

    let cfg = CaptureConfig {
        output: video_path.clone(),
        screen_index,
        // Audio is captured separately — keep the video process silent.
        mic_index: None,
        fps,
        capture_cursor: capture_cursor.unwrap_or(true),
        bitrate: 8_000_000,
        region: region.map(|r| Region {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        }),
    };

    let video_stop = Arc::new(AtomicBool::new(false));
    let video_flag = video_stop.clone();
    let video_thread = std::thread::spawn(move || record_loop(cfg, video_flag));

    // If an audio device fails to open, unwind whatever already started so we
    // never leave a half-live session behind.
    let mic_recording = match mic {
        Some(name) => match mic::start_microphone_capture(Some(name), mic_path.clone()) {
            Ok(rec) => Some(rec),
            Err(e) => {
                video_stop.store(true, Ordering::SeqCst);
                let _ = video_thread.join();
                return Err(format!("failed to start microphone capture: {e}"));
            }
        },
        None => None,
    };

    let system_recording = match system_audio {
        Some(name) => match system::start_system_audio_capture(&name, system_path.clone()) {
            Ok(rec) => Some(rec),
            Err(e) => {
                video_stop.store(true, Ordering::SeqCst);
                let _ = video_thread.join();
                if let Some(rec) = mic_recording {
                    let _ = rec.stop();
                }
                return Err(format!("failed to start system audio capture: {e}"));
            }
        },
        None => None,
    };

    let session = RecordingSession {
        project_dir: project_dir.clone(),
        video_path,
        video_stop,
        video_thread,
        mic: mic_recording,
        system: system_recording,
    };

    *state.session.lock().unwrap() = Some(session);

    Ok(project_dir.to_string_lossy().into_owned())
}

/// What `stop_recording` hands back: enough for the frontend to open the right
/// editor session without putting an absolute path in a URL.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopResult {
    /// Folder name under `projects/` — the editor's `?session=` id.
    pub id: String,
    pub project_dir: String,
    pub video_path: String,
}

/// Stop the current recording, join every capture thread and mux the audio
/// tracks back onto the video.
#[tauri::command]
pub async fn stop_recording(state: State<'_, RecordingState>) -> Result<StopResult, String> {
    let session = { state.session.lock().unwrap().take() };
    let session = session.ok_or_else(|| "not recording".to_string())?;

    let video = tokio::task::spawn_blocking(move || finish_session(session))
        .await
        .map_err(|e| format!("join task failed: {e}"))?
        .map_err(|e| e.to_string())?;

    // `finish_session` always returns a file inside the session folder
    // (`raw.mp4` or the muxed `final.mp4`).
    let project_dir = video.parent().unwrap_or(Path::new("")).to_path_buf();
    let id = project_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();

    Ok(StopResult {
        id,
        project_dir: project_dir.to_string_lossy().into_owned(),
        video_path: video.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub async fn is_recording(state: State<'_, RecordingState>) -> Result<bool, String> {
    Ok(state.session.lock().unwrap().is_some())
}

/// Re-mux an existing project folder (`raw.mp4` plus optional `mic.wav` /
/// `system.wav`) into `final.mp4`. Useful when the automatic mux is skipped.
#[tauri::command]
pub async fn mux_recording(project_dir: String) -> Result<String, String> {
    let dir = PathBuf::from(project_dir);
    let video = dir.join("raw.mp4");
    if !video.exists() {
        return Err(format!("no raw.mp4 found in {}", dir.display()));
    }

    tokio::task::spawn_blocking(move || -> anyhow::Result<PathBuf> {
        let tracks = audio_tracks_in(&dir);
        let refs: Vec<&Path> = tracks.iter().map(|p| p.as_path()).collect();
        mixer::mux_tracks(&video, &refs, &dir.join("final.mp4"))
    })
    .await
    .map_err(|e| format!("join task failed: {e}"))?
    .map(|path| path.to_string_lossy().into_owned())
    .map_err(|e| e.to_string())
}

/// The audio tracks that ended up on disk, in mic-then-system order.
fn audio_tracks_in(dir: &Path) -> Vec<PathBuf> {
    [dir.join("mic.wav"), dir.join("system.wav")]
        .into_iter()
        .filter(|path| path.exists())
        .collect()
}

/// Blocking teardown: stop video, stop audio, then mux. Runs in a blocking task
/// so the async runtime is never held up by `join()`.
fn finish_session(session: RecordingSession) -> anyhow::Result<PathBuf> {
    session.video_stop.store(true, Ordering::SeqCst);
    let video = session
        .video_thread
        .join()
        .map_err(|e| anyhow::anyhow!("video thread panicked: {e:?}"))??;

    let mut tracks: Vec<PathBuf> = Vec::new();
    if let Some(rec) = session.mic {

        match rec.stop() {
            Ok(path) => tracks.push(path),
            Err(e) => eprintln!("[recording] microphone stop failed: {e}"),
        }
    }

    if let Some(rec) = session.system {
        match rec.stop() {
            Ok(path) => tracks.push(path),
            Err(e) => eprintln!("[recording] system audio stop failed: {e}"),
        }
    }

    if tracks.is_empty() {
        return Ok(session.video_path);
    }

    let refs: Vec<&Path> = tracks.iter().map(|p| p.as_path()).collect();
    let output = session.project_dir.join("final.mp4");
    match mixer::mux_tracks(&video, &refs, &output) {
        Ok(path) => Ok(path),
        Err(e) => {
            // Keep the separate files around rather than losing the take.
            eprintln!("[recording] mux failed, keeping raw tracks: {e}");
            Ok(video)
        }
    }
}

