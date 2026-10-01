"use client"

import type { CSSProperties } from "react"
import { Toaster as Sonner, type ToasterProps } from "sonner"
import { Check, Info, Loader2, TriangleAlert, X } from "lucide-react"

/**
 * Revate toast surface — compact desktop styling layered over Sonner.
 *
 * Sonner's built-in look is switched off entirely with `unstyled: true`, which
 * stamps the toast with `data-styled="false"` and drops the
 * `[data-sonner-toast][data-styled='true']` rule. That matters: those selectors
 * are two attribute selectors deep (specificity 0-2-0) and would out-specify
 * any Tailwind utility passed through `classNames`, silently pinning padding to
 * 16px, font-size to 13px and gap to 6px.
 *
 * With the defaults gone we own the whole card: 300px wide, 12px text,
 * 10x12px padding, 14px icon, 8px gap, single-line messages, and a
 * violet-tinted `#8B5CF6` border on hover.
 *
 * Only Sonner's structural rules remain (stacking, lift, enter/exit transforms)
 * because those are what make a toast list behave.
 */
const ICON = "size-3.5 shrink-0"

const Toaster = ({ ...props }: ToasterProps) => (
  <Sonner
    theme="dark"
    position="bottom-right"
    offset={16}
    gap={8}
    duration={2500}
    visibleToasts={3}
    closeButton={false}
    expand={false}
    icons={{
      success: <Check className={`${ICON} text-emerald-400`} strokeWidth={2.75} />,
      error: <X className={`${ICON} text-red-400`} strokeWidth={2.75} />,
      info: <Info className={`${ICON} text-muted-foreground`} strokeWidth={2.5} />,
      warning: (
        <TriangleAlert className={`${ICON} text-amber-400`} strokeWidth={2.5} />
      ),
      loading: (
        <Loader2 className={`${ICON} animate-spin text-muted-foreground`} />
      ),
    }}
    style={
      {
        // Sonner hard-codes its own font-family on the container; `inherit`
        // pulls the app font (Integral CF) back in. `--width` keeps the
        // stacking column the same width as the toast card.
        fontFamily: "inherit",
        "--width": "300px",
      } as CSSProperties
    }
    toastOptions={{
      duration: 2500,
      unstyled: true,
      classNames: {
        toast:
          "group flex w-[300px] items-center gap-2 rounded-lg border border-border bg-card px-3 py-2.5 text-card-foreground shadow-lg transition-colors hover:border-[#8B5CF6]/50",
        icon: "flex size-3.5 shrink-0 items-center justify-center [&_svg]:size-3.5",
        content: "min-w-0 flex-1",
        title: "truncate text-[12px] leading-snug font-medium",
        description: "truncate text-[11px] text-muted-foreground",
      },
    }}
    {...props}
  />
)

export { Toaster }

