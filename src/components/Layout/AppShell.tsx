import type { ReactNode } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { Settings } from "lucide-react";

import { Logo } from "@/components/Brand/Logo";
import { cn } from "@/lib/utils";

export interface AppShellProps {
  children: ReactNode;
  className?: string;
}

/**
 * Revate application shell — one compact title strip plus the page body.
 *
 * The macOS title bar is an overlay (`titleBarStyle: "Overlay"` + `hiddenTitle`
 * in tauri.conf.json), so the traffic lights float over this 40px header. The
 * `pl-[78px]` gutter reserves their width, and `data-tauri-drag-region` keeps
 * the strip draggable.
 *
 * Tauri's drag handler only starts a drag for a *bare* attribute when the click
 * lands directly on the element that carries it, so the logo/wordmark wrapper
 * repeats the attribute — while the settings button (a clickable tag) is left
 * without it and therefore keeps its normal click behaviour.
 *
 * Contents are deliberately minimal: brand mark + wordmark on the left, one
 * settings gear on the right. No menus, tabs or title text.
 */
export function AppShell({ children, className }: AppShellProps) {
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const onSettings = pathname.startsWith("/settings");

  return (
    <div className="flex h-screen flex-col overflow-hidden bg-background text-[13px] text-foreground">
      <header
        data-tauri-drag-region
        className="flex h-10 shrink-0 items-center justify-between border-b border-border pr-1.5 pl-[78px] select-none"
      >
        <div data-tauri-drag-region className="flex items-center gap-2">
          <Logo size={20} />
          <span
            data-tauri-drag-region
            className="text-[12px] leading-none font-semibold tracking-tight"
          >
            Revate
          </span>
        </div>

        <button
          type="button"
          aria-label={onSettings ? "Back to recorder" : "Settings"}
          title={onSettings ? "Back to recorder" : "Settings"}
          onClick={() => navigate(onSettings ? "/recorder" : "/settings")}
          className={cn(
            "grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors",
            "hover:bg-secondary hover:text-foreground",
            "focus-visible:ring-2 focus-visible:ring-ring/60 focus-visible:outline-none",
            onSettings && "bg-secondary text-foreground",
          )}
        >
          <Settings className="size-3.5" />
        </button>
      </header>

      <main
        className={cn(
          // Pages own their own scrolling so they can pin footers/headers.
          "flex min-h-0 flex-1 flex-col overflow-hidden",
          className,
        )}
      >
        {children}
      </main>
    </div>
  );
}

export default AppShell;

