//! Editing-related Tauri commands.
//! Exposes editing and preview functions to the frontend.
//! TODO: Implement editing command handlers.

#[tauri::command]
pub async fn load_project(path: String) -> Result<ProjectInfo, String> {
    todo!("Implement load project command")
}

#[tauri::command]
pub async fn get_preview_frame(time: f64) -> Result<FrameData, String> {
    todo!("Implement get preview frame command")
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProjectInfo {
    pub title: String,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub frame_rate: f64,
    pub has_zoom: bool,
    pub has_speed_ramping: bool,
    pub has_sfx: bool,
    pub has_chapters: bool,
    pub redaction_count: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FrameData {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub timestamp: f64,
}
