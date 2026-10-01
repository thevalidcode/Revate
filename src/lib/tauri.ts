// Typed wrappers around the Rust commands registered in `src-tauri/src/lib.rs`.
// Keeping them in one place means the UI never has to remember command names or
// argument casing (Tauri maps camelCase JS keys onto snake_case Rust params).
import { convertFileSrc, invoke } from "@tauri-apps/api/core";

import type {
  AudioDevice,
  DisplayRect,
  ExportArgs,
  ProjectInfo,
  SessionAnalysis,
  SessionInfo,
  StartRecordingArgs,
  StopResult,
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

export const stopRecording = () => invoke<StopResult>("stop_recording");

export const isRecording = () => invoke<boolean>("is_recording");

/** Re-mux an existing project folder into `final.mp4`. */
export const muxRecording = (projectDir: string) =>
  invoke<string>("mux_recording", { projectDir });

// ---------- Editor window ----------

/** Close the recorder and hand `sessionId` to a fresh editor window. */
export const openEditor = (sessionId: string) =>
  invoke<void>("open_editor", { sessionId });

/** Reopen the recorder and retire the editor window. */
export const newRecording = () => invoke<void>("new_recording");

export const sessionInfo = (sessionId: string) =>
  invoke<SessionInfo>("session_info", { sessionId });

/** Extract a poster frame; returns "" when one can't be produced. */
export const makeThumbnail = (sessionId: string) =>
  invoke<string>("make_thumbnail", { sessionId });

/**
 * Read the take's recorded input trail and plan its auto-zoom segments.
 *
 * Returns an empty `zoomSegments` (and `hasCursorTrail: false`) for a take
 * recorded without input tracking — that is a normal outcome, not an error.
 */
export const sessionAnalysis = (sessionId: string) =>
  invoke<SessionAnalysis>("session_analysis", { sessionId });

// ---------- Export ----------

export const exportRecording = (args: ExportArgs) =>
  invoke<string>("export_recording", { ...args });

/** Select the saved file in Finder. */
export const revealInFinder = (path: string) =>
  invoke<void>("reveal_in_finder", { path });

/**
 * Turn an absolute path on disk into a URL the webview is allowed to load.
 * Requires `app.security.assetProtocol` in tauri.conf.json.
 */
export const assetUrl = (path: string) => convertFileSrc(path);

// ---------- Projects library ----------

/** Open (or focus) the standalone projects window. */
export const openProjectsWindow = () => invoke<void>("open_projects_window");

/** Every session that has a video, newest first. */
export const listProjects = () => invoke<ProjectInfo[]>("list_projects");

/** Rename a session folder; resolves with the new name. */
export const renameProject = (id: string, newName: string) =>
  invoke<string>("rename_project", { id, newName });

/** Permanently delete a session folder. */
export const deleteProject = (id: string) =>
  invoke<void>("delete_project", { id });

/** Open a project in the editor window (closes the projects window). */
export const openProjectInEditor = (id: string) =>
  invoke<void>("open_project_in_editor", { id });

