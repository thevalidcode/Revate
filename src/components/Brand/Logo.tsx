import { cn } from "cn";

/**
 * Revate brand mark: a white "R" on a violet (#8B5CF6) rounded square with a
 * small red dot in the corner that doubles as a "recording" indicator.
 *
 * Kept intentionally size-driven (no Tailwind sizing classes) so it can be
 * reused at 16px in the app chrome and at 512px on a splash screen with the
 * exact same proportions.
 */
export interface LogoProps {
  /** Edge length of the rounded square in px. */
  size?: number;
  className?: string;
  /** Hide the record dot (e.g. on a mono/tray variant). */
  withDot?: boolean;
  /** Pulse the dot while a recording is in progress. */
  recording?: boolean;
}

export function Logo({
  size = 24,
  className,
  withDot = true,
  recording = false,
}: LogoProps) {
  const dot = Math.max(5, Math.round(size * 0.26));
  const offset = Math.round(size * 0.07);

  return (
    <span
      className={cn(
        "relative inline-grid shrink-0 place-items-center rounded-[28%] bg-[#8B5CF6] text-white shadow-sm",
        className,
      )}
      style={{ width: size, height: size }}
      aria-hidden="true"
    >
      <span
        className="font-semibold leading-none tracking-tight"
        style={{ fontSize: Math.round(size * 0.58), marginTop: -Math.round(size * 0.02) }}
      >
        R
      </span>
      {withDot && (
        <span
          className={cn(
            "absolute rounded-full bg-[#EF4444] ring-2 ring-background",
            recording && "animate-pulse",
          )}
          style={{
            width: dot,
            height: dot,
            top: -offset,
            right: -offset,
          }}
        />
      )}
    </span>
  );
}

/** Logo + wordmark, used in headers. */
export function Wordmark({
  size = 20,
  className,
  textClassName,
  recording,
}: LogoProps & { textClassName?: string }) {
  return (
    <span className={cn("inline-flex items-center gap-2", className)}>
      <Logo size={size} recording={recording} />
      <span
        className={cn("font-semibold tracking-tight", textClassName)}
        style={{ fontSize: Math.round(size * 0.65) }}
      >
        Revate
      </span>
    </span>
  );
}

export default Logo;
