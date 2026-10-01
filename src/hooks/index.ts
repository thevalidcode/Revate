import { useEffect, useState } from "react";
import type { RefObject } from "react";

/** A measured box, in CSS pixels. */
export interface ElementSize {
  width: number;
  height: number;
}

const EMPTY: ElementSize = { width: 0, height: 0 };

/**
 * Track an element's content box with a `ResizeObserver`.
 *
 * The first read is synchronous (via `getBoundingClientRect`) so the very first
 * paint already has a size, and later reads are coalesced by comparing against
 * the previous value so a resize storm doesn't re-render on every frame.
 *
 * Layout built on top of this is independent of its *content's* intrinsic size,
 * which is what makes it safe for media elements: a `<video>` reports 0×0 (and
 * falls back to 300×150) until its metadata arrives, and its intrinsic size is
 * different again once it does.
 */
export function useElementSize(ref: RefObject<HTMLElement | null>): ElementSize {
  const [size, setSize] = useState<ElementSize>(EMPTY);

  useEffect(() => {
    const element = ref.current;
    if (!element) return;

    const publish = (width: number, height: number) => {
      setSize((prev) =>
        // Sub-pixel jitter from a fractional window size is not worth a render.
        Math.abs(prev.width - width) < 1 && Math.abs(prev.height - height) < 1
          ? prev
          : { width, height },
      );
    };

    const rect = element.getBoundingClientRect();
    publish(rect.width, rect.height);

    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (!entry) return;
      publish(entry.contentRect.width, entry.contentRect.height);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref]);

  return size;
}
