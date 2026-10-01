//! Display enumeration and the on-screen display picker.
//!
//! `open_display_picker` spawns one borderless, transparent, always-on-top
//! window per monitor. Each overlay covers exactly one monitor and loads the
//! frontend with `?picker=<index>`, where `App.tsx` renders the picker instead
//! of the recorder. Clicking "Record here" calls `display_chosen`, which stores
//! the selection, closes every overlay and emits `display-chosen` back to the
//! main window.

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewUrl,
    WebviewWindowBuilder,
};

use crate::state::RecordingState;

/// Emitted on the main window once the user has picked a display.
pub const DISPLAY_CHOSEN_EVENT: &str = "display-chosen";
const PICKER_LABEL_PREFIX: &str = "picker";
const MAIN_WINDOW_LABEL: &str = "main";

/// A monitor, described in physical pixels.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayRect {
    /// Position in `available_monitors()`, used as the `?picker=N` id and the
    /// value stored in [`RecordingState::selected_display`].
    pub index: usize,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
    pub is_primary: bool,
}

/// Every connected monitor, in the same order as `app.available_monitors()`.
#[tauri::command]
pub async fn list_display_rects(app: AppHandle) -> Result<Vec<DisplayRect>, String> {
    let monitors = app.available_monitors().map_err(|e| e.to_string())?;
    let primary = app.primary_monitor().ok().flatten();

    Ok(monitors
        .iter()
        .enumerate()
        .map(|(index, monitor)| {
            let position = monitor.position();
            let size = monitor.size();
            let is_primary = primary
                .as_ref()
                .map(|p| p.position() == position && p.size() == size)
                .unwrap_or(false);

            DisplayRect {
                index,
                name: monitor
                    .name()
                    .cloned()
                    .unwrap_or_else(|| format!("Display {}", index + 1)),
                x: position.x,
                y: position.y,
                width: size.width,
                height: size.height,
                scale_factor: monitor.scale_factor(),
                is_primary,
            }
        })
        .collect())
}

/// The display index the user last confirmed, if any.
#[tauri::command]
pub async fn get_selected_display(state: State<'_, RecordingState>) -> Result<Option<usize>, String> {
    Ok(*state.selected_display.lock().unwrap())
}

/// Spawn one click-through-to-select overlay window per monitor.
/// Any overlays from a previous invocation are closed first.
#[tauri::command]
pub async fn open_display_picker(
    app: AppHandle,
    state: State<'_, RecordingState>,
) -> Result<(), String> {
    close_picker_windows(&app, &state);

    let monitors = app.available_monitors().map_err(|e| e.to_string())?;
    let mut labels = Vec::with_capacity(monitors.len());

    for (index, monitor) in monitors.iter().enumerate() {
        let label = format!("{PICKER_LABEL_PREFIX}-{index}");
        // A stale window from a previous run would block the new one.
        if let Some(existing) = app.get_webview_window(&label) {
            let _ = existing.close();
        }

        let position = monitor.position();
        let size = monitor.size();
        let scale = monitor.scale_factor().max(1.0);
        let url = WebviewUrl::App(format!("index.html?picker={index}").into());

        let window = WebviewWindowBuilder::new(&app, label.clone(), url)
            .title("Select a display to record")
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .resizable(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .accept_first_mouse(true)
            .visible(false)
            // position/inner_size take logical pixels…
            .position(position.x as f64 / scale, position.y as f64 / scale)
            .inner_size(size.width as f64 / scale, size.height as f64 / scale)
            .build()
            .map_err(|e| e.to_string())?;

        // …then we snap to the exact physical bounds so mixed-DPI setups line
        // up perfectly with each monitor.
        let _ = window.set_position(PhysicalPosition::new(position.x, position.y));
        let _ = window.set_size(PhysicalSize::new(size.width, size.height));
        let _ = window.show();

        labels.push(label);
    }

    *state.picker_windows.lock().unwrap() = labels;
    Ok(())
}

/// Confirm `index` as the recording display, tear down the overlays and notify
/// the main window.
#[tauri::command]
pub async fn display_chosen(
    app: AppHandle,
    state: State<'_, RecordingState>,
    index: usize,
) -> Result<DisplayRect, String> {
    let rects = list_display_rects(app.clone()).await?;
    let chosen = rects
        .get(index)
        .cloned()
        .ok_or_else(|| format!("unknown display index {index}"))?;

    *state.selected_display.lock().unwrap() = Some(index);
    close_picker_windows(&app, &state);

    if let Some(main) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        let _ = main.emit(DISPLAY_CHOSEN_EVENT, chosen.clone());
        let _ = main.set_focus();
    } else {
        let _ = app.emit(DISPLAY_CHOSEN_EVENT, chosen.clone());
    }

    Ok(chosen)
}

/// Close every overlay window (e.g. when the user cancels with `Esc`).
#[tauri::command]
pub async fn close_all_pickers(
    app: AppHandle,
    state: State<'_, RecordingState>,
) -> Result<(), String> {
    close_picker_windows(&app, &state);
    Ok(())
}

fn close_picker_windows(app: &AppHandle, state: &RecordingState) {
    let labels = std::mem::take(&mut *state.picker_windows.lock().unwrap());
    for label in labels {
        if let Some(window) = app.get_webview_window(&label) {
            let _ = window.close();
        }
    }
}

