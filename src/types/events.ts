// Shared shapes for the Tauri ⇄ React boundary.
// Rust uses `#[serde(rename_all = "camelCase")]`, so every field here is camelCase.

/** Mirror of `commands::displays::DisplayRect`. */
export interface DisplayRect {
  /** Position in the monitor list; also the `?picker=N` id. */
  index: number;
  name: string;
  x: number;
  y: number;
  width: number;
  height: number;
  scaleFactor: number;
  isPrimary: boolean;
}

/** Mirror of `commands::recording::AudioDevice`. */
export interface AudioDevice {
  id: string;
  name: string;
}

/** Mirror of `commands::recording::RegionInput`. */
export interface RegionRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Arguments accepted by the `start_recording` command. */
export interface StartRecordingArgs {
  displayIndex?: number;
  mic?: string | null;
  systemAudio?: string | null;
  region?: RegionRect | null;
  fps?: number;
  captureCursor?: boolean;
}

/** Emitted on the main window by `commands::displays::display_chosen`. */
export const DISPLAY_CHOSEN_EVENT = "display-chosen";

