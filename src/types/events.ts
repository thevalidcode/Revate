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
  /**
   * Events written to `events.revents`; `0` when input tracking was unavailable
   * (Accessibility permission refused, or the enhanced cursor was switched off).
   */
  eventsRecorded: number;
}

/** Why the planner created a zoom segment. */
export type SegmentReason = "click" | "dwell";

/** Mirror of `commands::analysis::SegmentInfo`. */
export interface ZoomSegmentInfo {
  /** Seconds from the start of the take. */
  startT: number;
  endT: number;
  /** Focus point, in video pixels. */
  x: number;
  y: number;
  /** 1.0 is the full frame; higher is more zoomed in. */
  zoomLevel: number;
  reason: SegmentReason;
}

/**
 * Mirror of `commands::analysis::SessionAnalysis` — the take's recorded input
 * trail, resolved into video-pixel space.
 *
 * A take with no trail yields `hasCursorTrail: false` and an empty
 * `zoomSegments`; that is a normal outcome, not a failure.
 */
export interface SessionAnalysis {
  hasCursorTrail: boolean;
  /** True when FFmpeg burned the system cursor in, so ours must not be drawn. */
  cursorBakedIn: boolean;
  eventCount: number;
  skipped: number;
  width: number;
  height: number;
  durationMs: number;
  clickCount: number;
  zoomSegments: ZoomSegmentInfo[];
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

