"use client";

/**
 * "Create a Graph" (mockup 03): a name, the storage, and how to start.
 * CSV and JSON lead to the upload step; "Start Empty" opens the ontology
 * editor without data; "Explore Demo" opens the demo scenarios (`/demos`)
 * without creating a graph, because `make demo-<name>` creates its own.
 */

import {
  ArrowRight,
  Braces,
  Command,
  Lock,
  type LucideIcon,
  Play,
  Sheet,
  SquareDashed,
} from "lucide-react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { type FormEvent, useMemo, useState } from "react";

import { AppShell } from "@/components/shell/AppShell";
import { Button, ErrorNote, Field } from "@/components/ui/primitives";
import { type ApiClient, ApiError, createApiClient } from "@/lib/api/client";
import { cn } from "@/lib/utils";

type Start = "csv" | "json" | "empty" | "demo";

interface Option {
  id: Start;
  title: string;
  description: string;
  tag: string;
  icon: LucideIcon;
}

const OPTIONS: Option[] = [
  {
    id: "csv",
    title: "Upload CSV",
    description: "Build a graph from tabular data.",
    tag: ".csv · multiple files",
    icon: Sheet,
  },
  {
    id: "json",
    title: "Upload JSON",
    description: "Build from structured JSON.",
    tag: ".json · arrays of flat objects",
    icon: Braces,
  },
  {
    id: "empty",
    title: "Start Empty",
    description: "Define your own ontology.",
    tag: "entities + relationships",
    icon: SquareDashed,
  },
  {
    id: "demo",
    title: "Explore Demo",
    description: "Try MindGraph with sample data.",
    tag: "fraud · knowledge · cyber",
    icon: Play,
  },
];

const NEXT_STEPS = [
  ["Upload", "Drop CSV or JSON files. Headers and types are parsed locally."],
  ["Detect", "MindGraph proposes entities, properties and relationships."],
  ["Review", "Rename, remove or remap anything in the ontology."],
  ["Generate", "Your graph is built from the reviewed model."],
];

export function CreateGraph({ api: injected }: { api?: ApiClient }) {
  const api = useMemo(() => injected ?? createApiClient(), [injected]);
  const router = useRouter();
  const [name, setName] = useState("");
  const [start, setStart] = useState<Start>("csv");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const demo = start === "demo";

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (demo) {
      router.push("/demos");
      return;
    }
    if (!name.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      const graph = await api.createGraph(name.trim());
      router.push(
        start === "empty"
          ? `/graphs/${graph.id}/builder`
          : `/graphs/${graph.id}/upload?kind=${start}`,
      );
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
      setBusy(false);
    }
  }

  return (
    <AppShell>
      <div className="flex min-h-screen">
        <form onSubmit={submit} className="flex min-w-0 flex-1 flex-col gap-8 px-14 py-12">
          <header className="flex flex-col gap-2">
            <span className="font-mono text-[10px] font-medium tracking-[0.8px] text-accent">
              NEW GRAPH · STEP 1 OF 4
            </span>
            <h1 className="text-[32px] font-bold">Create a Graph</h1>
            <p className="text-[16px] text-text-2">How would you like to start?</p>
          </header>

          <div className="flex flex-wrap gap-4">
            <Field label="Graph name" className="w-[320px]">
              <input
                autoFocus
                maxLength={120}
                placeholder={demo ? "not needed for a demo" : "fraud-investigation"}
                value={name}
                disabled={demo}
                onChange={(event) => setName(event.target.value)}
                className="h-9 rounded-[4px] border border-border-strong bg-panel px-3 font-mono text-[12px] font-medium text-text outline-none focus:border-accent disabled:text-text-3"
              />
            </Field>
            <div className="flex w-[320px] flex-col gap-1">
              <span className="text-[12px] text-text-3">Storage</span>
              <span className="flex h-9 items-center gap-2 rounded-[4px] border border-border bg-panel px-3">
                <span className="flex-1 truncate font-mono text-[12px] font-medium">
                  Local HugeGraph (bundled)
                </span>
                <span className="shrink-0 font-mono text-[10px] text-text-3">only option</span>
              </span>
            </div>
          </div>

          <div
            role="radiogroup"
            aria-label="How to start"
            className="grid w-full max-w-[768px] grid-cols-1 overflow-hidden rounded-[8px] border border-border sm:grid-cols-2"
          >
            {OPTIONS.map((option, index) => {
              const Icon = option.icon;
              const selected = option.id === start;
              return (
                <button
                  key={option.id}
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  onClick={() => setStart(option.id)}
                  className={cn(
                    "flex h-[208px] flex-col gap-[22px] px-[26px] py-8 text-left",
                    selected ? "border-l-2 border-l-accent bg-raised" : "bg-bg hover:bg-panel",
                    index % 2 === 1 && !selected && "sm:border-l sm:border-border",
                    index >= 2 && "border-t border-border",
                  )}
                >
                  <span className="flex w-full items-center">
                    <span
                      className={cn(
                        "flex h-9 w-9 items-center justify-center rounded-[6px]",
                        selected
                          ? "bg-accent-soft text-accent"
                          : "border border-border-strong text-text-2",
                      )}
                    >
                      <Icon size={17} />
                    </span>
                    <span className="flex-1" />
                    <span
                      className={cn(
                        "h-[14px] w-[14px] rounded-full",
                        selected
                          ? "shadow-[inset_0_0_0_4.5px_var(--accent)]"
                          : "shadow-[inset_0_0_0_1.5px_var(--border-strong)]",
                      )}
                    />
                  </span>
                  <span className="flex flex-col gap-[3px]">
                    <span className="text-[17px] font-bold text-text">{option.title}</span>
                    <span className="text-[13.5px] text-text-2">{option.description}</span>
                  </span>
                  <span className="font-mono text-[10.5px] font-medium text-text-3">
                    {option.tag}
                  </span>
                </button>
              );
            })}
          </div>

          {error && <ErrorNote>{error}</ErrorNote>}

          <div className="flex-1" />
          <footer className="flex w-full max-w-[768px] items-center gap-[10px] border-t border-border pt-4">
            <Lock size={13} className="shrink-0 text-text-3" />
            <span className="flex-1 text-[12.5px] text-text-3">
              Files stay on this machine. AI suggestions send only a small profile to the model you
              configure.
            </span>
            <Link
              href="/"
              className="inline-flex h-8 items-center rounded-[4px] border border-border-strong bg-raised px-3 text-[13px] font-semibold hover:bg-hover"
            >
              Cancel
            </Link>
            <Button
              type="submit"
              variant="primary"
              disabled={(!demo && !name.trim()) || busy}
              icon={<ArrowRight size={14} />}
            >
              {busy ? "Creating…" : demo ? "See demo scenarios" : "Continue"}
            </Button>
          </footer>
        </form>

        <aside className="hidden w-[340px] shrink-0 flex-col gap-5 border-l border-border bg-panel px-7 py-12 lg:flex">
          <span className="font-mono text-[10px] font-medium tracking-[0.8px] text-text-3">
            WHAT HAPPENS NEXT
          </span>
          {NEXT_STEPS.map(([title, description], index) => (
            <div key={title} className="flex gap-[14px]">
              <span
                className={cn(
                  "flex h-6 w-6 shrink-0 items-center justify-center rounded-full font-mono text-[10.5px] font-medium",
                  index === 0
                    ? "text-accent shadow-[inset_0_0_0_1px_var(--accent)]"
                    : "text-text-2 shadow-[inset_0_0_0_1px_var(--border-strong)]",
                )}
              >
                {index + 1}
              </span>
              <div className="flex flex-col gap-[3px]">
                <span className="text-[14px] font-bold">{title}</span>
                <span className="text-[12.5px] leading-[18px] text-text-3">{description}</span>
              </div>
            </div>
          ))}
          <div className="flex-1" />
          <div className="flex flex-col gap-1.5 rounded-[6px] border border-border p-[14px]">
            <span className="flex items-center gap-2 text-[12.5px] font-semibold">
              <Command size={13} className="text-text-2" /> Tip
            </span>
            <span className="text-[12.5px] leading-[18px] text-text-3">
              Upload several related files at once: MindGraph models them together and links them
              across files.
            </span>
          </div>
        </aside>
      </div>
    </AppShell>
  );
}
