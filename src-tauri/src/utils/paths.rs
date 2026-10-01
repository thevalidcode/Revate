use anyhow::{Context, Result};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

pub fn projects_dir(app: &AppHandle) -> Result<PathBuf> {
    let base = app
        .path()
        .app_data_dir()
        .context("could not resolve app data dir")?;
    let dir = base.join("projects");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn new_project_dir(app: &AppHandle) -> Result<PathBuf> {
    let id = format!("rec-{}", timestamp());
    let dir = projects_dir(app)?.join(id);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// `YYYYMMDD-HHMMSS`-ish, but derived from the system clock so we avoid pulling
/// in a timezone database. Millisecond precision keeps back-to-back recordings
/// from colliding on the same folder name.
fn timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    let millis = duration.subsec_millis();
    format!("{secs}-{millis:03}")
}