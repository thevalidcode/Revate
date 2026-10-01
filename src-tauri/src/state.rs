use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use anyhow::Result;

use crate::audio::mic::MicRecording;
use crate::audio::system::SystemAudioRecording;

/// A live recording: the video process plus any audio capture handles.
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
}

#[derive(Default)]
pub struct RecordingState {
    pub session: Mutex<Option<RecordingSession>>,
    /// Labels of the per-monitor overlay windows, in display order.
    pub picker_windows: Mutex<Vec<String>>,
    /// Index (into `available_monitors`) of the display the user picked.
    pub selected_display: Mutex<Option<usize>>,
}
