//! Recording-related Tauri commands.
//! Exposes recording control functions to the frontend.
//! TODO: Implement recording command handlers.

#[tauri::command]
pub async fn start_recording(config: RecordingConfig) -> Result<(), String> {
    todo!("Implement start recording command")
}

#[tauri::command]
pub async fn stop_recording() -> Result<RecordingResult, String> {
    todo!("Implement stop recording command")
}

#[tauri::command]
pub async fn pause_recording() -> Result<(), String> {
    todo!("Implement pause recording command")
}

#[tauri::command]
pub async fn resume_recording() -> Result<(), String> {
    todo!("Implement resume recording command")
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecordingConfig {
    pub capture_mode: CaptureMode,
    pub audio_enabled: bool,
    pub microphone_enabled: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum CaptureMode {
    Fullscreen,
    Window { window_id: u64 },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecordingResult {
    pub file_path: String,
    pub duration: f64,
    pub events_file: String,
}
