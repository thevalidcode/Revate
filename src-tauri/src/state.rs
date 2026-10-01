use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use anyhow::Result;
use serde::Serialize;
use tokio::sync::Notify;

use crate::audio::mic::MicRecording;
use crate::audio::system::SystemAudioRecording;
use crate::input::tracker::Tracker;

/// A live recording: the video process plus any audio capture handles, and the
/// input tracker that writes the cursor trail.
/// Everything needed to stop the recording and locate its artifacts.
///
/// The audio files are discovered by
/// [`crate::commands::recording::finish_session`] from the paths returned by
/// each handle's `stop()`, so they are not stored here.
pub struct RecordingSession {
    pub project_dir: PathBuf,
    /// `raw.mp4`, and `final.mp4` once the audio has been muxed in.
    pub video_path: PathBuf,

    /// Signalled to make the video FFmpeg process finalize and exit.
    pub video_stop: Arc<AtomicBool>,
    pub video_thread: JoinHandle<Result<PathBuf>>,

    pub mic: Option<MicRecording>,
    pub system: Option<SystemAudioRecording>,

    /// `None` when Accessibility access was refused. In that case FFmpeg burned
    /// the system cursor into the video, and the editor must not draw a second
    /// one over it — `capture.json`'s `cursor_baked_in` records which happened.
    pub tracker: Option<Tracker>,
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
    /// Events written to `events.revents`; `0` when input tracking was refused.
    pub events_recorded: u64,
}

#[derive(Default)]
pub struct RecordingState {
    pub session: Mutex<Option<RecordingSession>>,
    /// Labels of the per-monitor overlay windows, in display order.
    pub picker_windows: Mutex<Vec<String>>,
    /// Index (into `available_monitors`) of the display the user picked.
    pub selected_display: Mutex<Option<usize>>,

    /// True from the moment `stop_recording` claims the session until the mux
    /// finishes.
    ///
    /// Stopping is not instant: FFmpeg has to finalize the MP4 and the audio has
    /// to be muxed, which takes seconds. The record button stays live during
    /// that window, so without this flag a second click would find an empty
    /// session slot and be told "not recording" while a take is still open.
    pub stopping: AtomicBool,

    /// Outcome of the stop now in flight (or the last one). A duplicate stop
    /// click awaits this instead of failing, so both callers get the same answer.
    pub last_stop: Mutex<Option<Result<StopResult, String>>>,
    /// Woken once [`Self::last_stop`] is published.
    pub stop_notify: Notify,
}
