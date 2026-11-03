/**
 * Small building blocks in the MindGraph look: dark, compact, bordered
 * panels and monospace labels. No component library.
 */

import type { ButtonHTMLAttributes, ReactNode } from "react";

import { type ConfidenceLevel, confidenceLevel } from "@/lib/builder/draft";
import { cn } from "@/lib/utils";

type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";

const BUTTON: Record<ButtonVariant, string> = {
  primary: "bg-accent text-white hover:bg-accent/90 disabled:bg-accent/40",
  secondary: "border border-border bg-panel hover:border-muted disabled:opacity-50",
  ghost: "text-muted hover:text-foreground disabled:opacity-40",
  danger: "text-danger hover:bg-danger/10 disabled:opacity-40",
};

export function Button({
  variant = "secondary",
  className,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: ButtonVariant }) {
  return (
    <button
      type="button"
      className={cn(
        "inline-flex items-center gap-1.5 rounded px-3 py-1.5 text-sm font-medium transition-colors disabled:cursor-not-allowed",
        BUTTON[variant],
        className,
      )}
      {...props}
    />
  );
}

export function Panel({
  title,
  actions,
  children,
  className,
}: {
  title?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("rounded-md border border-border bg-panel", className)}>
      {(title || actions) && (
        <header className="flex items-center justify-between gap-2 border-b border-border px-4 py-2">
          <h2 className="font-mono text-xs uppercase tracking-wider text-muted">{title}</h2>
          <div className="flex items-center gap-1">{actions}</div>
        </header>
      )}
      <div className="p-4">{children}</div>
    </section>
  );
}

type BadgeTone = "neutral" | "accent" | "success" | "warning" | "danger";

const BADGE: Record<BadgeTone, string> = {
  neutral: "border-border text-muted",
  accent: "border-accent/50 text-accent",
  success: "border-success/50 text-success",
  warning: "border-warning/50 text-warning",
  danger: "border-danger/50 text-danger",
};

export function Badge({ tone = "neutral", children }: { tone?: BadgeTone; children: ReactNode }) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded border px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wide",
        BADGE[tone],
      )}
    >
      {children}
    </span>
  );
}

const LEVEL_TONE: Record<ConfidenceLevel, BadgeTone> = {
  high: "success",
  medium: "neutral",
  low: "warning",
};

/** Model confidence, shown as a band and a percentage. Never a guarantee. */
export function ConfidenceBadge({ value }: { value: number | null }) {
  if (value === null) return null;
  const level = confidenceLevel(value);
  return (
    <span title={`Model confidence (${level}); not a guarantee`}>
      <Badge tone={LEVEL_TONE[level]}>{Math.round(value * 100)}%</Badge>
    </span>
  );
}

export function ProgressBar({ value, label }: { value: number; label: string }) {
  const percent = Math.round(Math.min(Math.max(value, 0), 1) * 100);
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent}
      className="h-1.5 w-full overflow-hidden rounded bg-border"
    >
      <div className="h-full bg-accent transition-[width]" style={{ width: `${percent}%` }} />
    </div>
  );
}

export function ErrorNote({ children }: { children: ReactNode }) {
  return (
    <p
      role="alert"
      className="rounded border border-danger/40 bg-danger/10 px-3 py-2 text-sm text-danger"
    >
      {children}
    </p>
  );
}

export const inputClass =
  "w-full rounded border border-border bg-background px-2 py-1 text-sm text-foreground outline-none focus:border-accent disabled:text-muted";
