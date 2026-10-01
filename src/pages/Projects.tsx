import { useCallback, useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { confirm } from "@tauri-apps/plugin-dialog";
import { FolderOpen, Pencil, RefreshCw, Trash2, Video } from "lucide-react";
import { toast } from "sonner";

import { AppShell } from "@/components/Layout/AppShell";
import { Button } from "@/components/ui/button";
import { Toaster } from "@/components/ui/sonner";
import { cn } from "@/lib/utils";
import {
  assetUrl,
  deleteProject,
  listProjects,
  makeThumbnail,
  openProjectInEditor,
  renameProject,
} from "@/lib/tauri";
import type { ProjectInfo } from "@/types/events";

function formatDuration(ms: number) {
  const total = Math.round(ms / 1000);
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

function formatSize(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

/**
 * Projects window body — rendered in its own webview on
 * `index.html?projects=1` (see `App.tsx`).
 *
 * One compact row per session folder, newest first. Posters are generated
 * lazily: a row that arrives without a `thumb.jpg` asks for one exactly once.
 */
export default function Projects() {
  const [items, setItems] = useState<ProjectInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [renaming, setRenaming] = useState<string | null>(null);
  const requestedThumbs = useRef(new Set<string>());

  const load = useCallback(async () => {
    try {
      setItems(await listProjects());
    } catch (error) {
      toast.error(String(error));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // Lazily fill in missing posters, once per session.
  useEffect(() => {
    for (const item of items) {
      if (item.thumbPath || requestedThumbs.current.has(item.id)) continue;
      requestedThumbs.current.add(item.id);

      void makeThumbnail(item.id)
        .then((path) => {
          if (!path) return;
          setItems((prev) =>
            prev.map((row) =>
              row.id === item.id ? { ...row, thumbPath: path } : row,
            ),
          );
        })
        .catch(() => {
          /* a missing poster is not worth a toast */
        });
    }
  }, [items]);

  const open = useCallback((id: string) => {
    void openProjectInEditor(id).catch((error) => toast.error(String(error)));
  }, []);

  const commitRename = useCallback(async (item: ProjectInfo, next: string) => {
    setRenaming(null);
    const name = next.trim();
    if (!name || name === item.id) return;

    try {
      const actual = await renameProject(item.id, name);
      setItems((prev) =>
        prev.map((row) =>
          row.id === item.id ? { ...row, id: actual, name: actual } : row,
        ),
      );
      toast.success("Renamed");
    } catch (error) {
      toast.error(String(error));
    }
  }, []);

  const remove = useCallback(async (item: ProjectInfo) => {
    const yes = await confirm(`Delete “${item.name}”? This cannot be undone.`, {
      title: "Delete recording",
      kind: "warning",
    });
    if (!yes) return;

    try {
      await deleteProject(item.id);
      setItems((prev) => prev.filter((row) => row.id !== item.id));
      toast.success("Deleted");
    } catch (error) {
      toast.error(String(error));
    }
  }, []);

  return (
    <AppShell
      label="Projects"
      actions={
        <Button
          variant="ghost"
          size="sm"
          onClick={load}
          className="h-7 gap-1.5 rounded-md px-2 text-[12px] font-medium"
        >
          <RefreshCw className="size-3.5" />
          Refresh
        </Button>
      }
    >
      <div className="min-h-0 flex-1 overflow-auto px-4 py-4">
        <div className="mx-auto w-full max-w-3xl">
          {loading ? (
            <p className="py-12 text-center text-[12px] text-muted-foreground">
              Loading…
            </p>
          ) : items.length === 0 ? (
            <p className="py-16 text-center text-[13px] text-muted-foreground">
              📁 No projects yet — record something first.
            </p>
          ) : (
            <ul className="divide-y divide-border overflow-hidden rounded-lg border border-border">
              {items.map((item) => (
                <ProjectRow
                  key={item.id}
                  item={item}
                  renaming={renaming === item.id}
                  onOpen={() => open(item.id)}
                  onRenameStart={() => setRenaming(item.id)}
                  onRenameCommit={(next) => void commitRename(item, next)}
                  onRenameCancel={() => setRenaming(null)}
                  onDelete={() => void remove(item)}
                />
              ))}
            </ul>
          )}
        </div>
      </div>

      <Toaster />
    </AppShell>
  );
}

/** One 48px project row. The whole row opens the project; actions appear on hover. */
function ProjectRow({
  item,
  renaming,
  onOpen,
  onRenameStart,
  onRenameCommit,
  onRenameCancel,
  onDelete,
}: {
  item: ProjectInfo;
  renaming: boolean;
  onOpen: () => void;
  onRenameStart: () => void;
  onRenameCommit: (next: string) => void;
  onRenameCancel: () => void;
  onDelete: () => void;
}) {
  const [draft, setDraft] = useState(item.name);

  useEffect(() => {
    setDraft(item.name);
  }, [item.name]);

  return (
    <li className="group relative flex h-12 items-center gap-3 px-3 transition-colors hover:bg-secondary/60">
      {/* Full-row click target, sitting under the content below. */}
      <button
        type="button"
        onClick={onOpen}
        aria-label={`Open ${item.name} in the editor`}
        className="absolute inset-0 rounded-none"
      />

      <div className="relative z-10 flex h-8 w-14 shrink-0 items-center justify-center overflow-hidden rounded bg-secondary">
        {item.thumbPath ? (
          <img
            src={assetUrl(item.thumbPath)}
            alt=""
            className="size-full object-cover"
          />
        ) : (
          <Video className="size-3.5 text-muted-foreground/50" />
        )}
      </div>

      <div className="relative z-10 min-w-0 flex-1">
        {renaming ? (
          <input
            autoFocus
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onBlur={() => onRenameCommit(draft)}
            onKeyDown={(event) => {
              if (event.key === "Enter") onRenameCommit(draft);
              if (event.key === "Escape") onRenameCancel();
            }}
            className="h-5 w-full rounded border border-ring bg-background px-1 text-[12px] outline-none"
          />
        ) : (
          <p className="truncate text-[12px] leading-tight font-medium">
            {item.name}
          </p>
        )}
        <p className="truncate text-[11px] leading-tight tabular-nums text-muted-foreground">
          {formatDuration(item.durationMs)} · {formatSize(item.sizeBytes)} ·{" "}
          {item.width}×{item.height}
        </p>
      </div>

      <div className="relative z-10 flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
        <RowAction label="Open in editor" onClick={onOpen}>
          <FolderOpen className="size-3.5" />
        </RowAction>
        <RowAction label="Rename" onClick={onRenameStart}>
          <Pencil className="size-3.5" />
        </RowAction>
        <RowAction label="Delete" danger onClick={onDelete}>
          <Trash2 className="size-3.5" />
        </RowAction>
      </div>
    </li>
  );
}

function RowAction({
  label,
  danger,
  onClick,
  children,
}: {
  label: string;
  danger?: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={label}
      aria-label={label}
      className={cn(
        "grid size-7 place-items-center rounded-md text-muted-foreground transition-colors",
        "hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/60 focus-visible:outline-none",
        danger
          ? "hover:bg-destructive/15 hover:text-destructive"
          : "hover:bg-secondary",
      )}
    >
      {children}
    </button>
  );
}
