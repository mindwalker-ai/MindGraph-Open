/**
 * Building blocks in the MindGraph mockup's look: dark, compact panels,
 * Titillium Web labels and IBM Plex Mono data. No component library.
 */

import Image from "next/image";
import type { ButtonHTMLAttributes, ReactNode } from "react";

import { cn } from "@/lib/utils";

type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";
type ButtonSize = "sm" | "md" | "lg";

const BUTTON: Record<ButtonVariant, string> = {
  primary: "bg-accent text-on-accent hover:bg-accent/90 disabled:bg-accent/40",
  secondary:
    "border border-border-strong bg-raised text-text hover:bg-hover disabled:text-text-3 disabled:hover:bg-raised",
  ghost: "text-text-2 hover:text-text disabled:text-text-3",
  danger: "text-danger hover:text-danger/80 disabled:text-text-3",
};

const SIZE: Record<ButtonSize, string> = {
  sm: "h-[26px] gap-[5px] px-2 text-[12px]",
  md: "h-8 gap-2 px-3 text-[13px]",
  lg: "h-9 gap-2 px-[18px] text-[14px]",
};

export function Button({
  variant = "secondary",
  size = "md",
  icon,
  className,
  children,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: ButtonVariant;
  size?: ButtonSize;
  icon?: ReactNode;
}) {
  return (
    <button
      type="button"
      className={cn(
        "inline-flex shrink-0 items-center justify-center rounded-[4px] font-semibold whitespace-nowrap transition-colors disabled:cursor-not-allowed",
        variant !== "ghost" && variant !== "danger" && SIZE[size],
        (variant === "ghost" || variant === "danger") && "gap-1.5 text-[12.5px]",
        BUTTON[variant],
        className,
      )}
      {...props}
    >
      {icon}
      {children}
    </button>
  );
}

/** A feature shown as in the design but not built yet. */
export function comingIn(milestone?: string): string {
  return milestone ? `Coming in ${milestone}` : "Coming soon";
}

/** The MindGraph mark from the design. */
export function Mark({ size = 22 }: { size?: number }) {
  return (
    <Image
      src="/mark.png"
      alt=""
      width={size}
      height={Math.round((size * 156) / 178)}
      priority
      className="shrink-0"
    />
  );
}

/** Small uppercase monospace label, e.g. "DEFINITION". */
export function Kicker({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <span
      className={cn(
        "font-mono text-[10px] font-medium tracking-[0.6px] text-text-3 uppercase",
        className,
      )}
    >
      {children}
    </span>
  );
}

type BadgeTone = "neutral" | "accent" | "ok" | "warn" | "danger";

const BADGE: Record<BadgeTone, string> = {
  neutral: "border-border-strong text-text-3",
  accent: "border-accent/60 text-accent",
  ok: "border-ok/50 text-ok",
  warn: "border-warn/50 text-warn",
  danger: "border-danger/50 text-danger",
};

export function Badge({ tone = "neutral", children }: { tone?: BadgeTone; children: ReactNode }) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-[3px] border px-[5px] py-px font-mono text-[9px] font-medium tracking-[0.6px] uppercase",
        BADGE[tone],
      )}
    >
      {children}
    </span>
  );
}

/** A thin meter, as the confidence bar of the design. */
export function Meter({ value, label }: { value: number; label: string }) {
  const percent = Math.round(Math.min(Math.max(value, 0), 1) * 100);
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent}
      className="h-1 w-full overflow-hidden rounded-[2px] bg-hover"
    >
      <div
        className="h-full rounded-[2px] bg-accent transition-[width]"
        style={{ width: `${percent}%` }}
      />
    </div>
  );
}

export function ErrorNote({ children }: { children: ReactNode }) {
  return (
    <p
      role="alert"
      className="rounded-[4px] border border-danger/40 bg-danger/10 px-3 py-2 text-[12.5px] text-danger"
    >
      {children}
    </p>
  );
}

/** Text inputs and selects of the design (dark well, monospace value). */
export const inputClass =
  "h-8 w-full rounded-[4px] border border-border-strong bg-bg px-[10px] font-mono text-[11.5px] text-text outline-none focus:border-accent disabled:text-text-3";

/** A labelled form field. */
export function Field({
  label,
  children,
  className,
}: {
  label: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <label className={cn("flex w-full flex-col gap-1", className)}>
      <span className="text-[12px] text-text-3">{label}</span>
      {children}
    </label>
  );
}
