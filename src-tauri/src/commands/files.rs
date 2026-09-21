//! File management Tauri commands.
//! Exposes file system operations to the frontend.
//! TODO: Implement file management command handlers.

#[tauri::command]
pub async fn select_recording_location() -> Result<String, String> {
    todo!("Implement select recording location command")
}

#[tauri::command]
pub async fn select_project_file() -> Result<String, String> {
    todo!("Implement select project file command")
}

#[tauri::command]
pub async fn save_project_file(path: String, data: String) -> Result<(), String> {
    todo!("Implement save project file command")
}

#[tauri::command]
pub async fn list_recent_projects() -> Result<Vec<RecentProject>, String> {
    todo!("Implement list recent projects command")
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecentProject {
    pub path: String,
    pub title: String,
    pub modified_at: String,
}
