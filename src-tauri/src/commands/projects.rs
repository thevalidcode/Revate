//! Projects window + library management.
//!
//! Every recording is a folder under `projects/` (`rec-<secs>-<millis>` by
//! default, renameable by the user). This module lists them, renames and
//! deletes them, and opens the standalone `projects` window that renders
//! `index.html?projects=1`.
//!
//! Folders without a playable video — an aborted take, or a session whose mux
//! left nothing behind — are skipped rather than shown as broken rows.

use std::path::Path;

use serde::Serialize;
use tauri::{AppHandle, Manager, TitleBarStyle, WebviewUrl, WebviewWindowBuilder};

use crate::commands::editor::{session_dir, spawn_editor, video_in};
use crate::utils::{ffprobe, paths};

const PROJECTS_LABEL: &str = "projects";

/// One row in the projects list.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    /// Folder name — the session id used by every other command.
    pub id: String,
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
    /// Absolute path of an already-extracted poster, if one exists.
    pub thumb_path: Option<String>,
    /// Folder mtime in epoch milliseconds, used for "newest first" ordering.
    pub modified_ms: u64,
}

/// Total bytes of every file directly inside the session folder.
fn folder_size(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| entry.metadata().ok())
                .filter(|meta| meta.is_file())
                .map(|meta| meta.len())
                .sum()
        })
        .unwrap_or(0)
}

/// Validate a user-supplied folder name. Rejects anything that could escape the
/// projects directory or upset the filesystem.
fn clean_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("name cannot be empty".into());
    }
    if trimmed.len() > 96 {
        return Err("name is too long".into());
    }
    if trimmed == "." || trimmed == ".." {
        return Err("invalid name".into());
    }
    if trimmed
        .chars()
        .any(|c| c == '/' || c == '\\' || c == ':' || c.is_control())
    {
        return Err("name cannot contain / \\ : or control characters".into());
    }
    Ok(trimmed.to_string())
}

/// Every session that actually has a video, newest first.
#[tauri::command]
pub async fn list_projects(app: AppHandle) -> Result<Vec<ProjectInfo>, String> {
    let root = paths::projects_dir(&app).map_err(|e| e.to_string())?;
    let entries = std::fs::read_dir(&root).map_err(|e| e.to_string())?;

    let mut projects: Vec<ProjectInfo> = entries
        .flatten()
        .filter_map(|entry| {
            let dir = entry.path();
            if !dir.is_dir() {
                return None;
            }
            // Skips aborted takes and any folder with no usable video.
            let video = video_in(&dir).ok()?;
            let id = entry.file_name().to_string_lossy().into_owned();
            let thumb = dir.join("thumb.jpg");
            let info = ffprobe::probe(&video).ok()?;

            Some(ProjectInfo {
                name: id.clone(),
                size_bytes: folder_size(&dir),
                duration_ms: info.duration_ms,
                width: info.width,
                height: info.height,
                thumb_path: thumb
                    .metadata()
                    .ok()
                    .filter(|m| m.len() > 0)
                    .map(|_| thumb.to_string_lossy().into_owned()),
                modified_ms: dir
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
                id,
                path: dir.to_string_lossy().into_owned(),
            })
        })
        .collect();

    projects.sort_by_key(|project| std::cmp::Reverse(project.modified_ms));
    Ok(projects)
}

/// Rename a session folder, returning the new name.
#[tauri::command]
pub async fn rename_project(
    app: AppHandle,
    id: String,
    new_name: String,
) -> Result<String, String> {
    let dir = session_dir(&app, &id)?;
    let name = clean_name(&new_name)?;

    let target = dir.parent().ok_or("invalid session path")?.join(&name);
    if target == dir {
        return Ok(name);
    }
    if target.exists() {
        return Err(format!("`{name}` already exists"));
    }

    std::fs::rename(&dir, &target).map_err(|e| format!("could not rename: {e}"))?;
    Ok(name)
}

/// Permanently delete a session folder and everything in it.
#[tauri::command]
pub async fn delete_project(app: AppHandle, id: String) -> Result<(), String> {
    let dir = session_dir(&app, &id)?;
    std::fs::remove_dir_all(&dir).map_err(|e| format!("could not delete: {e}"))
}

/// Open a project in the editor window and retire the projects window.
#[tauri::command]
pub async fn open_project_in_editor(app: AppHandle, id: String) -> Result<(), String> {
    if let Some(projects) = app.get_webview_window(PROJECTS_LABEL) {
        let _ = projects.close();
    }
    spawn_editor(&app, &id)
}

/// Open (or focus) the standalone projects window.
#[tauri::command]
pub async fn open_projects_window(app: AppHandle) -> Result<(), String> {
    // Just focus it if it's already up.
    if let Some(existing) = app.get_webview_window(PROJECTS_LABEL) {
        let _ = existing.set_focus();
        return Ok(());
    }

    let window = WebviewWindowBuilder::new(
        &app,
        PROJECTS_LABEL,
        WebviewUrl::App("index.html?projects=1".into()),
    )
    .title("Revate — Projects")
    .inner_size(900.0, 600.0)
    .min_inner_size(640.0, 420.0)
    .resizable(true)
    .center()
    .title_bar_style(TitleBarStyle::Overlay)
    .hidden_title(true)
    .build()
    .map_err(|e| e.to_string())?;

    let _ = window.set_focus();
    Ok(())
}
