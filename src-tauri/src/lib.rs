mod audio;
mod capture;
mod commands;
mod state;
mod utils;

use tauri::Manager;

use state::RecordingState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Folder picker used by the editor's "Destination" control.
        .plugin(tauri_plugin_dialog::init())
        .manage(RecordingState::default())
        // Quit once the *last* window goes away. Without this, closing the
        // recorder would leave a headless process behind. Tauri removes the
        // window from its registry before dispatching `Destroyed`, so the
        // emptiness check is already accurate here.
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Destroyed)
                && window.app_handle().webview_windows().is_empty()
            {
                window.app_handle().exit(0);
            }
        })
        .invoke_handler(tauri::generate_handler![
            // Display enumeration + the on-screen picker.
            commands::displays::list_display_rects,
            commands::displays::get_selected_display,
            commands::displays::open_display_picker,
            commands::displays::close_all_pickers,
            commands::displays::display_chosen,
            // Audio device enumeration.
            commands::recording::list_capture_devices,
            commands::recording::list_audio_inputs,
            commands::recording::list_system_audio_devices,
            // Recording lifecycle.
            commands::recording::start_recording,
            commands::recording::stop_recording,
            commands::recording::is_recording,
            commands::recording::mux_recording,
            // Editor window + session introspection.
            commands::editor::open_editor,
            commands::editor::new_recording,
            commands::editor::session_info,
            commands::editor::make_thumbnail,
            // Export pipeline.
            commands::export::export_recording,
            commands::export::reveal_in_finder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}