use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::audio::{mic, mixer, system};
use crate::capture::screen::{find_screen_index_for, list_avfoundation_devices, record_loop, reap_stray_captures, CaptureConfig, Region};
use crate::commands::displays::list_display_rects;
use crate::input::tracker::Tracker;
use crate::state::{RecordingSession, RecordingState, StopResult};
use crate::utils::capture_meta::{write_capture_meta, CaptureMeta, PixelRect};
use crate::utils::clock::{Clock, SharedClock};
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

    // A new take must not inherit the previous one's stop result, or a later
    // stop could hand back a session id that is already in the editor.
    state.stopping.store(false, Ordering::SeqCst);
    *state.last_stop.lock().unwrap() = None;

    let project_dir = paths::new_project_dir(&app).map_err(|e| e.to_string())?;
    let video_path = project_dir.join("raw.mp4");
    let mic_path = project_dir.join("mic.wav");
    let system_path = project_dir.join("system.wav");

    let screen_index =
        find_screen_index_for(display_index.unwrap_or(0)).map_err(|e| e.to_string())?;
    let fps = fps.unwrap_or(30).clamp(1, 60);

    // An earlier take that was interrupted (crash, force-quit, dev rebuild) may
    // still have a capture process holding the avfoundation device, which would
    // make this recording fail or come out empty. Clear it before we spawn.
    reap_stray_captures(&paths::projects_dir(&app).map_err(|e| e.to_string())?);

    // Geometry of the monitor being recorded, in physical pixels. Needed for
    // `capture.json`, the only place the screen → video conversion lives, and
    // for the zoom/cursor pass in the editor.
    let displays = list_display_rects(app.clone()).await?;
    let display = displays
        .get(display_index.unwrap_or(0) as usize)
        .cloned()
        .ok_or_else(|| "unknown display was selected".to_string())?;
    // Cursor coordinates are global *logical points* whose origin is the primary
    // monitor's top-left, so it is the primary's scale factor that converts them
    // — not the captured monitor's own. On a mixed-DPI desk those differ.
    let primary_scale = displays
        .iter()
        .find(|d| d.is_primary)
        .map(|d| d.scale_factor)
        .unwrap_or(display.scale_factor)
        .max(1.0);

    // One time base for the whole take, taken before any thread exists: video,
    // cursor trail and audio all stamp against it, which is what makes them line
    // up in the editor.
    let clock: SharedClock = Arc::new(Clock::start());

    // Start the input tracker *before* FFmpeg, because its result decides
    // whether FFmpeg hides the system cursor. When we can draw our own, the
    // baked-in one is switched off so the two never appear on top of each other;
    // without Accessibility permission we keep FFmpeg's cursor and record that
    // fact in `capture.json`.
    let wants_own_cursor = capture_cursor.unwrap_or(true);
    let tracker = if wants_own_cursor {
        match Tracker::start(&project_dir, clock.clone()) {
            Ok(tracker) => Some(tracker),
            Err(e) => {
                eprintln!("[recording] {e}");
                None
            }
        }
    } else {
        None
    };
    let cursor_baked_in = tracker.is_none();

    let cfg = CaptureConfig {
        output: video_path.clone(),
        screen_index,
        // Audio is captured separately — keep the video process silent.
        mic_index: None,
        fps,
        capture_cursor: cursor_baked_in,
        bitrate: 8_000_000,
        region: region.map(|r| Region {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        }),
    };

    // Written before the take starts, while the geometry is still known. Best
    // effort: a take without it simply gets no cursor work later.
    let _ = write_capture_meta(
        &project_dir,
        &CaptureMeta {
            display: PixelRect::new(display.x, display.y, display.width, display.height),
            primary_scale_factor: primary_scale,
            region: region.map(|r| PixelRect::new(r.x, r.y, r.width, r.height)),
            fps,
            cursor_baked_in,
        },
    );

    let video_stop = Arc::new(AtomicBool::new(false));
    let video_flag = video_stop.clone();
    let video_thread = std::thread::spawn(move || record_loop(cfg, video_flag));

    // If an audio device fails to open, unwind whatever already started so we
    // never leave a half-live session behind.
    let mic_recording = match mic {
        Some(name) => match mic::start_microphone_capture(Some(name), mic_path.clone()) {
            Ok(rec) => Some(rec),
            Err(e) => {
                stop_partial(video_stop, video_thread, tracker, None);
                return Err(format!("failed to start microphone capture: {e}"));
            }
        },
        None => None,
    };

    let system_recording = match system_audio {
        Some(name) => match system::start_system_audio_capture(&name, system_path.clone()) {
            Ok(rec) => Some(rec),
            Err(e) => {
                stop_partial(video_stop, video_thread, tracker, mic_recording);
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
        tracker,
    };

    *state.session.lock().unwrap() = Some(session);

    Ok(project_dir.to_string_lossy().into_owned())
}

/// Unwind a partially started take: stop the video, close the cursor trail and
/// release any audio device that was already opened.
fn stop_partial(
    video_stop: Arc<AtomicBool>,
    video_thread: std::thread::JoinHandle<anyhow::Result<PathBuf>>,
    tracker: Option<Tracker>,
    mic: Option<mic::MicRecording>,
) {
    video_stop.store(true, Ordering::SeqCst);
    let _ = video_thread.join();
    if let Some(tracker) = tracker {
        let _ = tracker.stop();
    }
    if let Some(rec) = mic {
        let _ = rec.stop();
    }
}

/// Stop the current recording, join every capture thread and mux the audio
/// tracks back onto the video.
///
/// Safe to call more than once. Stopping is not instantaneous — FFmpeg has to
/// finalize the MP4 and the audio has to be muxed — so a second click (or a
/// second recorder window) can arrive while the first call is still working. That
/// caller waits for the in-flight stop and receives the same answer instead of
/// being told "not recording" while a take is still open.
#[tauri::command]
pub async fn stop_recording(state: State<'_, RecordingState>) -> Result<StopResult, String> {
    // Claim the session. Only one caller can win this; the rest fall through to
    // awaiting the result.
    let session = { state.session.lock().unwrap().take() };

    let Some(session) = session else {
        return await_in_flight_stop(&state).await;
    };

    state.stopping.store(true, Ordering::SeqCst);

    let outcome = tokio::task::spawn_blocking(move || finish_session(session))
        .await
        .map_err(|e| format!("join task failed: {e}"))
        .and_then(|inner| inner.map_err(|e| e.to_string()));

    // `finish_session` always returns a file inside the session folder
    // (`raw.mp4` or the muxed `final.mp4`).
    let result = match outcome {
        Ok(finished) => {
            let video = finished.video;
            let project_dir = video.parent().unwrap_or(Path::new("")).to_path_buf();
            let id = project_dir
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();

            Ok(StopResult {
                id,
                project_dir: project_dir.to_string_lossy().into_owned(),
                video_path: video.to_string_lossy().into_owned(),
                events_recorded: finished.events_recorded,
            })
        }
        Err(message) => Err(message),
    };

    // Publish *before* clearing `stopping`, so a caller arriving in between sees
    // the finished answer rather than concluding nothing was recording.
    *state.last_stop.lock().unwrap() = Some(result.clone());
    state.stopping.store(false, Ordering::SeqCst);
    state.stop_notify.notify_waiters();

    result
}

/// Wait for a stop another caller is already running, and return its outcome.
///
/// Only reports "not recording" when there is genuinely nothing running, which
/// is the one case where that message is accurate.
async fn await_in_flight_stop(state: &RecordingState) -> Result<StopResult, String> {
    // A stop that finished a moment ago still answers this click.
    if !state.stopping.load(Ordering::SeqCst) {
        return match state.last_stop.lock().unwrap().clone() {
            Some(result) => result,
            None => Err("not recording".to_string()),
        };
    }

    // Generous: a long take on a slow disk can take a while to finalize + mux.
    let deadline = std::time::Instant::now() + Duration::from_secs(300);
    loop {
        // Arm the notification *before* re-reading, so a stop completing between
        // the read and the wait cannot be missed.
        let notified = state.stop_notify.notified();
        if let Some(result) = state.last_stop.lock().unwrap().clone() {
            return result;
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err("timed out waiting for the recording to stop".into());
        }
        let _ = tokio::time::timeout(remaining, notified).await;
    }
}

#[tauri::command]
pub async fn is_recording(state: State<'_, RecordingState>) -> Result<bool, String> {
    // Still "recording" while the mux runs, so a UI polling this does not think
    // the take ended before the file is actually ready.
    Ok(state.session.lock().unwrap().is_some() || state.stopping.load(Ordering::SeqCst))
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

/// What a finished take left on disk.
struct FinishedTake {
    /// `raw.mp4`, or `final.mp4` when the audio muxed in.
    video: PathBuf,
    /// Events written to `events.revents`.
    events_recorded: u64,
}

/// Blocking teardown: stop video, close the cursor trail, stop audio, then mux.
/// Runs in a blocking task so the async runtime is never held up by `join()`.
fn finish_session(session: RecordingSession) -> anyhow::Result<FinishedTake> {
    session.video_stop.store(true, Ordering::SeqCst);
    let video = session
        .video_thread
        .join()
        .map_err(|e| anyhow::anyhow!("video thread panicked: {e:?}"))??;

    // Close the trail first, so a take always has its cursor data flushed even
    // if the mux below goes wrong.
    let events_recorded = match session.tracker {
        Some(tracker) => match tracker.stop() {
            Ok(written) => written,
            Err(e) => {
                eprintln!("[recording] closing the cursor trail failed: {e}");
                0
            }
        },
        None => 0,
    };

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
        return Ok(FinishedTake {
            video,
            events_recorded,
        });
    }

    let refs: Vec<&Path> = tracks.iter().map(|p| p.as_path()).collect();
    let output = session.project_dir.join("final.mp4");
    let video = match mixer::mux_tracks(&video, &refs, &output) {
        Ok(path) => path,
        Err(e) => {
            // Keep the separate files around rather than losing the take.
            eprintln!("[recording] mux failed, keeping raw tracks: {e}");
            video
        }
    };

    Ok(FinishedTake {
        video,
        events_recorded,
    })
}

