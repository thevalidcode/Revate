import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ComponentProps, ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  ChevronRight,
  Circle,
  Crop,
  FolderOpen,
  Mic,
  Monitor,
  Square,
  Volume2,
} from "lucide-react";
import { toast } from "sonner";

import { AppShell } from "@/components/Layout/AppShell";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { Toaster } from "@/components/ui/sonner";
import { cn } from "@/lib/utils";
import {
  getSelectedDisplay,
  listAudioInputs,
  listDisplayRects,
  listSystemAudioDevices,
  openDisplayPicker,
  openEditor,
  openProjectsWindow,
  startRecording,
  stopRecording,
} from "@/lib/tauri";
import { DISPLAY_CHOSEN_EVENT } from "@/types/events";
import type { AudioDevice, DisplayRect, RegionRect } from "@/types/events";

/** Fixed-size region presets. Regions are anchored to the display's top-left. */
const REGION_PRESETS = [
  { id: "1080p", label: "1080p", width: 1920, height: 1080 },
  { id: "1440p", label: "1440p", width: 2560, height: 1440 },
  { id: "720p", label: "720p", width: 1280, height: 720 },
] as const;

type RegionMode = "full" | (typeof REGION_PRESETS)[number]["id"] | "custom";

const REGION_OPTIONS: { value: RegionMode; label: string }[] = [
  { value: "full", label: "Full screen" },
  ...REGION_PRESETS.map((preset) => ({
    value: preset.id as RegionMode,
    label: preset.label,
  })),
  { value: "custom", label: "Custom" },
];

const FPS = 30;

export default function Recorder() {
  // ---------- Displays ----------
  const [displays, setDisplays] = useState<DisplayRect[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(0);

  // ---------- Audio ----------
  const [mics, setMics] = useState<AudioDevice[]>([]);
  const [selectedMic, setSelectedMic] = useState("");
  const [micEnabled, setMicEnabled] = useState(false);
  const [systemDevices, setSystemDevices] = useState<AudioDevice[]>([]);
  const [selectedSystem, setSelectedSystem] = useState("");
  const [systemEnabled, setSystemEnabled] = useState(false);

  // ---------- Region ----------
  const [regionMode, setRegionMode] = useState<RegionMode>("full");
  const [customWidth, setCustomWidth] = useState(1920);
  const [customHeight, setCustomHeight] = useState(1080);

  // ---------- Recording ----------
  const [recording, setRecording] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const startedAt = useRef<number | null>(null);

  // ---------- Load devices on mount (Rust may not be reachable in a plain
  // browser preview, so every call is guarded) ----------
  useEffect(() => {
    let cancelled = false;

    void (async () => {
      try {
        const [rects, selected] = await Promise.all([
          listDisplayRects(),
          getSelectedDisplay(),
        ]);
        if (cancelled) return;
        setDisplays(rects);
        setSelectedIndex(selected ?? rects[0]?.index ?? 0);
      } catch (error) {
        console.error("list_display_rects failed", error);
      }

      try {
        const [inputs, system] = await Promise.all([
          listAudioInputs(),
          listSystemAudioDevices(),
        ]);
        if (cancelled) return;
        setMics(inputs);
        setSelectedMic((prev) => prev || inputs[0]?.id || "");
        setSystemDevices(system);
        setSelectedSystem((prev) => prev || system[0]?.id || "");
      } catch (error) {
        console.error("audio device enumeration failed", error);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, []);

  // ---------- The picker reports its choice back through an event ----------
  useEffect(() => {
    const unlisten = listen<DisplayRect>(DISPLAY_CHOSEN_EVENT, (event) => {
      const chosen = event.payload;
      setDisplays((prev) =>
        prev.map((display) =>
          display.index === chosen.index ? chosen : display,
        ),
      );
      setSelectedIndex(chosen.index);
      toast.success(`Recording ${chosen.name}`);
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

  // ---------- Recording timer ----------
  useEffect(() => {
    if (!recording) return;
    const id = window.setInterval(() => {
      const from = startedAt.current;
      if (from !== null) setElapsed((Date.now() - from) / 1000);
    }, 100);
    return () => window.clearInterval(id);
  }, [recording]);

  // ---------- Derived ----------
  const activeDisplay = useMemo(
    () => displays.find((display) => display.index === selectedIndex) ?? null,
    [displays, selectedIndex],
  );

  const region: RegionRect | null = useMemo(() => {
    if (regionMode === "full") return null;
    if (regionMode === "custom") {
      return {
        x: 0,
        y: 0,
        width: Math.max(1, Math.round(customWidth) || 1),
        height: Math.max(1, Math.round(customHeight) || 1),
      };
    }
    const preset = REGION_PRESETS.find((item) => item.id === regionMode);
    return preset
      ? { x: 0, y: 0, width: preset.width, height: preset.height }
      : null;
  }, [regionMode, customWidth, customHeight]);

  // ---------- Actions ----------
  const pickDisplay = useCallback(async () => {
    try {
      await openDisplayPicker();
    } catch (error) {
      toast.error(String(error));
    }
  }, []);

  const start = useCallback(async () => {
    startedAt.current = Date.now();
    try {
      await startRecording({
        displayIndex: selectedIndex,
        mic: micEnabled ? selectedMic || null : null,
        systemAudio: systemEnabled ? selectedSystem || null : null,
        region,
        fps: FPS,
        captureCursor: true,
      });
      setRecording(true);
      setElapsed(0);
      toast.success("Recording started");
    } catch (error) {
      startedAt.current = null;
      toast.error(String(error));
    }
  }, [
    micEnabled,
    region,
    selectedIndex,
    selectedMic,
    selectedSystem,
    systemEnabled,
  ]);

  const stop = useCallback(async () => {
    try {
      const result = await stopRecording();
      // Hand the take to a dedicated editor window. That closes this window,
      // so there is no post-stop state left to render here.
      await openEditor(result.id);
    } catch (error) {
      toast.error(String(error));
      setRecording(false);
      setElapsed(0);
      startedAt.current = null;
    }
  }, []);

  return (
    <AppShell
      toolbar={
        <Button
          variant="ghost"
          size="sm"
          disabled={recording}
          onClick={() =>
            void openProjectsWindow().catch((error) => toast.error(String(error)))
          }
          className="h-7 gap-1.5 rounded-md px-2 text-[12px] font-medium"
        >
          <FolderOpen className="size-3.5" />
          Projects
        </Button>
      }
    >
      <div className="flex min-h-0 flex-1 flex-col">
        <div className="flex-1 overflow-auto px-6 py-6">
          <div className="mx-auto flex w-full max-w-2xl flex-col gap-5">
            {/* Preview ------------------------------------------------ */}
            <Card className="gap-0 overflow-hidden border-border py-0 ring-1 ring-border">
              <div className="relative grid aspect-video place-items-center bg-linear-to-br from-zinc-900 via-zinc-950 to-black">
                <div className="text-center">
                  <Monitor className="mx-auto size-9 text-muted-foreground/40" />
                  <p className="mt-2 text-[12px] text-muted-foreground">
                    {activeDisplay
                      ? `${activeDisplay.name} · ${activeDisplay.width}×${activeDisplay.height}`
                      : "Select a display…"}
                  </p>
                </div>

                {/* Region indicator, proportional to the display */}
                {activeDisplay && region && (
                  <div
                    className="absolute top-3 left-3 rounded-sm border border-dashed border-[#8B5CF6]/80 bg-[#8B5CF6]/10"
                    style={{
                      width: `${Math.min(100, (region.width / activeDisplay.width) * 100)}%`,
                      height: `${Math.min(100, (region.height / activeDisplay.height) * 100)}%`,
                    }}
                  />
                )}

                {recording && (
                  <div className="absolute top-3 right-3 flex items-center gap-2 rounded-full border border-red-500/30 bg-red-500/15 px-2.5 py-1 backdrop-blur-sm">
                    <span className="size-1.5 animate-pulse rounded-full bg-red-500" />
                    <span className="text-[11px] font-medium tabular-nums text-red-400">
                      {formatTime(elapsed)}
                    </span>
                  </div>
                )}
              </div>
            </Card>

            {/* Settings rows ------------------------------------------ */}
            <Card className="border-border py-0 ring-1 ring-border">
              {/* Display */}
              <Row
                icon={<Monitor className="size-4" />}
                label="Display"
                hint={
                  activeDisplay
                    ? `${activeDisplay.width}×${activeDisplay.height}`
                    : undefined
                }
              >
                <Button
                  variant="outline"
                  size="sm"
                  onClick={pickDisplay}
                  className="w-56 justify-between"
                >
                  <span className="truncate">
                    {activeDisplay ? activeDisplay.name : "Choose display"}
                  </span>
                  <ChevronRight className="size-3.5 shrink-0 opacity-60" />
                </Button>
              </Row>

              <Separator />

              {/* Microphone */}
              <Row
                icon={<Mic className="size-4" />}
                label="Microphone"
                toggle={
                  <Switch checked={micEnabled} onCheckedChange={setMicEnabled} />
                }
              >
                <Select
                  value={selectedMic}
                  onValueChange={(value) => setSelectedMic(value ?? "")}
                  disabled={!micEnabled || mics.length === 0}
                >
                  <SelectTrigger className="h-8 w-56" size="sm">
                    <SelectValue placeholder="No input devices" />
                  </SelectTrigger>
                  <SelectContent>
                    {mics.map((mic) => (
                      <SelectItem key={mic.id} value={mic.id}>
                        {mic.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </Row>

              <Separator />


              {/* System audio */}
              <Row
                icon={<Volume2 className="size-4" />}
                label="System audio"
                hint="macOS needs BlackHole · brew install --cask blackhole-2ch"
                toggle={
                  <Switch
                    checked={systemEnabled}
                    onCheckedChange={setSystemEnabled}
                  />
                }
              >
                <Select
                  value={selectedSystem}
                  onValueChange={(value) => setSelectedSystem(value ?? "")}
                  disabled={!systemEnabled || systemDevices.length === 0}
                >
                  <SelectTrigger className="h-8 w-56" size="sm">
                    <SelectValue placeholder="BlackHole not installed" />
                  </SelectTrigger>
                  <SelectContent>
                    {systemDevices.map((device) => (
                      <SelectItem key={device.id} value={device.id}>
                        {device.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </Row>

              <Separator />

              {/* Region */}
              <Row icon={<Crop className="size-4" />} label="Region">
                <div className="flex items-center gap-2">
                  <SegmentGroup
                    value={regionMode}
                    options={REGION_OPTIONS}
                    onChange={setRegionMode}
                  />
                  {regionMode === "custom" && (
                    <div className="flex items-center gap-1.5">
                      <NumberField
                        value={customWidth}
                        onChange={setCustomWidth}
                        aria-label="Custom width"
                      />
                      <span className="text-muted-foreground">×</span>
                      <NumberField
                        value={customHeight}
                        onChange={setCustomHeight}
                        aria-label="Custom height"
                      />
                    </div>
                  )}
                </div>
              </Row>
            </Card>
          </div>
        </div>

        {/* Record button ---------------------------------------------- */}
        <footer className="shrink-0 border-t border-border/80 bg-background/80 px-6 py-3.5 backdrop-blur-xl">
          <div className="mx-auto flex max-w-2xl justify-center">
            <Button
              onClick={recording ? stop : start}
              size="lg"
              className={cn(
                "h-11 gap-2 rounded-full px-7 text-[13px] font-semibold shadow-lg",
                recording
                  ? "bg-red-500/15 text-red-400 shadow-red-500/10 hover:bg-red-500/25"
                  : "bg-[#8B5CF6] text-white shadow-[#8B5CF6]/25 hover:bg-[#7C3AED]",
              )}
            >
              {recording ? (
                <>
                  <Square className="size-3.5 fill-current" />
                  Stop · {formatTime(elapsed)}
                </>
              ) : (
                <>
                  <Circle className="size-3.5 fill-current" />
                  Start recording
                </>
              )}
            </Button>
          </div>
        </footer>
      </div>

      <Toaster />
    </AppShell>
  );
}


// ---------- Presentational helpers ----------

/** A compact settings row: icon, label/hint, optional toggle, then controls. */
function Row({
  icon,
  label,
  hint,
  toggle,
  children,
}: {
  icon: ReactNode;
  label: string;
  hint?: string;
  toggle?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className="flex items-center gap-3.5 px-3.5 py-3">
      <div className="grid size-8 shrink-0 place-items-center rounded-lg bg-secondary text-muted-foreground">
        {icon}
      </div>
      <div className="min-w-0 flex-1">
        <Label className="text-[13px] font-medium">{label}</Label>
        {hint && (
          <p className="mt-0.5 text-[11px] text-muted-foreground">{hint}</p>
        )}
      </div>
      {toggle && <div className="mr-0.5">{toggle}</div>}
      {children}
    </div>
  );
}

/** Small pill-style radio group used for the region presets. */
function SegmentGroup<T extends string>({
  value,
  options,
  onChange,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
}) {
  return (
    <div className="inline-flex items-center gap-0.5 rounded-lg bg-secondary/60 p-0.5">
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          onClick={() => onChange(option.value)}
          className={cn(
            "rounded-md px-2.5 py-1 text-[11px] font-medium whitespace-nowrap transition-colors",
            value === option.value
              ? "bg-background text-foreground shadow-sm"
              : "text-muted-foreground hover:text-foreground",
          )}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

/** Numeric input for the custom region width/height. */
function NumberField({
  value,
  onChange,
  ...props
}: {
  value: number;
  onChange: (value: number) => void;
} & Omit<ComponentProps<"input">, "value" | "onChange">) {
  return (
    <input
      type="number"
      inputMode="numeric"
      min={1}
      value={Number.isFinite(value) ? value : ""}
      onChange={(event) => onChange(Number(event.target.value))}
      className="h-8 w-[4.5rem] rounded-lg border border-input bg-transparent px-2 text-[12px] tabular-nums outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50"
      {...props}
    />
  );
}

function formatTime(seconds: number) {
  const minutes = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  const tenths = Math.floor((seconds % 1) * 10);
  return `${minutes}:${String(secs).padStart(2, "0")}.${tenths}`;
}

