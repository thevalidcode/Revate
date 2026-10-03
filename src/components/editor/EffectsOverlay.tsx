import type { ReactNode } from "react";

import { sampleAt, spriteSize } from "@/lib/effects";
import type { Viewport } from "@/lib/effects";
import type { EffectRow, OverlayOptions, SpriteInfo } from "@/types/events";

/**
 * The effects layer drawn over the preview: the auto-zoom's crop and the custom
 * cursor.
 *
 * This is a *preview*, not a second renderer. It reads the same rows the export
 * turns into FFmpeg commands (`effects.rs`), so what the user scrubs through here
 * is what they get in the file. Nothing about the recording is modified — every
 * adjustment is a slider, and the export re-resolves the table from them.
 *
 * Children (the `<video>`) are wrapped in the zoom's clip rather than being
 * transformed directly, because the video has to keep its own full-frame box: the
 * crop overlay's handles and the full-frame layout are both expressed against it.
 * The cursor is a sibling of that clip, not inside it — FFmpeg composites the
 * sprite *after* the crop, so a cursor inside the clip would be cropped away.
 */
export interface EffectsOverlayProps {
  /** The resolved table; empty means "nothing to draw". */
  rows: EffectRow[];
  /** Playback position in seconds. */
  time: number;
  /** The visible box in video pixels (the crop, or the whole frame). */
  bounds: Viewport;
  /** The visible box's size on screen, in CSS pixels. */
  boxWidth: number;
  boxHeight: number;
  options: OverlayOptions;
  /** The cursor image, as a data URL. */
  sprite: string | null;
  spriteInfo: SpriteInfo | null;
  /**
   * True when FFmpeg already burned a cursor into the pixels. The overlay then
   * draws nothing: compositing ours on top would show two pointers.
   */
  hideCursor?: boolean;
  children: ReactNode;
}

export function EffectsOverlay({
  rows,
  time,
  bounds,
  boxWidth,
  boxHeight,
  options,
  sprite,
  spriteInfo,
  hideCursor = false,
  children,
}: EffectsOverlayProps) {
  // No table (a take with no trail, or both layers off): render the children
  // untouched rather than clipping to something meaningless.
  if (rows.length === 0 || bounds.width <= 0 || bounds.height <= 0) {
    return <>{children}</>;
  }

  const { view, cursor } = sampleAt(rows, time, bounds);

  // Video pixels -> output pixels. `bounds` is the whole visible box, so this is
  // the scale the frame itself is drawn at, which is exactly what the cursor has
  // to travel through.
  const scaleX = boxWidth / bounds.width;
  const scaleY = boxHeight / bounds.height;

  // Where the zoom's rectangle sits inside the visible box, as a percentage, so
  // the clip rect can be expressed without reading layout back out of the DOM.
  const left = ((view.x - bounds.x) / bounds.width) * 100;
  const top = ((view.y - bounds.y) / bounds.height) * 100;
  const width = (view.width / bounds.width) * 100;
  const height = (view.height / bounds.height) * 100;

  // The cursor rides the zoom, not the frame: its row position is in video
  // pixels, so it goes through the same crop-then-scale the image did.
  const pointer =
    cursor && options.cursor
      ? {
          left: (cursor.x - view.x) * scaleX,
          top: (cursor.y - view.y) * scaleY,
        }
      : null;

  const size =
    pointer && spriteInfo
      ? spriteSize(spriteInfo, bounds.width, options.cursorScale)
      : null;

  // A take whose cursor FFmpeg burned in already has one drawn in the pixels;
  // adding ours would double it.
  const showCursor = Boolean(
    options.cursor && !hideCursor && sprite && spriteInfo && pointer,
  );

  return (
    <>
      <div
        className="absolute inset-0"
        style={
          options.zoom
            ? {
                clipPath: `inset(${top}% ${100 - left - width}% ${
                  100 - top - height
                }% ${left}%)`,
              }
            : undefined
        }
      >
        {children}
      </div>

      {showCursor && pointer && size && sprite && spriteInfo && (
        // The tip is the hotspot, so the arrow's corner is offset back by it —
        // the same subtraction `effects::cursor_position` does for the overlay.
        <img
          src={sprite}
          alt=""
          aria-hidden
          className="pointer-events-none absolute max-w-none select-none"
          style={{
            width: size.width * scaleX,
            height: size.height * scaleY,
            left: pointer.left - spriteInfo.hotX * scaleX,
            top: pointer.top - spriteInfo.hotY * scaleY,
          }}
        />
      )}
    </>
  );
}
