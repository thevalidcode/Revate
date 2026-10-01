import { useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";

import { cn } from "@/lib/utils";

export interface CircularProgressProps {
  /** Target progress, 0–100. Values outside the range are clamped. */
  percent: number;
  size?: number;
  strokeWidth?: number;
  /** Small caption under the counter, e.g. "Exporting…". */
  label?: string;
  className?: string;
  /** Fire the sparkle burst once the counter lands on 100. */
  celebrate?: boolean;
}

/** Six evenly spaced particles for the completion burst. */
const SPARKLE_ANGLES = [0, 60, 120, 180, 240, 300];

const clamp = (value: number, min: number, max: number) =>
  Math.min(max, Math.max(min, value));

/**
 * Compact circular progress readout for the editor's export flow.
 *
 * The ring is driven by a tweened copy of `percent` rather than the raw prop,
 * so even a jump from 0 → 100 animates as a sweep instead of snapping, and the
 * numeric counter glides alongside it. When the counter reaches 100 a burst of
 * violet sparkle particles fires outward.
 */
export function CircularProgress({
  percent,
  size = 132,
  strokeWidth = 6,
  label,
  className,
  celebrate = true,
}: CircularProgressProps) {
  const target = clamp(percent, 0, 100);
  const [shown, setShown] = useState(0);
  const shownRef = useRef(0);

  // Ease the displayed value toward the target and stop the loop once it lands.
  useEffect(() => {
    let frame = requestAnimationFrame(function step() {
      const diff = target - shownRef.current;
      if (Math.abs(diff) < 0.05) {
        shownRef.current = target;
        setShown(target);
        return;
      }
      shownRef.current += diff * 0.15;
      setShown(shownRef.current);
      frame = requestAnimationFrame(step);
    });

    return () => cancelAnimationFrame(frame);
  }, [target]);

  const done = shown >= 99.5;

  // Re-key the burst on every fresh completion so it replays.
  const [burst, setBurst] = useState(0);
  const wasDone = useRef(false);
  useEffect(() => {
    if (done && !wasDone.current) setBurst((n) => n + 1);
    wasDone.current = done;
  }, [done]);

  const radius = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * radius;
  const offset = circumference * (1 - shown / 100);

  return (
    <span className={cn("inline-flex flex-col items-center gap-2.5", className)}>
      <span
        className="relative inline-grid place-items-center"
        style={{ width: size, height: size }}
        role="progressbar"
        aria-valuenow={Math.round(shown)}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-label={label ?? "Progress"}
      >
        <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} className="-rotate-90">
          <circle
            cx={size / 2}
            cy={size / 2}
            r={radius}
            fill="none"
            strokeWidth={strokeWidth}
            className="text-secondary"
          />
          <circle
            cx={size / 2}
            cy={size / 2}
            r={radius}
            fill="none"
            stroke="#8B5CF6"
            strokeWidth={strokeWidth}
            strokeLinecap="round"
            strokeDasharray={circumference}
            strokeDashoffset={offset}
          />
        </svg>

        {celebrate && burst > 0 && (
          <span key={burst} className="pointer-events-none absolute inset-0">
            {SPARKLE_ANGLES.map((deg, i) => {
              const rad = (deg * Math.PI) / 180;
              const distance = size * 0.58;
              return (
                <span
                  key={deg}
                  className="absolute top-1/2 left-1/2 size-1 rounded-full bg-[#8B5CF6]"
                  style={
                    {
                      "--sx": `${Math.cos(rad) * distance}px`,
                      "--sy": `${Math.sin(rad) * distance}px`,
                      animation:
                        "revate-sparkle 900ms cubic-bezier(0.22, 1, 0.36, 1) forwards",
                      animationDelay: `${i * 45}ms`,
                    } as CSSProperties
                  }
                />
              );
            })}
          </span>
        )}

        <span className="absolute inset-0 grid place-items-center">
          <span className="text-[22px] leading-none font-semibold tabular-nums">
            {Math.round(shown)}
            <span className="text-[12px] font-medium text-muted-foreground">
              %
            </span>
          </span>
        </span>
      </span>

      {label && <span className="text-[11px] text-muted-foreground">{label}</span>}
    </span>
  );
}

export default CircularProgress;