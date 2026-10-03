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

use crate::effects;
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
///
/// Prefers the muxed `final.mp4`, but only when it is actually non-empty: a
/// failed audio mux leaves a 0-byte file behind, and handing that to the
/// player would show a broken clip for a perfectly good `raw.mp4`.
pub fn video_in(dir: &Path) -> Result<PathBuf, String> {
    let usable = |path: PathBuf| -> Option<PathBuf> {
        path.metadata().ok().filter(|m| m.len() > 0).map(|_| path)
    };

    usable(dir.join("final.mp4"))
        .or_else(|| usable(dir.join("raw.mp4")))
        .ok_or_else(|| format!("no playable video found in {}", dir.display()))
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

/// The cursor image as a `data:` URL, for the editor's preview.
///
/// The PNG is embedded in the binary, so the preview and the export draw the very
/// same pixels — and the same hotspot, which `effects` measures from the same
/// constants. A data URL rather than a file path because the webview's asset
/// protocol is scoped to the app's data directory, where this asset does not
/// live, and writing it out per session would leave litter behind.
#[tauri::command]
pub fn cursor_asset() -> String {
    format!("data:image/png;base64,{}", base64_encode(effects::CURSOR_PNG))
}

/// Standard base64, written out rather than pulled in as a dependency.
///
/// The input is the crate's own embedded constant, so this only ever runs over a
/// few kilobytes at editor start-up.
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(ALPHABET[(triple >> 18 & 0x3F) as usize] as char);
        out.push(ALPHABET[(triple >> 12 & 0x3F) as usize] as char);
        // The last group is padded rather than truncated, which is what a decoder
        // expects for 1 or 2 leftover bytes.
        out.push(if chunk.len() > 1 {
            ALPHABET[(triple >> 6 & 0x3F) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(triple & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Create (or focus) the editor window for `session_id`, retiring the recorder.
///
/// Shared by the recorder's "hand-off" and the projects window's
/// "Open in editor" so both produce an identical window.
pub fn spawn_editor(app: &AppHandle, session_id: &str) -> Result<(), String> {
    let dir = session_dir(app, session_id)?;
    video_in(&dir)?;

    // Only ever one editor.
    if let Some(existing) = app.get_webview_window(EDITOR_LABEL) {
        let _ = existing.close();
    }

    let url = WebviewUrl::App(format!("index.html?editor=1&session={session_id}").into());
    let window = WebviewWindowBuilder::new(app, EDITOR_LABEL, url)
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

/// Hand the recording over to a fresh editor window and retire the recorder.
#[tauri::command]
pub async fn open_editor(app: AppHandle, session_id: String) -> Result<(), String> {
    spawn_editor(&app, &session_id)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn base64_matches_the_standard_alphabet() {
        // "Man" -> "TWFu" is RFC 4648's worked example.
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
        assert_eq!(base64_encode(b""), "");
    }

    #[test]
    fn base64_pads_only_the_last_group() {
        assert_eq!(base64_encode(b"abc"), "YWJj");
        assert_eq!(base64_encode(b"abcd"), "YWJjZA==");
        assert_eq!(base64_encode(b"abcde"), "YWJjZGU=");
    }

    #[test]
    fn the_asset_is_a_png_data_url() {
        let url = cursor_asset();
        assert!(url.starts_with("data:image/png;base64,"));
        assert!(url.len() > 64, "suspiciously short: {url}");
    }

    /// The encoder is hand-written, so it is checked against a decoder that was
    /// not: the system's `base64`. A one-character slip would surface as a broken
    /// image in the preview and nowhere else.
    #[test]
    fn the_cursor_asset_decodes_back_to_the_original_png() {
        let url = cursor_asset();
        let encoded = url
            .strip_prefix("data:image/png;base64,")
            .expect("not a png data url");

        let Ok(mut child) = Command::new("base64")
            .arg("-d")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        else {
            return; // no base64 on PATH
        };

        child
            .stdin
            .as_mut()
            .expect("stdin was piped")
            .write_all(encoded.as_bytes())
            .expect("could not write to base64");

        let Ok(output) = child.wait_with_output() else {
            return;
        };
        if !output.status.success() {
            return;
        }
        assert_eq!(
            output.stdout,
            effects::CURSOR_PNG,
            "the data url does not decode back to the embedded asset"
        );
    }
}