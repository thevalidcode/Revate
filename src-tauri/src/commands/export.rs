//! Export-related Tauri commands.
//! Exposes video export and encoding functions to the frontend.
//! TODO: Implement export command handlers.

#[tauri::command]
pub async fn export_video(
    project_path: String,
    output_path: String,
    options: ExportOptions,
) -> Result<ExportResult, String> {
    todo!("Implement export video command")
}

#[tauri::command]
pub async fn cancel_export() -> Result<(), String> {
    todo!("Implement cancel export command")
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExportOptions {
    pub codec: String,
    pub bitrate: u32,
    pub preset: String,
    pub crf: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExportResult {
    pub output_path: String,
    pub file_size: u64,
    pub duration: f64,
}
