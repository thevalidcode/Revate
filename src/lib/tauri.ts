// Typed wrappers around the Rust commands registered in `src-tauri/src/lib.rs`.
// Keeping them in one place means the UI never has to remember command names or
// argument casing (Tauri maps camelCase JS keys onto snake_case Rust params).
import { invoke } from "@tauri-apps/api/core";

import type {
  AudioDevice,
  DisplayRect,
  StartRecordingArgs,
} from "@/types/events";

// ---------- Displays ----------

export const listDisplayRects = () =>
  invoke<DisplayRect[]>("list_display_rects");

export const getSelectedDisplay = () =>
  invoke<number | null>("get_selected_display");

/** Spawn one overlay window per monitor. */
export const openDisplayPicker = () => invoke<void>("open_display_picker");

export const closeAllPickers = () => invoke<void>("close_all_pickers");

/** Confirm the display shown in the current picker window. */
export const displayChosen = (index: number) =>
  invoke<DisplayRect>("display_chosen", { index });

// ---------- Audio ----------

export const listAudioInputs = () => invoke<AudioDevice[]>("list_audio_inputs");

export const listSystemAudioDevices = () =>
  invoke<AudioDevice[]>("list_system_audio_devices");

// ---------- Recording ----------

export const startRecording = (args: StartRecordingArgs) =>
  invoke<string>("start_recording", { ...args });

export const stopRecording = () => invoke<string>("stop_recording");

export const isRecording = () => invoke<boolean>("is_recording");

/** Re-mux an existing project folder into `final.mp4`. */
export const muxRecording = (projectDir: string) =>
  invoke<string>("mux_recording", { projectDir });

