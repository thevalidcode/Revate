import { useCallback, useEffect, useState } from "react";
import { Monitor, MousePointerClick } from "lucide-react";

import { Wordmark } from "@/components/Brand/Logo";
import { cn } from "@/lib/utils";
import { closeAllPickers, displayChosen, listDisplayRects } from "@/lib/tauri";
import type { DisplayRect } from "@/types/events";

/**
 * Full-screen overlay rendered inside one of the per-monitor picker windows
 * spawned by `open_display_picker`. The window is already sized to exactly
 * cover its monitor, so this is just the visual layer:
 *
 *   idle   → dimmed backdrop, display name + resolution, muted grab pill
 *   hover  → violet inset border + a filled "Record here" pill
 *   click  → `display_chosen` closes every overlay and reports back to main
 *   Esc    → `close_all_pickers`
 */
export default function DisplayPicker({ index }: { index: number }) {
  const [display, setDisplay] = useState<DisplayRect | null>(null);
  const [hovered, setHovered] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    listDisplayRects()
      .then((rects) => {
        if (!cancelled) {
          setDisplay(rects.find((rect) => rect.index === index) ?? null);
        }
      })
      .catch(() => {
        /* overlay still works without the label */
      });
    return () => {
      cancelled = true;
    };
  }, [index]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        void closeAllPickers();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const choose = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    try {
      await displayChosen(index);
    } catch (error) {
      console.error("display_chosen failed", error);
      setBusy(false);
    }
  }, [busy, index]);

  return (
    <button
      type="button"
      aria-label="Record this display"
      onClick={choose}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      className="fixed inset-0 grid cursor-pointer place-items-center bg-black/40 text-foreground outline-none select-none"
    >
      {/* Violet inset frame on hover */}
      <span
        className={cn(
          "pointer-events-none absolute inset-2 rounded-3xl border-2 transition-all duration-150",
          hovered
            ? "border-[#8B5CF6] bg-[#8B5CF6]/10 shadow-[inset_0_0_120px_rgba(139,92,246,0.35)]"
            : "border-white/20",
        )}
      />

      <span className="pointer-events-none absolute top-6 left-8 flex items-center gap-2.5">
        <Wordmark size={22} textClassName="text-white/90" />
      </span>

      <span className="pointer-events-none absolute top-6 right-8 text-[12px] font-medium tracking-wide text-white/60 uppercase">
        Press Esc to cancel
      </span>

      <span
        className={cn(
          "pointer-events-none relative flex flex-col items-center gap-5 transition-transform duration-150",
          hovered && "scale-[1.03]",
        )}
      >
        <span className="flex flex-col items-center gap-1.5 rounded-2xl bg-black/55 px-7 py-5 backdrop-blur-md ring-1 ring-white/10">
          <span className="flex items-center gap-2 text-white">
            <Monitor className="size-4 opacity-70" />
            <span className="text-[17px] font-semibold tracking-tight">
              {display ? display.name : `Display ${index + 1}`}
            </span>
            {display?.isPrimary && (
              <span className="rounded-full bg-white/15 px-2 py-0.5 text-[10px] font-medium tracking-wide text-white/80 uppercase">
                Primary
              </span>
            )}
          </span>
          {display && (
            <span className="text-[12px] tabular-nums text-white/60">
              {display.width} × {display.height}
              {display.scaleFactor !== 1 && ` · ${display.scaleFactor}×`}
            </span>
          )}
        </span>

        <span
          className={cn(
            "inline-flex items-center gap-2 rounded-full px-6 py-3 text-[14px] font-semibold shadow-lg transition-all duration-150",
            hovered
              ? "bg-[#8B5CF6] text-white shadow-[#8B5CF6]/40"
              : "bg-white/12 text-white/80 ring-1 ring-white/20 backdrop-blur-md",
          )}
        >
          <MousePointerClick className="size-4" />
          {busy ? "Selecting…" : "Record here"}
        </span>
      </span>
    </button>
  );
}
