mod audio;
mod capture;
mod commands;
mod state;
mod utils;

use state::RecordingState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(RecordingState::default())
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}