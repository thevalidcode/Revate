//! Editor window lifecycle and session introspection.
//!
//! When a recording stops, `open_editor` closes the recorder window and opens a
//! second window on `index.html?editor=1&session=<id>`. `App.tsx` reads that
//! query string and renders the editor page instead of the router.
//!
//! A "session" is just the folder name under `projects/` (`rec-<secs>-<millis>`)
//! that `paths::new_project_dir` creates, which keeps absolute paths out of URLs.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;
use tauri::{AppHandle, Manager, TitleBarStyle, WebviewUrl, WebviewWindowBuilder};

use crate::utils::{ffprobe, paths};

/// Emitted on the editor window while an export runs: `{ percent, phase }`.
pub const EXPORT_PROGRESS_EVENT: &str = "export-progress";

const EDITOR_LABEL: &str = "editor";
const MAIN_LABEL: &str = "main";

/// Everything the editor needs to describe the recording it is editing.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    pub project_dir: String,
    /// `final.mp4` when the audio mux succeeded, otherwise `raw.mp4`.
    pub video_path: String,
    pub thumb_path: Option<String>,
    pub width: u32,
    pub height: u32,
    pub duration_ms: u64,
    pub has_audio: bool,
    pub suggested_name: String,
    /// Where exports land unless the user picks somewhere else.
    pub default_folder: String,
}

/// Resolve `<projects>/<id>` and make sure it exists.
///
/// The id is restricted to `[A-Za-z0-9_-]` because it is both joined onto a
/// path *and* interpolated into the editor window's URL.
pub fn session_dir(app: &AppHandle, session_id: &str) -> Result<PathBuf, String> {
    let safe = !session_id.is_empty()
        && session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !safe {
        return Err("invalid session id".into());
    }

    let dir = paths::projects_dir(app)
        .map_err(|e| e.to_string())?
        .join(session_id);
    if !dir.is_dir() {
        return Err(format!("unknown session `{session_id}`"));
    }
    Ok(dir)
}

/// The playable video inside a session folder.
pub fn video_in(dir: &Path) -> Result<PathBuf, String> {
    let final_mp4 = dir.join("final.mp4");
    if final_mp4.exists() {
        return Ok(final_mp4);
    }
    let raw = dir.join("raw.mp4");
    if raw.exists() {
        return Ok(raw);
    }
    Err(format!("no video found in {}", dir.display()))
}

#[tauri::command]
pub async fn session_info(app: AppHandle, session_id: String) -> Result<SessionInfo, String> {
    let dir = session_dir(&app, &session_id)?;
    let video = video_in(&dir)?;
    let info = ffprobe::probe(&video).map_err(|e| e.to_string())?;

    let thumb = dir.join("thumb.jpg");
    Ok(SessionInfo {
        id: session_id,
        project_dir: dir.to_string_lossy().into_owned(),
        video_path: video.to_string_lossy().into_owned(),
        thumb_path: thumb.exists().then(|| thumb.to_string_lossy().into_owned()),
        width: info.width,
        height: info.height,
        duration_ms: info.duration_ms,
        has_audio: info.has_audio,
        suggested_name: "revate-clip".into(),
        default_folder: app
            .path()
            .video_dir()
            .unwrap_or_else(|_| dir.clone())
            .to_string_lossy()
            .into_owned(),
    })
}

/// Pull a single frame out of the recording for the preview poster.
#[tauri::command]
pub async fn make_thumbnail(app: AppHandle, session_id: String) -> Result<String, String> {
    let dir = session_dir(&app, &session_id)?;
    let video = video_in(&dir)?;
    let thumb = dir.join("thumb.jpg");

    let already_there = thumb.metadata().map(|m| m.len() > 0).unwrap_or(false);
    if !already_there {
        let status = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-ss", "0.2", "-i"])
            .arg(&video)
            .args(["-frames:v", "1", "-vf", "scale=640:-2", "-q:v", "4"])
            .arg(&thumb)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| format!("failed to run ffmpeg for the thumbnail: {e}"))?;
        if !status.success() {
            // Not fatal: the editor falls back to the video's own first frame.
            return Ok(String::new());
        }
    }

    Ok(thumb.to_string_lossy().into_owned())
}

/// Hand the recording over to a fresh editor window and retire the recorder.
#[tauri::command]
pub async fn open_editor(app: AppHandle, session_id: String) -> Result<(), String> {
    let dir = session_dir(&app, &session_id)?;
    video_in(&dir)?;

    // Only ever one editor.
    if let Some(existing) = app.get_webview_window(EDITOR_LABEL) {
        let _ = existing.close();
    }

    let url = WebviewUrl::App(format!("index.html?editor=1&session={session_id}").into());
    let window = WebviewWindowBuilder::new(&app, EDITOR_LABEL, url)
        .title("Revate — Editor")
        .inner_size(1000.0, 720.0)
        .min_inner_size(880.0, 620.0)
        .resizable(true)
        .center()
        .title_bar_style(TitleBarStyle::Overlay)
        .hidden_title(true)
        .build()
        .map_err(|e| e.to_string())?;

    // The editor is built and focusable before the recorder goes away, so the
    // app never has a moment with no window. `lib.rs` owns the "quit when the
    // last window closes" rule, which keeps both handoffs seamless.
    if let Some(main) = app.get_webview_window(MAIN_LABEL) {
        let _ = main.close();
    }
    let _ = window.set_focus();

    Ok(())
}

/// Go back to the recorder from the editor.
#[tauri::command]
pub async fn new_recording(app: AppHandle) -> Result<(), String> {
    if app.get_webview_window(MAIN_LABEL).is_none() {
        WebviewWindowBuilder::new(&app, MAIN_LABEL, WebviewUrl::App("index.html".into()))
            .title("Revate")
            .inner_size(720.0, 880.0)
            .min_inner_size(720.0, 620.0)
            .resizable(true)
            .center()
            .title_bar_style(TitleBarStyle::Overlay)
            .hidden_title(true)
            .build()
            .map_err(|e| e.to_string())?;
    }
    if let Some(editor) = app.get_webview_window(EDITOR_LABEL) {
        let _ = editor.close();
    }
    Ok(())
}