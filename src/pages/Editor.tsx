import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Crop,
  Download,
  FolderOpen,
  Lock,
  LockOpen,
  Pause,
  Play,
  RotateCcw,
  Video,
} from "lucide-react";
import { toast } from "sonner";

import { AppShell } from "@/components/Layout/AppShell";
import {
  centeredCrop,
  CropOverlay,
  FULL_FRAME,
  isFullFrame,
} from "@/components/editor/CropOverlay";
import type { CropRect } from "@/components/editor/CropOverlay";
import { EffectsOverlay } from "@/components/editor/EffectsOverlay";
import { Button } from "@/components/ui/button";
import { CircularProgress } from "@/components/ui/CircularProgress";
import { Label } from "@/components/ui/label";
import { Toaster } from "@/components/ui/sonner";
import { Switch } from "@/components/ui/switch";
import { useElementSize, usePlaybackTime } from "@/hooks";
import { DEFAULT_OPTIONS, sanitizeOptions } from "@/lib/effects";
import { cn } from "@/lib/utils";
import {
  assetUrl,
  cursorAsset,
  exportRecording,
  makeThumbnail,
  newRecording,
  revealInFinder,
  sessionAnalysis,
  sessionInfo,
} from "@/lib/tauri";
import { EXPORT_PROGRESS_EVENT } from "@/types/events";
import type {
  AspectId,
  ExportProgress,
  OverlayOptions,
  SessionAnalysis,
  SessionInfo,
} from "@/types/events";

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
 *
 * The preview is a *clean* frame by default: no crop chrome, just a transport
 * button that gets out of the way. The mask, grid and eight handles belong to
 * custom crop mode only, which the sidebar's "Custom" control toggles and `Esc`
 * leaves.
 *
 * Sizing is deliberately measurement-driven. The window is created at 1000×720
 * (`spawn_editor`), but its first layout pass can land before that is applied —
 * and a `<video>` has no usable intrinsic size until its metadata decodes. So
 * the stage measures itself and the frame is laid out at the size that fits,
 * rather than letting the video's own box decide how big the preview is.
 */
export default function Editor({ sessionId }: { sessionId: string }) {
  const [info, setInfo] = useState<SessionInfo | null>(null);
  const [thumb, setThumb] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [name, setName] = useState("");
  const [folder, setFolder] = useState("");
  const [aspect, setAspect] = useState<AspectId>("original");

  // ---------- Custom crop ----------
  const [crop, setCrop] = useState<CropRect>(FULL_FRAME);
  /** True only while the interactive crop overlay is on screen. */
  const [custom, setCustom] = useState(false);
  /** Pixel ratio to hold while resizing, or null for free-form. */
  const [lockRatio, setLockRatio] = useState<number | null>(null);
  const [locked, setLocked] = useState(false);

  const [exporting, setExporting] = useState(false);
  const [progress, setProgress] = useState(0);

  // ---------- Preview ----------
  const stageRef = useRef<HTMLDivElement>(null);
  const videoRef = useRef<HTMLVideoElement>(null);
  const stage = useElementSize(stageRef);
/**
   * The effect settings the user can bend. Sent to both `session_analysis` and
   * `export_recording`, so the preview and the saved file are resolved from one
   * set of values rather than two that can drift apart.
   */
  const [options, setOptions] = useState<OverlayOptions>(DEFAULT_OPTIONS);
  /** The cursor image as a data URL, the same bytes the export composites. */
  const [cursorImage, setCursorImage] = useState<string | null>(null);

  const setOption = useCallback(
    <K extends keyof OverlayOptions>(key: K, value: OverlayOptions[K]) =>
      setOptions((prev) => ({ ...prev, [key]: value })),
    [],
  );

  // ---------- Preview ----------
  const [playing, setPlaying] = useState(false);
  const [hovering, setHovering] = useState(false);
  /** The take's recorded trail + planned zooms, or null when it has none. */
/** Playback position, sampled per animation frame — drives the effects layers. */
  const playTime = usePlaybackTime(videoRef);

  // ---------- Initial load ----------
  const [analysis, setAnalysis] = useState<SessionAnalysis | null>(null);

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

      // The cursor trail is optional: a take recorded without input tracking has
      // none, and that must not turn into an error banner over a perfectly good
      // recording. Read it separately so it can only ever add information.
      void cursorAsset()
        .then((url) => {
          if (!cancelled) setCursorImage(url);
        })
        .catch(() => {
          // No preview cursor; the export embeds its own copy regardless.
        });
    })();

    return () => {
      cancelled = true;
    };
  }, [sessionId]);

  // ---------- Re-resolve the effects table when a slider moves ----------
  //
  // The table is not computed in the webview: Rust resolves it, because the
  // export has to reproduce it exactly. Moving a slider therefore re-asks for the
  // table rather than nudging it locally, which is what keeps the preview and the
  // file from disagreeing about where the zoom goes.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const result = await sessionAnalysis(
          sessionId,
          sanitizeOptions(options),
        );
        if (!cancelled) setAnalysis(result);
      } catch {
        if (!cancelled) setAnalysis(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [options, sessionId]);

  // ---------- Export progress, pushed from Rust ----------
  useEffect(() => {
    const unlisten = listen<ExportProgress>(EXPORT_PROGRESS_EVENT, (event) => {
      setProgress(event.payload.percent);
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

  // ---------- Esc leaves custom crop ----------
  useEffect(() => {
    if (!custom) return;

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setCustom(false);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [custom]);

  // ---------- Derived ----------
  // ---------- Derived ----------
  const loading = !info && !loadError;

  /**
   * The crop rect the *preview* is currently showing, or `null` for the whole
   * frame.
   *
   * Custom mode always shows the full frame — the mask and handles need the
   * whole picture to drag against. Everywhere else a non-full rect clips the
   * preview, so picking a preset visibly reshapes the video instead of only
   * moving some chrome around on top of it.
   */
  const previewCrop = useMemo(() => {
    if (custom) return null;
    return isFullFrame(crop) ? null : crop;
  }, [crop, custom]);

  /**
   * Scale that fits the *whole* recording inside the measured stage. The
   * preview never zooms: cropping shrinks the visible box and clips, so a
   * pixel of video stays the same size on screen while you switch presets.
   */
  const scale = useMemo(() => {
    if (!info || stage.width < 1 || stage.height < 1) return null;
    return Math.min(stage.width / info.width, stage.height / info.height);
  }, [info, stage.height, stage.width]);

  /** The <video>'s own box at that scale — always the full frame. */
  const videoBox = useMemo(() => {
    if (!info || scale == null) return null;
    return { width: info.width * scale, height: info.height * scale };
  }, [info, scale]);

  /**
   * The visible box: the video box clipped to the current crop. Its aspect
   * ratio is the export's, so the preview reads as the finished file.
   */
  const frame = useMemo(() => {
    if (!videoBox) return null;
    const rect = previewCrop ?? FULL_FRAME;
    return {
      width: Math.max(1, Math.round(videoBox.width * rect.w)),
      height: Math.max(1, Math.round(videoBox.height * rect.h)),
    };
  }, [previewCrop, videoBox]);

  /** Offset that slides the crop's top-left corner onto the box's origin. */
  const videoOffset = useMemo(() => {
    if (!videoBox) return null;
    const rect = previewCrop ?? FULL_FRAME;
    return { left: -rect.x * videoBox.width, top: -rect.y * videoBox.height };
  }, [previewCrop, videoBox]);

  /** Pixel size the current crop will produce (even, for yuv420p). */
  const cropSize = useMemo(() => {
    if (!info) return null;
    const w = Math.round((crop.w * info.width) / 2) * 2;
    const h = Math.round((crop.h * info.height) / 2) * 2;
    return `${w}×${h}`;
  }, [crop, info]);

/**
   * The visible box in **video** pixels — the same rectangle the export crops to,
   * so the effects rows (also in video pixels) can be drawn against it directly
   * instead of being converted first.
   */
  const effectBounds = useMemo(() => {
    if (!info) return null;
    const rect = previewCrop ?? FULL_FRAME;
    return {
      x: rect.x * info.width,
      y: rect.y * info.height,
      width: rect.w * info.width,
      height: rect.h * info.height,
    };
  }, [info, previewCrop]);

  /** Ratio actively held while resizing: the lock, or the source frame. */
  /** Ratio actively held while resizing: the lock, or the source frame. */
  const effectiveLock = locked
    ? (lockRatio ?? (info ? info.width / info.height : null))
    : null;

  // ---------- Actions ----------
  const togglePlay = useCallback(() => {
    const video = videoRef.current;
    if (!video) return;
    if (video.paused) void video.play().catch(() => setPlaying(false));
    else video.pause();
  }, []);

  /**
   * Preset buttons are a quick-start: they centre the rect, lock the ratio and
   * drop straight back to the clean preview — the rect is applied, but there is
   * nothing left to drag.
   */
  const applyPreset = useCallback(
    (preset: (typeof ASPECTS)[number]) => {
      setAspect(preset.id);
      setCustom(false);
      if (!info) return;

      if (!preset.ratio) {
        setCrop(FULL_FRAME);
        setLockRatio(null);
        setLocked(false);
        return;
      }
      setCrop(centeredCrop(preset.ratio, info.width / info.height));
      setLockRatio(preset.ratio);
      setLocked(true);
    },
    [info],
  );

  /**
   * Reveal the crop chrome, keeping whatever rect is already set so a preset can
   * be nudged rather than re-picked. The export falls back to "no crop" here —
   * in custom mode the rect is the only source of truth.
   */
  const enterCustom = useCallback(() => {
    setAspect("original");
    setCustom(true);
  }, []);

  /** Back to the full frame, staying in custom mode so it can be re-dragged. */
  const resetCrop = useCallback(() => {
    setAspect("original");
    setCrop(FULL_FRAME);
    setLockRatio(null);
    setLocked(false);
  }, []);

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
      saved = await exportRecording({
        sessionId,
        folder,
        fileName: name,
        aspect,
        // A full-frame rect means "no crop", which lets the preset decide.
        crop: isFullFrame(crop) ? null : crop,
        // The settings the user just approved on screen. The export re-resolves
        // the effects table from these, so the file matches the preview rather
        // than falling back to whatever the defaults happen to be.
        options: sanitizeOptions(options),
      });
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
  }, [aspect, crop, folder, info, name, options, sessionId]);

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
          {/*
            The stage owns the layout: it is `flex-1 min-h-0` inside the column,
            so it always has a height of its own. The full-frame video box is
            sized in pixels from that measurement, and the visible box is that
            box clipped to the active crop — never the `<video>`'s intrinsic
            size, which is 300×150 until its metadata lands and is what used to
            make the preview come up small on a cold window.
          */}
          <div className="flex min-h-0 flex-1 items-center justify-center overflow-hidden p-5">
            {loadError ? (
              <div className="max-w-sm text-center">
                <p className="text-[13px] font-medium">
                  This recording could not be opened
                </p>
                <p className="mt-1 text-[11px] wrap-break-word text-muted-foreground">
                  {loadError}
                </p>
                <Button
                  onClick={() => void newRecording()}
                  className="mt-3 h-8 rounded-md px-3 text-[12px] font-medium"
                >
                  Back to recorder
                </Button>
              </div>
            ) : (
              // Always mounted (even before `info` lands) so the observer has
              // something to measure.
              <div
                ref={stageRef}
                className="relative flex size-full items-center justify-center"
              >
                {info && frame && videoBox && videoOffset && (
                  <div
                    className="relative shrink-0 overflow-hidden rounded-lg border border-border bg-black"
                    style={{ width: frame.width, height: frame.height }}
                    onPointerEnter={() => setHovering(true)}
                    onPointerLeave={() => setHovering(false)}
                  >
                    {/*
                    The <video> stays at full-frame size and is slid behind
                    the box's origin; `overflow-hidden` on the parent does the
                    cropping. `max-w-none` is required — Tailwind's preflight
                    caps replaced elements at `max-width: 100%`, which would
                    otherwise squash the video into the (smaller) box.
                  */}
                    {/*
                    The zoom's crop is a clip-path on a wrapper around the video
                    rather than a transform on it: the video has to keep its own
                    full-frame box, because the crop overlay's handles and the
                    full-frame layout are both expressed against it.

                    The wrapper is `inset: 0` on the visible box, so its percentages
                    are relative to exactly the rectangle the export crops to.
                  */}
                    <EffectsOverlay
                      rows={analysis?.rows ?? []}
                      time={playTime}
                      bounds={effectBounds ?? { x: 0, y: 0, width: 0, height: 0 }}
                      boxWidth={frame.width}
                      boxHeight={frame.height}
                      options={options}
                      sprite={cursorImage}
                      spriteInfo={analysis?.cursorSprite ?? null}
                      hideCursor={analysis?.cursorBakedIn ?? false}
                    >
                      <video
                        ref={videoRef}
                        src={assetUrl(info.videoPath)}
                        poster={thumb ? assetUrl(thumb) : undefined}
                        preload="metadata"
                        playsInline
                        onClick={() => {
                          // In custom mode the overlay owns the pointer, so a
                          // click on the frame means a drag, not a seek.
                          if (!custom) togglePlay();
                        }}
                        onPlay={() => setPlaying(true)}
                        onPause={() => setPlaying(false)}
                        className="absolute max-w-none object-contain"
                        style={{
                          left: videoOffset.left,
                          top: videoOffset.top,
                          width: videoBox.width,
                          height: videoBox.height,
                        }}
                      />
                    </EffectsOverlay>

                    {/* Mask, thirds guides and the eight handles — custom only. */}
                    {custom && (
                      <CropOverlay
                        rect={crop}
                        onChange={setCrop}
                        lockRatio={effectiveLock}
                        disabled={busy}
                      />
                    )}

                    <PlayPauseButton
                      playing={playing}
                      visible={!playing || hovering}
                      onToggle={togglePlay}
                    />
                  </div>
                )}
              </div>
            )}
          </div>

          <div className="shrink-0 border-t border-border px-4 py-2 text-[11px] tabular-nums text-muted-foreground">
            {info
              ? `${info.width}×${info.height} · ${formatDuration(info.durationMs)} · ${info.hasAudio ? "Audio" : "No audio"}`
              : "—"}
            {/* What the recorded input trail yielded, so a missing trail is visible
                rather than a silent no-op. */}
            {analysis && (
              <span className="ml-2">
                ·{" "}
                {analysis.hasCursorTrail
                  ? `Cursor trail: ${analysis.eventCount} events · ${analysis.zoomSegments.length} zoom${
                      analysis.zoomSegments.length === 1 ? "" : "s"
                    }`
                  : analysis.cursorBakedIn
                    ? "No cursor trail (Accessibility permission needed)"
                    : "No cursor trail"}
              </span>
            )}
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
                const active = !custom && aspect === preset.id;
                return (
                  <button
                    key={preset.id}
                    type="button"
                    disabled={busy}
                    onClick={() => applyPreset(preset)}
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

            {/*
              The only way into the crop chrome. Everything else — mask, grid,
              handles, lock, reset — is scoped to this mode.
            */}
            <button
              type="button"
              disabled={busy}
              aria-pressed={custom}
              onClick={enterCustom}
              className={cn(
                "mt-1.5 flex h-8 w-full items-center gap-1.5 rounded-md border px-2 text-[11px] font-medium transition-colors disabled:pointer-events-none disabled:opacity-50",
                custom
                  ? "border-[#8B5CF6] bg-[#8B5CF6]/10 text-foreground"
                  : "border-border text-muted-foreground hover:bg-secondary hover:text-foreground",
              )}
            >
              <Crop className="size-3.5" />
              Custom
              {custom && (
                <span className="ml-auto text-[10px] text-muted-foreground">
                  Esc to close
                </span>
              )}
            </button>

            <p className="mt-2 text-[11px] text-muted-foreground">
              {custom
                ? isFullFrame(crop)
                  ? "Drag the handles to crop."
                  : `Exports at ${cropSize}. Drag inside to move.`
                : isFullFrame(crop)
                  ? "The full frame is kept."
                  : `Exports at ${cropSize}.`}
            </p>

            {custom && (
              <div className="mt-2 flex items-center gap-1.5">
                <Button
                  variant={locked ? "secondary" : "ghost"}
                  size="sm"
                  disabled={busy}
                  onClick={() => setLocked((prev) => !prev)}
                  title={
                    locked
                      ? "Unlock — resize freely"
                      : `Lock to ${lockRatio ? lockRatio.toFixed(2) : "the frame"}:1`
                  }
                  className="h-7 gap-1.5 rounded-md px-2 text-[11px] font-medium"
                >
                  {locked ? (
                    <Lock className="size-3.5" />
                  ) : (
                    <LockOpen className="size-3.5" />
                  )}
                  {locked ? "Ratio locked" : "Free crop"}
                </Button>

                <Button
                  variant="ghost"
                  size="sm"
                  disabled={busy}
                  onClick={resetCrop}
                  title="Reset crop to the full frame"
                  className="h-7 gap-1.5 rounded-md px-2 text-[11px] font-medium"
                >
                  <RotateCcw className="size-3.5" />
                  Reset
                </Button>
              </div>
            )}
          </div>

          {/*
              Effects. These are the sliders the whole feature exists for: the
              preview redraws from the re-resolved table as they move, and the
              same values travel with the export. Nothing here touches the
              recording.
            */}
            <div className="border-b border-border px-3.5 py-3">
              <Label className="text-[12px] font-medium">Effects</Label>

              {/* A take with no trail has nothing to preview, so the panel says so
                  rather than offering sliders that would silently do nothing. */}
              {!analysis?.hasCursorTrail ? (
                <p className="mt-1.5 text-[11px] text-muted-foreground">
                  {analysis?.cursorBakedIn
                    ? "This recording has no cursor trail, so the cursor and auto-zoom are unavailable."
                    : "No cursor trail was recorded for this take, so there is nothing to overlay."}
                </p>
              ) : (
                <>
                  <div className="mt-2 space-y-2">
                    <SwitchRow
                      label="Auto-zoom"
                      hint={`${analysis.zoomSegments.length} focus point${analysis.zoomSegments.length === 1 ? "" : "s"}`}
                      checked={options.zoom}
                      disabled={busy}
                      onChange={(v) => setOption("zoom", v)}
                    />
                    <SwitchRow
                      label="Cursor"
                      hint={analysis.cursorBakedIn ? "already in the video" : undefined}
                      checked={options.cursor && !analysis.cursorBakedIn}
                      disabled={busy || analysis.cursorBakedIn}
                      onChange={(v) => setOption("cursor", v)}
                    />
                  </div>

                  {/* The ranges mirror Rust's `sanitized()` exactly; a slider that
                      could send a value Rust would silently clamp would be a
                      slider lying to the user. */}
                  {options.zoom && (
                    <Slider
                      label="Zoom strength"
                      value={options.zoomStrength}
                      min={1}
                      max={2.5}
                      step={0.05}
                      format={(v) => `${v.toFixed(2)}×`}
                      disabled={busy}
                      onChange={(v) => setOption("zoomStrength", v)}
                    />
                  )}
                  {options.zoom && (
                    <Slider
                      label="Edge snap"
                      value={options.zoomEdgeSnap}
                      min={0}
                      max={0.5}
                      step={0.01}
                      format={(v) => v.toFixed(2)}
                      disabled={busy}
                      onChange={(v) => setOption("zoomEdgeSnap", v)}
                    />
                  )}
                  {options.cursor && !analysis.cursorBakedIn && (
                    <Slider
                      label="Cursor size"
                      value={options.cursorScale}
                      min={0.25}
                      max={3}
                      step={0.05}
                      format={(v) => `${v.toFixed(2)}×`}
                      disabled={busy}
                      onChange={(v) => setOption("cursorScale", v)}
                    />
                  )}
                  {options.cursor && !analysis.cursorBakedIn && (
                    <Slider
                      label="Cursor smoothing"
                      value={options.cursorSmoothing}
                      min={4}
                      max={48}
                      step={1}
                      format={(v) => `${v.toFixed(0)} rad/s`}
                      disabled={busy}
                      onChange={(v) => setOption("cursorSmoothing", v)}
                    />
                  )}
                </>
              )}
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

/**
 * The one piece of chrome a clean preview keeps: a centred transport button.
 *
 * It stays put while the clip is paused, and fades away once playback starts and
 * the pointer leaves — the frame should read as video, not as an editor. When it
 * is hidden it is also unclickable, so it never swallows a click meant for the
 * video underneath.
 */
/**
 * A labelled on/off row for one effects layer.
 *
 * The optional `hint` carries why a switch is unavailable or what it is doing —
 * a disabled cursor with no explanation just looks broken.
 */
function SwitchRow({
  label,
  hint,
  checked,
  disabled,
  onChange,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <label
      className={cn(
        "flex items-center gap-2 text-[12px]",
        disabled ? "opacity-60" : "cursor-pointer",
      )}
    >
      <Switch
        size="sm"
        checked={checked}
        disabled={disabled}
        onCheckedChange={onChange}
      />
      <span className="font-medium">{label}</span>
      {hint && (
        <span className="ml-auto text-[10px] text-muted-foreground">{hint}</span>
      )}
    </label>
  );
}

/**
 * One bounded numeric effect setting.
 *
 * `min`/`max` are not decoration: they are the range Rust's `sanitized()` clamps
 * to, and a slider that could emit a value the export would silently clamp would
 * be telling the user one thing and saving another.
 */
function Slider({
  label,
  value,
  min,
  max,
  step,
  format,
  disabled,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  format: (value: number) => string;
  disabled?: boolean;
  onChange: (value: number) => void;
}) {
  return (
    <div className={cn("mt-2.5", disabled && "opacity-60")}>
      <div className="flex items-baseline justify-between">
        <span className="text-[11px] font-medium">{label}</span>
        <span className="text-[10px] tabular-nums text-muted-foreground">
          {format(value)}
        </span>
      </div>
      <input
        type="range"
        aria-label={label}
        value={value}
        min={min}
        max={max}
        step={step}
        disabled={disabled}
        onChange={(event) => onChange(Number(event.target.value))}
        className="mt-1 h-1 w-full cursor-pointer appearance-none rounded-full bg-input accent-[#8B5CF6] disabled:cursor-not-allowed"
      />
    </div>
  );
}

function PlayPauseButton({
  playing,
  visible,
  onToggle,
}: {
  playing: boolean;
  visible: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-label={playing ? "Pause" : "Play"}
      title={playing ? "Pause" : "Play"}
      className={cn(
        "absolute top-1/2 left-1/2 z-20 grid size-12 -translate-x-1/2 -translate-y-1/2 place-items-center rounded-full",
        "bg-black/55 text-white backdrop-blur-sm transition-opacity duration-200",
        "hover:bg-black/70 focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-white/70 focus-visible:outline-none",
        visible ? "opacity-100" : "pointer-events-none opacity-0",
      )}
    >
      {playing ? (
        <Pause className="size-5" />
      ) : (
        <Play className="ml-0.5 size-5" />
      )}
    </button>
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
