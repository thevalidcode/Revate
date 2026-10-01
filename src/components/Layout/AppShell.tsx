import type { ReactNode } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { Settings2, ArrowLeft } from "lucide-react";

import { Wordmark } from "@/components/Brand/Logo";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

export interface AppShellProps {
  children: ReactNode;
  className?: string;
  /** Show the back arrow (settings → recorder). */
  showBack?: boolean;
}

/**
 * Revate application shell.
 *
 * - 44px draggable title bar (`data-tauri-drag-region`) with the brand mark on
 *   the left and the settings action on the right.
 * - Compact desktop density: 13px base text, 8–12px radii, 12–14px row padding
 *   (row padding lives with each settings row, see `Recorder`).
 *
 * Every page should render inside `<AppShell>` so the chrome stays consistent.
 */
export function AppShell({ children, className, showBack = false }: AppShellProps) {
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const onSettings = pathname.startsWith("/settings");

  return (
    <div className="flex h-screen flex-col overflow-hidden bg-background text-[13px] text-foreground">
      <header
        data-tauri-drag-region
        className="flex h-11 shrink-0 select-none items-center justify-between gap-2 border-b border-border/80 bg-background/80 px-3 backdrop-blur-xl"
      >
        <div data-tauri-drag-region className="flex items-center gap-2">
          {showBack && (
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="Back to recorder"
              className="-ml-1"
              onClick={() => navigate("/recorder")}
            >
              <ArrowLeft className="size-4" />
            </Button>
          )}
          <span data-tauri-drag-region className="flex items-center">
            <Wordmark size={20} />
          </span>
        </div>

        <div data-tauri-drag-region className="flex items-center gap-1">
          <Button
            variant={onSettings ? "secondary" : "ghost"}
            size="icon-sm"
            aria-label="Settings"
            title="Settings"
            onClick={() => navigate(onSettings ? "/recorder" : "/settings")}
          >
            <Settings2 className="size-4" />
          </Button>
        </div>
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
