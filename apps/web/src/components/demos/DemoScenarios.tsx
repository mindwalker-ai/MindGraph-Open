"use client";

/**
 * Demo scenarios (`/demos`, M10): the three bundled datasets in
 * `examples/`, whether each one is loaded, and how to load it.
 *
 * Loading happens outside the browser on purpose: `make demo-<name>` runs
 * the `mindgraph demo` loader, which uses Core's public REST API and checks
 * the result. This page only reads: it lists graphs, finds each demo graph
 * by its exact name and links to it once it exists.
 */

import {
  ArrowRight,
  CopyCheck,
  Library,
  type LucideIcon,
  Plug,
  Radar,
  RefreshCw,
  ShieldAlert,
  Terminal,
  Waypoints,
} from "lucide-react";
import Link from "next/link";
import { useEffect, useMemo, useState } from "react";

import { lookOf } from "@/components/builder/appearance";
import { AppShell } from "@/components/shell/AppShell";
import { CopyButton } from "@/components/ui/CopyButton";
import { Badge, Button, ErrorNote, Kicker } from "@/components/ui/primitives";
import { type ApiClient, ApiError, createApiClient } from "@/lib/api/client";
import type { GraphProject } from "@/lib/api/types";
import { DEMOS, type DemoScenario, demoGraph } from "@/lib/demos";

const ICONS: Record<DemoScenario["slug"], LucideIcon> = {
  fraud: ShieldAlert,
  cyber: Radar,
  "knowledge-graph": Library,
};

interface Loaded {
  graphs: GraphProject[];
  /** Graph id to the id of its demo's focus entity, where found. */
  focus: Record<string, string>;
}

export function DemoScenarios({ api: injected }: { api?: ApiClient }) {
  const api = useMemo(() => injected ?? createApiClient(), [injected]);
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [version, setVersion] = useState(0);

  useEffect(() => {
    let cancelled = false;
    api.listGraphs().then(
      async (graphs) => {
        const focus: Record<string, string> = {};
        await Promise.all(
          DEMOS.map(async (demo) => {
            const graph = demoGraph(demo, graphs);
            if (!graph) return;
            try {
              const found = await api.searchEntities(graph.id, {
                entity_type: demo.focus.entity_type,
                property_filters: { [demo.focus.property]: demo.focus.value },
                limit: 1,
              });
              if (found.data[0]) focus[graph.id] = found.data[0].id;
            } catch {
              // The link then opens the Explorer on its first entity.
            }
          }),
        );
        if (cancelled) return;
        setLoaded({ graphs, focus });
        setError(null);
      },
      (err: unknown) => {
        if (cancelled) return;
        setLoaded({ graphs: [], focus: {} });
        setError(
          err instanceof ApiError
            ? err.message
            : "MindGraph Core cannot be reached, so loaded demos cannot be shown.",
        );
      },
    );
    return () => {
      cancelled = true;
    };
  }, [api, version]);

  return (
    <AppShell>
      <main className="flex max-w-[1180px] flex-col gap-8 px-14 py-10">
        <header className="flex items-end gap-4">
          <div className="flex flex-1 flex-col gap-2">
            <Kicker className="text-accent">SAMPLE DATA · RUNS LOCALLY</Kicker>
            <h1 className="text-[30px] font-bold">Demo scenarios</h1>
            <p className="max-w-[680px] text-[14px] leading-[22px] text-text-2">
              Three small synthetic datasets ship with MindGraph Open. Load one with a single
              command on the machine running MindGraph: it goes through the same REST API as your
              own data, checks the result and prints a link to the graph. Running it again never
              duplicates or deletes anything.
            </p>
          </div>
          <Button
            icon={<RefreshCw size={13} />}
            onClick={() => setVersion((n) => n + 1)}
            disabled={loaded === null}
          >
            Check again
          </Button>
        </header>

        {error && <ErrorNote>{error}</ErrorNote>}

        <ul className="flex flex-col gap-4" aria-label="Demo scenarios">
          {DEMOS.map((demo) => (
            <DemoCard
              key={demo.slug}
              demo={demo}
              checking={loaded === null}
              graph={loaded ? demoGraph(demo, loaded.graphs) : undefined}
              focus={loaded?.focus ?? {}}
            />
          ))}
        </ul>

        <p className="flex items-start gap-2 text-[12.5px] leading-[19px] text-text-3">
          <Terminal size={13} className="mt-[3px] shrink-0" aria-hidden />
          <span>
            Run the commands from the MindGraph Open folder. Without make:{" "}
            <code className="font-mono text-[11.5px] text-text-2">
              docker compose --profile demo run --rm demo fraud
            </code>{" "}
            (or <code className="font-mono text-[11.5px] text-text-2">cyber</code>,{" "}
            <code className="font-mono text-[11.5px] text-text-2">knowledge-graph</code>). All
            people, companies, hosts and addresses in the demos are fictional.
          </span>
        </p>
      </main>
    </AppShell>
  );
}

function DemoCard({
  demo,
  graph,
  focus,
  checking,
}: {
  demo: DemoScenario;
  graph: GraphProject | undefined;
  focus: Record<string, string>;
  checking: boolean;
}) {
  const Icon = ICONS[demo.slug];
  const titleId = `demo-${demo.slug}-title`;
  return (
    <li
      id={demo.slug}
      aria-labelledby={titleId}
      className="grid scroll-mt-6 grid-cols-1 overflow-hidden rounded-[8px] border border-border lg:grid-cols-[1fr_340px]"
    >
      <section className="flex min-w-0 flex-col gap-4 px-6 py-5">
        <div className="flex items-center gap-3">
          <span
            className="flex h-[34px] w-[34px] shrink-0 items-center justify-center rounded-[6px] border border-border-strong"
            style={{ color: demo.color }}
          >
            <Icon size={16} aria-hidden />
          </span>
          <div className="flex min-w-0 flex-col">
            <h2 id={titleId} className="text-[17px] font-bold">
              {demo.title}
            </h2>
            <p className="text-[13px] text-text-2">{demo.description}</p>
          </div>
        </div>
        <div className="flex flex-col gap-2">
          <Kicker>Entities</Kicker>
          <ul className="flex flex-wrap gap-x-4 gap-y-1">
            {demo.entityTypes.map((type) => (
              <li
                key={type}
                className="flex items-center gap-1.5 font-mono text-[11px] text-text-2"
              >
                <span
                  aria-hidden
                  className="h-2 w-2 rounded-full"
                  style={{ background: lookOf(type).color }}
                />
                {type}
              </li>
            ))}
          </ul>
          <span className="font-mono text-[10.5px] text-text-3">
            {demo.relationshipTypes.join(" · ")}
          </span>
        </div>
        <div className="flex flex-col gap-1.5">
          <Kicker>What to look for</Kicker>
          <ul className="flex flex-col gap-1">
            {demo.highlights.map((line) => (
              <li key={line} className="flex gap-2 text-[13px] leading-[19px] text-text-2">
                <span aria-hidden className="text-text-3">
                  ›
                </span>
                {line}
              </li>
            ))}
          </ul>
        </div>
      </section>
      <section
        aria-label={`${demo.title} status`}
        className="flex flex-col gap-3 border-t border-border bg-panel px-5 py-5 lg:border-t-0 lg:border-l"
      >
        {checking ? (
          <span className="text-[13px] text-text-3" aria-busy>
            Checking which demos are loaded…
          </span>
        ) : graph ? (
          <Ready demo={demo} graph={graph} focus={focus[graph.id]} />
        ) : (
          <NotLoaded demo={demo} />
        )}
      </section>
    </li>
  );
}

function Ready({
  demo,
  graph,
  focus,
}: {
  demo: DemoScenario;
  graph: GraphProject;
  focus: string | undefined;
}) {
  const explore = `/graphs/${graph.id}/explore${focus ? `?entity=${focus}` : ""}`;
  return (
    <>
      <div className="flex items-center gap-2">
        <Badge tone="ok">Loaded</Badge>
        <span className="truncate text-[12.5px] text-text-2" title={graph.name}>
          {graph.name}
        </span>
      </div>
      <Link
        href={explore}
        className="inline-flex h-8 items-center justify-center gap-2 rounded-[4px] bg-accent px-3 text-[13px] font-semibold text-on-accent hover:bg-accent/90"
      >
        <Waypoints size={14} aria-hidden /> Open in Explorer
      </Link>
      <div className="flex flex-col gap-1 text-[12.5px]">
        {demo.resolution && (
          <Link
            href={`/graphs/${graph.id}/resolution`}
            className="inline-flex items-center gap-1.5 text-text-2 hover:text-text"
          >
            <CopyCheck size={13} aria-hidden /> Review possible duplicates
          </Link>
        )}
        <Link
          href={`/graphs/${graph.id}/integrations/mcp`}
          className="inline-flex items-center gap-1.5 text-text-2 hover:text-text"
        >
          <Plug size={13} aria-hidden /> Connect an AI agent over MCP
        </Link>
      </div>
    </>
  );
}

function NotLoaded({ demo }: { demo: DemoScenario }) {
  return (
    <>
      <div className="flex items-center gap-2">
        <Badge>Not loaded</Badge>
      </div>
      <p className="text-[12.5px] leading-[18px] text-text-2">
        Run this in the MindGraph Open folder, then press Check again:
      </p>
      <div className="flex items-center gap-2 rounded-[4px] border border-border bg-bg px-3 py-2">
        <span aria-hidden className="font-mono text-[12px] text-text-3">
          $
        </span>
        <code className="flex-1 font-mono text-[12px] font-medium text-text">{demo.command}</code>
        <CopyButton text={demo.command} label={`Copy ${demo.command}`} />
      </div>
      <span className="flex items-center gap-1.5 text-[11.5px] text-text-3">
        <ArrowRight size={11} aria-hidden /> It prints the Explorer link when the graph is ready.
      </span>
    </>
  );
}
