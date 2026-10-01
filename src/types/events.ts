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

/** Emitted on the editor window while an export runs. */
export const EXPORT_PROGRESS_EVENT = "export-progress";

/** Mirror of `commands::recording::StopResult`. */
export interface StopResult {
  /** Session id — the editor window's `?session=` value. */
  id: string;
  projectDir: string;
  videoPath: string;
}

/** Mirror of `commands::editor::SessionInfo`. */
export interface SessionInfo {
  id: string;
  projectDir: string;
  videoPath: string;
  /** Absolute path to the extracted poster frame, or `null`/empty. */
  thumbPath: string | null;
  width: number;
  height: number;
  durationMs: number;
  hasAudio: boolean;
  suggestedName: string;
  /** Where exports land unless the user picks somewhere else. */
  defaultFolder: string;
}

/** Payload of the `export-progress` event. */
export interface ExportProgress {
  percent: number;
  /** `encoding` while FFmpeg runs, then `done` (or `error`). */
  phase: string;
}

/** Aspect-ratio presets understood by `commands::export::Aspect`. */
export type AspectId = "original" | "16-9" | "1-1" | "9-16";

/** Arguments accepted by the `export_recording` command. */
export interface ExportArgs {
  sessionId: string;
  folder: string;
  fileName: string;
  aspect: AspectId;
  /**
   * Custom crop as fractions (0–1) of the source frame. Wins over `aspect`
   * when present; omit it (or use a full-frame rect) for no crop.
   */
  crop?: NormalizedCrop | null;
}

/** A crop rectangle in normalized 0–1 frame coordinates. */
export interface NormalizedCrop {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Mirror of `commands::projects::ProjectInfo`. */
export interface ProjectInfo {
  /** Folder name — the session id used by every other command. */
  id: string;
  name: string;
  path: string;
  sizeBytes: number;
  durationMs: number;
  width: number;
  height: number;
  /** Absolute path of an already-extracted poster, if one exists. */
  thumbPath: string | null;
  /** Folder mtime in epoch milliseconds (list is ordered newest first). */
  modifiedMs: number;
}

