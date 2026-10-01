import { useRef } from "react";
import type { PointerEvent as ReactPointerEvent } from "react";

/** A crop rectangle in normalized 0–1 frame coordinates. */
export interface CropRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export const FULL_FRAME: CropRect = { x: 0, y: 0, w: 1, h: 1 };

export const isFullFrame = (rect: CropRect) =>
  rect.x <= 0.001 && rect.y <= 0.001 && rect.w >= 0.999 && rect.h >= 0.999;

type Handle = "move" | "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";

/** Smallest crop we allow, as a fraction of the frame. */
const MIN = 0.04;

const clamp = (value: number, lo: number, hi: number) =>
  Math.min(hi, Math.max(lo, value));

/** Largest box of the given *pixel* ratio that fits in the frame, centred. */
export function centeredCrop(ratio: number, frameRatio: number): CropRect {
  const w = frameRatio > ratio ? ratio / frameRatio : 1;
  const h = frameRatio > ratio ? 1 : frameRatio / ratio;
  return { x: (1 - w) / 2, y: (1 - h) / 2, w, h };
}

/**
 * Move/resize a normalized rect by a normalized delta.
 *
 * `boxRatio` is the *pixel* aspect (width/height) of the video element, which is
 * needed because a normalized rect's w/h is not the same as the crop's pixel
 * aspect — the frame's own proportions scale it.
 */
function applyDrag(
  handle: Handle,
  origin: CropRect,
  dx: number,
  dy: number,
  lockRatio: number | null,
  boxRatio: number,
): CropRect {
  if (handle === "move") {
    return {
      ...origin,
      x: clamp(origin.x + dx, 0, 1 - origin.w),
      y: clamp(origin.y + dy, 0, 1 - origin.h),
    };
  }

  const west = handle.includes("w");
  const east = handle.includes("e");
  const north = handle.includes("n");
  const south = handle.includes("s");

  let left = origin.x;
  let top = origin.y;
  let right = origin.x + origin.w;
  let bottom = origin.y + origin.h;

  if (west) left = clamp(left + dx, 0, right - MIN);
  if (east) right = clamp(right + dx, left + MIN, 1);
  if (north) top = clamp(top + dy, 0, bottom - MIN);
  if (south) bottom = clamp(bottom + dy, top + MIN, 1);

  let w = right - left;
  let h = bottom - top;

  if (lockRatio && lockRatio > 0) {
    // Normalized w/h that yields the requested pixel ratio.
    const target = lockRatio / boxRatio;
    const horizontal = west || east;
    const corner = horizontal && (north || south);
    const useWidth = corner
      ? Math.abs(w - origin.w) >= Math.abs(h - origin.h)
      : horizontal;

    // Cap the driven dimension by the room its anchored edge leaves, so the
    // derived one still fits the frame. Clamping *after* deriving would break
    // the ratio.
    if (useWidth) {
      const room = north ? bottom : south ? 1 - top : bottom - top;
      w = Math.min(w, room * target);
      h = w / target;
    } else {
      const room = west ? right : east ? 1 - left : right - left;
      h = Math.min(h, room / target);
      w = h * target;
    }

    // Re-anchor each axis on the edge the pointer did not drag.
    if (west) left = right - w;
    else if (east) right = left + w;
    if (north) top = bottom - h;
    else if (south) bottom = top + h;

    // The derived dimension can push the rect out of frame; pull it back.
    if (bottom > 1) {
      bottom = 1;
      top = Math.max(0, 1 - h);
    }
    if (top < 0) {
      top = 0;
      bottom = Math.min(1, h);
    }
    if (right > 1) {
      right = 1;
      left = Math.max(0, 1 - w);
    }
    if (left < 0) {
      left = 0;
      right = Math.min(1, w);
    }
  }

  w = Math.min(1, right - left);
  h = Math.min(1, bottom - top);
  if (w < MIN || h < MIN) return origin;

  return { x: left, y: top, w, h };
}

const HANDLES: { id: Exclude<Handle, "move">; className: string }[] = [
  { id: "nw", className: "-top-1 -left-1 cursor-nwse-resize" },
  { id: "n", className: "-top-1 left-1/2 -translate-x-1/2 cursor-ns-resize" },
  { id: "ne", className: "-top-1 -right-1 cursor-nesw-resize" },
  { id: "e", className: "top-1/2 -right-1 -translate-y-1/2 cursor-ew-resize" },
  { id: "se", className: "-bottom-1 -right-1 cursor-nwse-resize" },
  { id: "s", className: "-bottom-1 left-1/2 -translate-x-1/2 cursor-ns-resize" },
  { id: "sw", className: "-bottom-1 -left-1 cursor-nesw-resize" },
  { id: "w", className: "top-1/2 -left-1 -translate-y-1/2 cursor-ew-resize" },
];

/**
 * Interactive crop layer drawn over the preview.
 *
 * Drag inside the rect to move it, drag any of the eight handles to resize, and
 * the four panels around it dim everything that will be cut away. All maths is
 * done in normalized frame coordinates, so the result is independent of the
 * recording's resolution.
 */
export function CropOverlay({
  rect,
  onChange,
  lockRatio,
  disabled,
}: {
  rect: CropRect;
  onChange: (rect: CropRect) => void;
  lockRatio: number | null;
  disabled?: boolean;
}) {
  const box = useRef<HTMLDivElement>(null);
  const drag = useRef<{
    handle: Handle;
    origin: CropRect;
    px: number;
    py: number;
  } | null>(null);

  const start =
    (handle: Handle) =>
    (event: ReactPointerEvent) => {
      if (disabled) return;
      event.preventDefault();
      event.stopPropagation();
      box.current?.setPointerCapture(event.pointerId);
      drag.current = {
        handle,
        origin: rect,
        px: event.clientX,
        py: event.clientY,
      };
    };

  const move = (event: ReactPointerEvent) => {
    const active = drag.current;
    const element = box.current;
    if (!active || !element) return;

    const bounds = element.getBoundingClientRect();
    if (bounds.width === 0 || bounds.height === 0) return;

    onChange(
      applyDrag(
        active.handle,
        active.origin,
        (event.clientX - active.px) / bounds.width,
        (event.clientY - active.py) / bounds.height,
        lockRatio,
        bounds.width / bounds.height,
      ),
    );
  };

  const end = (event: ReactPointerEvent) => {
    if (!drag.current) return;
    box.current?.releasePointerCapture(event.pointerId);
    drag.current = null;
  };

  const pct = (value: number) => `${value * 100}%`;
  const guides = [33.33, 66.66];

  return (
    <div
      ref={box}
      onPointerMove={move}
      onPointerUp={end}
      onPointerCancel={end}
      className="absolute inset-0 touch-none"
    >
      {/* Dimmed area outside the crop, as four panels so the rect stays hittable. */}
      <div
        className="pointer-events-none absolute inset-x-0 top-0 bg-black/55"
        style={{ height: pct(rect.y) }}
      />
      <div
        className="pointer-events-none absolute inset-x-0 bottom-0 bg-black/55"
        style={{ height: pct(1 - (rect.y + rect.h)) }}
      />
      <div
        className="pointer-events-none absolute top-0 bottom-0 left-0 bg-black/55"
        style={{ width: pct(rect.x) }}
      />
      <div
        className="pointer-events-none absolute top-0 right-0 bottom-0 bg-black/55"
        style={{ width: pct(1 - (rect.x + rect.w)) }}
      />

      {/* The crop rect. */}
      <div
        onPointerDown={start("move")}
        className="absolute cursor-move border-2 border-[#8B5CF6]"
        style={{
          left: pct(rect.x),
          top: pct(rect.y),
          width: pct(rect.w),
          height: pct(rect.h),
        }}
      >
        {/* Rule-of-thirds guides. */}
        <div className="pointer-events-none absolute inset-0">
          {guides.map((pos) => (
            <div
              key={`v${pos}`}
              className="absolute top-0 bottom-0 w-px bg-white/25"
              style={{ left: `${pos}%` }}
            />
          ))}
          {guides.map((pos) => (
            <div
              key={`h${pos}`}
              className="absolute right-0 left-0 h-px bg-white/25"
              style={{ top: `${pos}%` }}
            />
          ))}
        </div>

        {HANDLES.map((handle) => (
          <div
            key={handle.id}
            onPointerDown={start(handle.id)}
            className={`absolute z-10 size-2.5 rounded-[2px] border border-[#8B5CF6] bg-white ${handle.className}`}
          />
        ))}
      </div>
    </div>
  );
}

