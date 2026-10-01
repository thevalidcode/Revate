use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use anyhow::Result;

use crate::audio::mic::MicRecording;
use crate::audio::system::SystemAudioRecording;

/// A live recording: the video process plus any audio capture handles.
/// Everything needed to stop the recording and locate its artifacts.
pub struct RecordingSession {
    pub project_dir: PathBuf,
    pub video_path: PathBuf,
    /// 32-bit float WAV written by cpal, if the mic was enabled.
    pub mic_path: PathBuf,
    /// 32-bit float WAV written by the second FFmpeg process, if system audio
    /// was enabled.
    pub system_path: PathBuf,

    /// Signalled to make the video FFmpeg process finalize and exit.
    pub video_stop: Arc<AtomicBool>,
    pub video_thread: JoinHandle<Result<PathBuf>>,

    pub mic: Option<MicRecording>,
    pub system: Option<SystemAudioRecording>,

    pub started_at: Instant,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

#[derive(Default)]
pub struct RecordingState {
    pub session: Mutex<Option<RecordingSession>>,
    /// Labels of the per-monitor overlay windows, in display order.
    pub picker_windows: Mutex<Vec<String>>,
    /// Index (into `available_monitors`) of the display the user picked.
    pub selected_display: Mutex<Option<usize>>,
}
