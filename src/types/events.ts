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
  /**
   * The resolved effects table, one row per 1/60 s, in video pixels. This is the
   * same table the export turns into FFmpeg commands, so the preview and the
   * saved file cannot disagree. Empty when the take has no trail, or when both
   * layers are switched off.
   */
  rows: EffectRow[];
  /** Clicks, for the preview's ripple. */
  clicks: ClickMark[];
  /**
   * The cursor image's geometry, so the preview anchors the tip exactly where the
   * export does instead of guessing at its own hotspot.
   */
  cursorSprite: SpriteInfo;
}

/**
 * Mirror of `effects::EffectRow` — one tick of the effects table.
 *
 * `x/y/w/h` are the visible rectangle in **video pixels**; `cx/cy` are the cursor
 * tip in the same space, or null when the pointer was not known at that instant.
 */
export interface EffectRow {
  /** Seconds from the start of the take. */
  t: number;
  x: number;
  y: number;
  w: number;
  h: number;
  cx: number | null;
  cy: number | null;
}

/** Mirror of `effects::ClickMark`. */
export interface ClickMark {
  t: number;
  x: number;
  y: number;
}

/** Mirror of `effects::SpriteInfo` — the cursor image's own geometry. */
export interface SpriteInfo {
  width: number;
  height: number;
  /** The hotspot (the tip of the arrow) inside the image. */
  hotX: number;
  hotY: number;
  /** The opaque content's box, used to size the *visible* arrow. */
  contentWidth: number;
  contentHeight: number;
}

/**
 * Mirror of `effects::OverlayOptions` — the sidebar's effect sliders.
 *
 * These are sent with both `session_analysis` and `export_recording` so the
 * preview and the render are resolved from the same settings. Rust clamps every
 * field, so an out-of-range value here is corrected rather than trusted.
 */
export interface OverlayOptions {
  zoom: boolean;
  /** Multiplier on each planned segment's zoom level. */
  zoomStrength: number;
  /** How far the viewport centre is held away from the frame's edges. */
  zoomEdgeSnap: number;
  cursor: boolean;
  /** Multiplier on the cursor's size. */
  cursorScale: number;
  /** Spring angular frequency in rad/s; higher settles faster. */
  cursorSmoothing: number;
  ripples: boolean;
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
  /**
   * The editor's effect settings. Sent so the export is resolved from exactly
   * what the preview showed — omit it and the defaults are used, which is what a
   * caller with no sidebar should get.
   */
  options?: OverlayOptions;
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

