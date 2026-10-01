import { useEffect, useState } from "react";
import { Info, MonitorSpeaker, Terminal, Volume2 } from "lucide-react";

import { AppShell } from "@/components/Layout/AppShell";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { Toaster } from "@/components/ui/sonner";

const BREW_BLACKHOLE = "brew install --cask blackhole-2ch";

/**
 * Minimal settings screen. Two things live here for now:
 *  - the system-audio (BlackHole) setup instructions, and
 *  - the raw `ffmpeg -f avfoundation -list_devices` dump, which is the quickest
 *    way to debug an empty mic / system-audio dropdown.
 */
export default function Settings() {
  const [devices, setDevices] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void (async () => {
      try {
        setDevices(await invoke<string>("list_capture_devices"));
      } catch (err) {
        setError(String(err));
      }
    })();
  }, []);

  const copyBrew = async () => {
    try {
      await navigator.clipboard.writeText(BREW_BLACKHOLE);
      toast.success("Copied");
    } catch {
      toast.error("Could not access the clipboard");
    }
  };

  return (
    <AppShell showBack>
      <div className="min-h-0 flex-1 overflow-auto px-6 py-6">
        <div className="mx-auto flex w-full max-w-2xl flex-col gap-4">
          <h1 className="text-[15px] font-semibold tracking-tight">Settings</h1>

          <Card className="border-border py-0 ring-1 ring-border">
            <div className="flex items-start gap-3.5 px-3.5 py-3">
              <div className="grid size-8 shrink-0 place-items-center rounded-lg bg-secondary text-muted-foreground">
                <Volume2 className="size-4" />
              </div>
              <div className="min-w-0 flex-1">
                <p className="text-[13px] font-medium">System audio</p>
                <p className="mt-0.5 text-[11px] text-muted-foreground">
                  macOS has no built-in loopback device. Install BlackHole, then
                  select it in the recorder's “System audio” row.
                </p>
                <code className="mt-2 inline-block rounded-md bg-secondary px-2 py-1 text-[11px]">
                  {BREW_BLACKHOLE}
                </code>
              </div>
              <Button variant="outline" size="sm" onClick={copyBrew}>
                Copy
              </Button>
            </div>

            <Separator />

            <div className="flex items-start gap-3.5 px-3.5 py-3">
              <div className="grid size-8 shrink-0 place-items-center rounded-lg bg-secondary text-muted-foreground">
                <MonitorSpeaker className="size-4" />
              </div>
              <div className="min-w-0 flex-1">
                <p className="text-[13px] font-medium">Permissions</p>
                <p className="mt-0.5 text-[11px] text-muted-foreground">
                  Revate needs <strong>Screen Recording</strong> and{" "}
                  <strong>Microphone</strong> access. Grant both in System
                  Settings → Privacy &amp; Security, then fully restart the app.
                </p>
              </div>
            </div>
          </Card>

          <h2 className="mt-2 flex items-center gap-1.5 text-[13px] font-semibold tracking-tight">
            <Terminal className="size-3.5 text-muted-foreground" />
            Capture devices
          </h2>

          <Card className="border-border py-0 ring-1 ring-border">
            <div className="flex items-start gap-2.5 px-3.5 py-3">
              <Info className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
              <p className="text-[11px] text-muted-foreground">
                Raw output of{" "}
                <code>ffmpeg -f avfoundation -list_devices true -i ""</code>.
              </p>
            </div>
            <Separator />
            <pre className="max-h-80 overflow-auto px-3.5 py-3 text-[11px] leading-relaxed whitespace-pre-wrap text-muted-foreground">
              {error ? `Error: ${error}` : devices || "Loading…"}
            </pre>
          </Card>
        </div>
      </div>

      <Toaster position="bottom-right" theme="dark" />
    </AppShell>
  );
}

