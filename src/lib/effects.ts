// The preview's half of the effects timeline.
//
// Rust resolves the whole take once into rows at 60 Hz (`commands::analysis`),
// and the export turns those same rows into FFmpeg commands. This module does
// the other half: reading the table back at an arbitrary playback time so the
// editor can draw what the export will produce. The arithmetic deliberately
// mirrors `effects.rs` rather than re-deriving anything — the whole point of the
// shared table is that the two sides cannot disagree.

import type { EffectRow, OverlayOptions, SpriteInfo } from "@/types/events";

/** A rectangle in video pixels. */
export interface Viewport {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** The defaults Rust's `OverlayOptions::default()` uses. */
export const DEFAULT_OPTIONS: OverlayOptions = {
  zoom: true,
  zoomStrength: 1,
  zoomEdgeSnap: 0.3,
  cursor: true,
  cursorScale: 1,
  // Mirrors `input::cursor::DEFAULT_SMOOTHING` (18.0).
  cursorSmoothing: 18,
  ripples: true,
};

const clamp = (value: number, low: number, high: number) =>
  Math.min(Math.max(value, low), high);

/**
 * Clamp an options object the way Rust's `sanitized()` does.
 *
 * A slider dragged past its stop can produce Infinity, and `Infinity - Infinity`
 * is NaN — which in the layout maths silently collapses the preview to nothing.
 * Rust clamps independently, so doing the same here keeps the drawn frame inside
 * the box Rust actually renders.
 */
export function sanitizeOptions(options: OverlayOptions): OverlayOptions {
  const safe = (value: number, fallback: number, low: number, high: number) =>
    Number.isFinite(value) ? clamp(value, low, high) : fallback;

  return {
    zoom: options.zoom,
    zoomStrength: safe(options.zoomStrength, 1, 1, 2.5),
    zoomEdgeSnap: safe(options.zoomEdgeSnap, 0.3, 0, 0.5),
    cursor: options.cursor,
    cursorScale: safe(options.cursorScale, 1, 0.25, 3),
    cursorSmoothing: safe(options.cursorSmoothing, 18, 4, 48),
    ripples: options.ripples,
  };
}

/**
 * Fit a viewport into the output box — the TS twin of `effects::fit_viewport`.
 *
 * A viewport larger than the box (the un-zoomed case) shrinks to it; a smaller
 * one keeps its size and slides until it sits inside. Keep the two in step.
 */
export function fitViewport(view: Viewport, bounds: Viewport): Viewport {
  const width = Math.max(2, Math.min(view.width, bounds.width));
  const height = Math.max(2, Math.min(view.height, bounds.height));

  const cx = view.x + view.width / 2;
  const cy = view.y + view.height / 2;
  const x = clamp(
    cx - width / 2,
    bounds.x,
    Math.max(bounds.x, bounds.x + bounds.width - width),
  );
  const y = clamp(
    cy - height / 2,
    bounds.y,
    Math.max(bounds.y, bounds.y + bounds.height - height),
  );

  return { x, y, width, height };
}

/**
 * The two rows bracketing `time`, or null for an empty table.
 *
 * A binary search: a ten-minute take is 36 000 rows, and the preview asks this
 * question on every animation frame.
 */
function bracket(rows: EffectRow[], time: number): [EffectRow, EffectRow] | null {
  if (rows.length === 0) return null;
  if (time <= rows[0].t) return [rows[0], rows[0]];
  const last = rows[rows.length - 1];
  if (time >= last.t) return [last, last];

  let low = 0;
  let high = rows.length - 1;
  while (high - low > 1) {
    const mid = (low + high) >> 1;
    if (rows[mid].t <= time) low = mid;
    else high = mid;
  }
  return [rows[low], rows[high]];
}

const lerp = (a: number, b: number, f: number) => a + (b - a) * f;

/**
 * The viewport and cursor position at `time`, interpolated between rows.
 *
 * Rows are 60 Hz and the display refreshes at 60 Hz or more, so interpolating is
 * what keeps the zoom from visibly stepping. The cursor is interpolated only
 * when *both* rows know it: a null tip means "not seen yet", and inventing a
 * position there would send the pointer flying in from the corner.
 */
export function sampleAt(
  rows: EffectRow[],
  time: number,
  bounds: Viewport,
): { view: Viewport; cursor: { x: number; y: number } | null } {
  const pair = bracket(rows, time);
  if (!pair) return { view: bounds, cursor: null };

  const [a, b] = pair;
  const span = b.t - a.t;
  const f = span > 0 ? clamp((time - a.t) / span, 0, 1) : 0;

  const view = fitViewport(
    {
      x: lerp(a.x, b.x, f),
      y: lerp(a.y, b.y, f),
      width: lerp(a.w, b.w, f),
      height: lerp(a.h, b.h, f),
    },
    bounds,
  );

  const cursor =
    a.cx !== null && a.cy !== null && b.cx !== null && b.cy !== null
      ? { x: lerp(a.cx, b.cx, f), y: lerp(a.cy, b.cy, f) }
      : null;

  return { view, cursor };
}

/**
 * The cursor image's size for the preview, in **output** pixels.
 *
 * Mirrors `effects::sprite_metrics`: the arrow is sized against the video's own
 * width (a 4K take should not get a thumbnail-sized pointer) and measured
 * against the image's *content* box, because the transparent padding around the
 * arrow would otherwise shrink the visible pointer.
 */
export function spriteSize(
  sprite: SpriteInfo,
  outWidth: number,
  cursorScale: number,
): { width: number; height: number } {
  const reference = Math.max(2, 28 * (outWidth / 1920) * cursorScale);
  return {
    width: reference * (sprite.width / sprite.contentWidth),
    height: reference * (sprite.height / sprite.contentHeight),
  };
}