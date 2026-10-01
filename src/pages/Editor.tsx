import { useCallback, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { Download, FolderOpen, Video } from "lucide-react";
import { toast } from "sonner";

import { AppShell } from "@/components/Layout/AppShell";
import { Button } from "@/components/ui/button";
import { CircularProgress } from "@/components/ui/CircularProgress";
import { Label } from "@/components/ui/label";
import { Toaster } from "@/components/ui/sonner";
import { cn } from "@/lib/utils";
import {
  assetUrl,
  exportRecording,
  makeThumbnail,
  newRecording,
  revealInFinder,
  sessionInfo,
} from "@/lib/tauri";
import { EXPORT_PROGRESS_EVENT } from "@/types/events";
import type { AspectId, ExportProgress, SessionInfo } from "@/types/events";

/** Crop presets. `ratio` doubles as the little preview glyph's shape. */
const ASPECTS: { id: AspectId; label: string; ratio: number | null }[] = [
  { id: "original", label: "Original", ratio: null },
  { id: "16-9", label: "16:9", ratio: 16 / 9 },
  { id: "1-1", label: "1:1", ratio: 1 },
  { id: "9-16", label: "9:16", ratio: 9 / 16 },
];

const baseName = (path: string) => path.split(/[\\/]/).pop() ?? path;

function formatDuration(ms: number) {
  const total = Math.round(ms / 1000);
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

/**
 * Editor window body — rendered in its own webview on
 * `index.html?editor=1&session=<id>` (see `App.tsx`).
 *
 * Scope for now: review the take, pick a crop preset, and save a copy into a
 * folder of your choosing. Both the initial load and the export are fronted by
 * the circular loader — the former from local state, the latter from the
 * `export-progress` events Rust emits while FFmpeg runs.
 */
export default function Editor({ sessionId }: { sessionId: string }) {
  const [info, setInfo] = useState<SessionInfo | null>(null);
  const [thumb, setThumb] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [name, setName] = useState("");
  const [folder, setFolder] = useState("");
  const [aspect, setAspect] = useState<AspectId>("original");

  const [exporting, setExporting] = useState(false);
  const [progress, setProgress] = useState(0);

  // ---------- Initial load ----------
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const [meta, poster] = await Promise.all([
          sessionInfo(sessionId),
          makeThumbnail(sessionId),
        ]);
        if (cancelled) return;
        setInfo(meta);
        setThumb(poster || null);
        setName(meta.suggestedName);
        setFolder(meta.defaultFolder);
      } catch (error) {
        if (!cancelled) setLoadError(String(error));
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [sessionId]);

  // ---------- Export progress, pushed from Rust ----------
  useEffect(() => {
    const unlisten = listen<ExportProgress>(EXPORT_PROGRESS_EVENT, (event) => {
      setProgress(event.payload.percent);
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

  // ---------- Derived ----------
  const loading = !info && !loadError;

  /** Crop rectangle as a percentage of the source frame, for the preview mask. */
  const cropBox = useMemo(() => {
    const preset = ASPECTS.find((item) => item.id === aspect);
    if (!info || !preset?.ratio) return null;
    const source = info.width / info.height;
    const ratio = preset.ratio;
    return {
      width: (source > ratio ? ratio / source : 1) * 100,
      height: (source > ratio ? 1 : source / ratio) * 100,
    };
  }, [aspect, info]);

  // ---------- Actions ----------
  const chooseFolder = useCallback(async () => {
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        title: "Choose a destination folder",
      });
      if (typeof picked === "string") setFolder(picked);
    } catch (error) {
      toast.error(String(error));
    }
  }, []);

  const runExport = useCallback(async () => {
    if (!info || !folder || !name.trim()) return;

    setExporting(true);
    setProgress(0);

    let saved: string | null = null;
    try {
      saved = await exportRecording({ sessionId, folder, fileName: name, aspect });
      toast.success(`✨ Saved to ${baseName(saved)}`, { duration: 4000 });
    } catch (error) {
      toast.error(String(error));
    }

    if (saved) {
      // Hand the file to Finder straight away; a failure here is cosmetic.
      void revealInFinder(saved).catch(() => {});
    }

    // Let the ring sit at 100% long enough for the sparkle burst to read.
    window.setTimeout(() => setExporting(false), saved ? 900 : 0);
  }, [aspect, folder, info, name, sessionId]);

  const busy = exporting || loading;
  const canExport = Boolean(info && folder && name.trim());

  return (
    <AppShell
      label={name ? `${name}.mp4` : undefined}
      actions={
        <Button
          variant="ghost"
          size="sm"
          disabled={exporting}
          onClick={() => void newRecording()}
          className="h-7 gap-1.5 rounded-md px-2 text-[12px] font-medium"
        >
          <Video className="size-3.5" />
          New recording
        </Button>
      }
    >
      <div className="flex min-h-0 flex-1">
        {/* ---------------- Preview ---------------- */}
        <section className="flex min-w-0 flex-1 flex-col border-r border-border">
          <div className="grid min-h-0 flex-1 place-items-center p-5">
            {loadError ? (
              <div className="max-w-sm text-center">
                <p className="text-[13px] font-medium">
                  This recording could not be opened
                </p>
                <p className="mt-1 text-[11px] break-words text-muted-foreground">
                  {loadError}
                </p>
                <Button
                  onClick={() => void newRecording()}
                  className="mt-3 h-8 rounded-md px-3 text-[12px] font-medium"
                >
                  Back to recorder
                </Button>
              </div>
            ) : info ? (
              <div className="relative inline-grid max-h-full max-w-full">
                <video
                  src={assetUrl(info.videoPath)}
                  poster={thumb ? assetUrl(thumb) : undefined}
                  controls
                  preload="metadata"
                  className="max-h-full max-w-full rounded-lg border border-border bg-black"
                />

                {/* Crop mask — the huge spread shadow dims everything outside. */}
                {cropBox && (
                  <div className="pointer-events-none absolute inset-0 grid place-items-center">
                    <div
                      className="absolute rounded-sm border-2 border-dashed border-[#8B5CF6] shadow-[0_0_0_9999px_rgba(0,0,0,0.55)]"
                      style={{
                        width: `${cropBox.width}%`,
                        height: `${cropBox.height}%`,
                      }}
                    />
                  </div>
                )}
              </div>
            ) : null}
          </div>

          <div className="shrink-0 border-t border-border px-4 py-2 text-[11px] tabular-nums text-muted-foreground">
            {info
              ? `${info.width}×${info.height} · ${formatDuration(info.durationMs)} · ${info.hasAudio ? "Audio" : "No audio"}`
              : "—"}
          </div>
        </section>

        {/* ---------------- Side rail ---------------- */}
        <aside className="flex w-[284px] shrink-0 flex-col overflow-auto">
          {/* Crop */}
          <div className="border-b border-border px-3.5 py-3">
            <Label className="text-[12px] font-medium">Crop</Label>
            <div className="mt-2 grid grid-cols-4 gap-1">
              {ASPECTS.map((preset) => {
                const ratio = preset.ratio ?? 16 / 9;
                const active = aspect === preset.id;
                return (
                  <button
                    key={preset.id}
                    type="button"
                    disabled={busy}
                    onClick={() => setAspect(preset.id)}
                    className={cn(
                      "flex flex-col items-center gap-1.5 rounded-md border px-1 py-2 text-[10px] font-medium transition-colors disabled:pointer-events-none disabled:opacity-50",
                      active
                        ? "border-[#8B5CF6] bg-[#8B5CF6]/10 text-foreground"
                        : "border-border text-muted-foreground hover:bg-secondary hover:text-foreground",
                    )}
                  >
                    <span
                      className="block rounded-[2px] border border-current"
                      style={{ width: Math.round(14 * ratio), height: 14 }}
                    />
                    {preset.label}
                  </button>
                );
              })}
            </div>
            <p className="mt-2 text-[11px] text-muted-foreground">
              {cropBox
                ? "Centre crop is applied when you save."
                : "The full frame is exported."}
            </p>
          </div>

          {/* Save */}
          <div className="px-3.5 py-3">
            <Label htmlFor="editor-name" className="text-[12px] font-medium">
              Name
            </Label>
            <div className="mt-1.5 flex items-center gap-1.5">
              <input
                id="editor-name"
                value={name}
                disabled={busy}
                onChange={(event) => setName(event.target.value)}
                className="h-8 min-w-0 flex-1 rounded-md border border-input bg-transparent px-2 text-[12px] outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 disabled:opacity-50"
              />
              <span className="shrink-0 text-[11px] text-muted-foreground">
                .mp4
              </span>
            </div>

            <Label className="mt-3 block text-[12px] font-medium">
              Destination
            </Label>
            <div className="mt-1.5 flex items-center gap-1.5">
              <div
                title={folder}
                className="min-w-0 flex-1 truncate rounded-md border border-input px-2 py-1.5 text-[11px] text-muted-foreground"
              >
                {folder || "No folder selected"}
              </div>
              <Button
                variant="outline"
                size="icon"
                disabled={busy}
                onClick={chooseFolder}
                aria-label="Choose destination folder"
                title="Choose destination folder"
                className="size-8 shrink-0"
              >
                <FolderOpen className="size-3.5" />
              </Button>
            </div>

            <Button
              onClick={runExport}
              disabled={busy || !canExport}
              className="mt-3 h-8 w-full gap-1.5 rounded-md bg-[#8B5CF6] text-[12px] font-semibold text-white hover:bg-[#7C3AED]"
            >
              <Download className="size-3.5" />
              Save a copy
            </Button>
          </div>
        </aside>
      </div>

      {loading && (
        <CenteredLoader>
          <CircularProgress percent={90} label="Loading recording…" />
        </CenteredLoader>
      )}

      {exporting && (
        <CenteredLoader>
          <CircularProgress percent={progress} label="Exporting…" />
        </CenteredLoader>
      )}

      <Toaster />
    </AppShell>
  );
}

/** Full-window scrim behind the circular loader. */
function CenteredLoader({ children }: { children: ReactNode }) {
  return (
    <div className="fixed inset-0 z-50 grid place-items-center bg-background/85 backdrop-blur-sm">
      {children}
    </div>
  );
}