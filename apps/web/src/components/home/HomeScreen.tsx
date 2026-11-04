"use client";

/**
 * The home route: the welcome screen (mockup 01) until a graph exists, then
 * "Your Graphs" (mockup 02). Templates, the demo and MCP are shown as in the
 * design but disabled until their milestones.
 */

import {
  ArrowRight,
  BookOpen,
  Bug,
  Copy,
  Ellipsis,
  FolderGit2,
  Library,
  Play,
  Plug,
  Plus,
  ShieldAlert,
  Upload,
  Waypoints,
} from "lucide-react";
import Link from "next/link";
import { useEffect, useMemo, useState, useSyncExternalStore } from "react";

import { lookOf } from "@/components/builder/appearance";
import { HeroGraph } from "@/components/home/HeroGraph";
import { APP_VERSION, AppShell } from "@/components/shell/AppShell";
import { Button, ErrorNote, Kicker, Mark, comingIn } from "@/components/ui/primitives";
import { type ApiClient, ApiError, createApiClient } from "@/lib/api/client";
import type { GraphProject, Ontology } from "@/lib/api/types";

const REPOSITORY = "https://github.com/mindwalker-ai/MindGraph-Open";

function useHost(): string {
  return useSyncExternalStore(
    () => () => {},
    () => window.location.host,
    () => "localhost:3000",
  );
}

export function HomeScreen({ api: injected }: { api?: ApiClient }) {
  const api = useMemo(() => injected ?? createApiClient(), [injected]);
  const [graphs, setGraphs] = useState<GraphProject[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .listGraphs()
      .then(setGraphs, (err: unknown) =>
        setError(err instanceof ApiError ? err.message : "MindGraph Core cannot be reached."),
      );
  }, [api]);

  if (error) {
    return (
      <AppShell>
        <main className="px-14 py-10">
          <ErrorNote>{error}</ErrorNote>
        </main>
      </AppShell>
    );
  }
  if (graphs === null) return <div className="min-h-screen bg-bg" aria-busy />;
  if (graphs.length === 0) return <Welcome />;
  return <YourGraphs api={api} graphs={graphs} />;
}

// --- 01 Welcome --------------------------------------------------------------------

export function Welcome() {
  const host = useHost();
  return (
    <div className="relative min-h-screen overflow-hidden bg-bg">
      <div className="absolute top-[70px] left-[650px] hidden xl:block">
        <HeroGraph />
      </div>
      <div className="pointer-events-none absolute top-0 left-[560px] hidden h-full w-[360px] bg-gradient-to-r from-bg to-transparent xl:block" />
      <div className="pointer-events-none absolute bottom-0 left-0 h-[120px] w-full bg-gradient-to-t from-bg to-transparent" />

      <header className="relative flex h-16 items-center gap-6 px-12">
        <div className="flex items-center gap-[9px]">
          <Mark size={26} />
          <span className="text-[18px] font-bold">MindGraph</span>
          <span className="rounded-[3px] border border-border-strong px-[5px] py-px font-mono text-[9px] font-medium text-text-3">
            OPEN · {APP_VERSION.slice(0, 4)}
          </span>
        </div>
        <div className="flex-1" />
        <a
          href={`${REPOSITORY}#readme`}
          className="flex items-center gap-[7px] text-[13.5px] text-text-2 hover:text-text"
        >
          <BookOpen size={14} /> Documentation
        </a>
        <a
          href={REPOSITORY}
          className="flex items-center gap-[7px] text-[13.5px] text-text-2 hover:text-text"
        >
          <FolderGit2 size={14} /> GitHub
        </a>
        <span className="flex items-center gap-[7px] rounded-[4px] border border-border px-[10px] py-[5px]">
          <span className="h-1.5 w-1.5 rounded-full bg-ok" />
          <span className="font-mono text-[10px] font-medium text-text-2">{host}</span>
        </span>
      </header>

      <main className="relative flex min-h-[calc(100vh-64px)] flex-col justify-between px-24 pt-[146px] pb-12">
        <section className="flex w-full max-w-[720px] flex-col gap-7">
          <div className="flex items-center gap-2">
            <span className="h-[1.5px] w-4 bg-accent" />
            <span className="font-mono text-[11px] font-medium tracking-[1.2px] text-accent">
              OPEN-SOURCE GRAPH INTELLIGENCE
            </span>
          </div>
          <h1 className="text-[52px] leading-[54px] font-bold tracking-[-1px]">
            Turn messy data into an
            <br />
            explorable knowledge graph.
          </h1>
          <p className="max-w-[500px] text-[17px] leading-[26px] text-text-2">
            Open-source Graph Intelligence for humans and AI agents. Build knowledge graphs,
            investigate relationships, run GraphRAG, and connect AI agents through MCP.
          </p>
          <div className="flex items-center gap-[10px] pt-1">
            <Link
              href="/graphs/new"
              className="inline-flex h-[42px] items-center gap-2 rounded-[4px] bg-accent px-5 text-[15px] font-semibold text-on-accent hover:bg-accent/90"
            >
              <Plus size={16} /> Create Graph
            </Link>
            <span
              aria-disabled
              title={comingIn("M10")}
              className="inline-flex h-[42px] cursor-not-allowed items-center gap-2 rounded-[4px] border border-border-strong bg-raised px-[18px] text-[15px] font-semibold text-text-3"
            >
              <Play size={15} /> Explore Demo
            </span>
          </div>
          <div className="flex items-center">
            {["GRAPHRAG", "MCP", "ENTITY RESOLUTION", "INVESTIGATION"].map((cap, index) => (
              <span
                key={cap}
                className={
                  index === 0
                    ? "pr-[14px] font-mono text-[10.5px] font-medium tracking-[0.8px] text-text-2"
                    : "border-l border-border-strong px-[14px] font-mono text-[10.5px] font-medium tracking-[0.8px] text-text-2"
                }
              >
                {cap}
              </span>
            ))}
          </div>
        </section>

        <section className="mt-16 flex w-full max-w-[430px] flex-col gap-2">
          <Kicker>RUN IT YOURSELF</Kicker>
          <div className="flex flex-col gap-1 rounded-[6px] border border-border bg-panel px-[14px] py-3">
            <div className="flex items-center gap-2 font-mono text-[12px] font-medium">
              <span className="text-text-3">$</span>
              <span className="flex-1">docker compose up</span>
              <button
                type="button"
                aria-label="Copy command"
                className="text-text-3 hover:text-text"
                onClick={() => void navigator.clipboard?.writeText("docker compose up")}
              >
                <Copy size={13} />
              </button>
            </div>
            <div className="flex gap-2 font-mono text-[12px] font-medium">
              <span className="text-ok">✓</span>
              <span className="text-text-2">MindGraph is running at http://{host}</span>
            </div>
          </div>
        </section>
      </main>

      <div className="absolute right-[100px] bottom-[52px] hidden items-center gap-2 xl:flex">
        <span className="h-[7px] w-[7px] rounded-full bg-risk" />
        <span className="font-mono text-[10px] font-medium text-text-2">
          Fraud demo · 2 customers linked through DEVICE-921
        </span>
      </div>
    </div>
  );
}

// --- 02 Your Graphs ----------------------------------------------------------------

const DOTS = [
  "var(--risk)",
  "var(--t-customer)",
  "var(--t-ip)",
  "var(--t-merchant)",
  "var(--t-device)",
];

function greeting(): string {
  const hour = new Date().getHours();
  if (hour < 12) return "Good morning.";
  if (hour < 18) return "Good afternoon.";
  return "Good evening.";
}

function updatedAgo(iso: string): string {
  const minutes = Math.max(0, Math.round((Date.now() - new Date(iso).getTime()) / 60_000));
  if (minutes < 1) return "Updated just now";
  if (minutes < 60) return `Updated ${minutes} minute${minutes === 1 ? "" : "s"} ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `Updated ${hours} hour${hours === 1 ? "" : "s"} ago`;
  const days = Math.round(hours / 24);
  if (days < 7) return `Updated ${days} day${days === 1 ? "" : "s"} ago`;
  return days < 14 ? "Updated last week" : `Updated ${new Date(iso).toLocaleDateString()}`;
}

/** A small drawing of a graph's ontology: one dot per type, a line per relationship. */
function OntologyThumbnail({ ontology }: { ontology: Ontology | null | undefined }) {
  const types = ontology?.entity_types ?? [];
  const width = 368;
  const height = 150;
  const position = (index: number) => {
    const angle = (index / Math.max(types.length, 1)) * Math.PI * 2 - Math.PI / 2;
    const radius = types.length > 1 ? 48 : 0;
    return {
      x: width / 2 + Math.cos(angle) * radius * 2.2,
      y: height / 2 + Math.sin(angle) * radius,
    };
  };
  const at = new Map(types.map((t, i) => [t.name, position(i)]));
  return (
    <svg
      viewBox={`0 0 ${width} ${height}`}
      className="h-[150px] w-full border-b border-border bg-panel"
      role="img"
      aria-label={types.length ? `Ontology with ${types.length} entity types` : "Empty ontology"}
    >
      {types.length === 0 && (
        <text
          x={width / 2}
          y={height / 2}
          textAnchor="middle"
          className="fill-text-3 font-mono text-[10px]"
        >
          {ontology === undefined ? "" : "No ontology yet"}
        </text>
      )}
      {ontology?.relationship_types.map((r) => {
        const a = at.get(r.source_entity_type);
        const b = at.get(r.target_entity_type);
        return a && b ? (
          <line
            key={r.name}
            x1={a.x}
            y1={a.y}
            x2={b.x}
            y2={b.y}
            stroke="var(--border-strong)"
            strokeWidth={1}
          />
        ) : null;
      })}
      {types.map((t) => {
        const p = at.get(t.name) as { x: number; y: number };
        return (
          <g key={t.name}>
            <circle cx={p.x} cy={p.y} r={5} fill={lookOf(t.name).color} />
            <text
              x={p.x}
              y={p.y + 16}
              textAnchor="middle"
              className="fill-text-3 font-mono text-[9px]"
            >
              {t.name}
            </text>
          </g>
        );
      })}
    </svg>
  );
}

function GraphCard({
  graph,
  ontology,
  index,
}: {
  graph: GraphProject;
  ontology: Ontology | null | undefined;
  index: number;
}) {
  // An active graph opens in the Explorer; a draft one still needs data.
  const active = graph.status === "active";
  const explore = `/graphs/${graph.id}/explore`;
  const upload = `/graphs/${graph.id}/upload`;
  return (
    <div className="relative flex min-w-0 flex-col border-border bg-bg hover:bg-raised [&:nth-child(n+2)]:border-t md:[&:nth-child(-n+3)]:border-t-0 md:[&:not(:nth-child(3n+1))]:border-l">
      <OntologyThumbnail ontology={ontology} />
      <div className="flex flex-col gap-[10px] px-4 pt-[14px] pb-4">
        <div className="flex items-center gap-2">
          <span
            className="h-2 w-2 shrink-0 rounded-[2px]"
            style={{ background: DOTS[index % DOTS.length] }}
          />
          <Link
            href={active ? explore : upload}
            className="flex-1 truncate text-[16px] font-bold after:absolute after:inset-0"
          >
            {graph.name}
          </Link>
          <span title={comingIn()} className="text-text-3">
            <Ellipsis size={14} />
          </span>
        </div>
        <div className="flex gap-6" title="Graph-wide counts are not computed yet">
          {["entities", "relationships"].map((label) => (
            <div key={label} className="flex flex-col">
              <span className="text-[18px] font-semibold text-text-3">—</span>
              <span className="font-mono text-[10px] font-medium text-text-3">{label}</span>
            </div>
          ))}
        </div>
        <div className="flex items-center gap-2">
          <span className="flex-1 truncate font-mono text-[10px] font-medium text-text-3">
            hugegraph · {graph.status} · ontology v{ontology?.version ?? "–"}
          </span>
          <span className="text-[12px] text-text-3">{updatedAgo(graph.updated_at)}</span>
        </div>
        <div className="relative z-10 flex items-center gap-4 text-[12.5px]">
          {active && (
            <Link
              href={explore}
              className="inline-flex items-center gap-1.5 font-semibold text-accent hover:text-accent/80"
            >
              <Waypoints size={13} /> Explore Graph
            </Link>
          )}
          <Link
            href={upload}
            className="inline-flex items-center gap-1.5 text-text-2 hover:text-text"
          >
            <Upload size={13} /> Add data
          </Link>
        </div>
      </div>
    </div>
  );
}

const TEMPLATES = [
  {
    name: "Fraud Investigation",
    description: "Trace shared devices, IPs and merchants across accounts.",
    entities: "Customer · Account · Transaction · Merchant · Device · IP",
    icon: ShieldAlert,
    color: "var(--risk)",
  },
  {
    name: "Knowledge Graph",
    description: "Map people, companies, topics and documents.",
    entities: "Person · Company · Topic · Document · Project",
    icon: Library,
    color: "var(--t-customer)",
  },
  {
    name: "Cyber Investigation",
    description: "Follow users, endpoints and domains to malware.",
    entities: "User · Endpoint · IP · Domain · Process · Malware",
    icon: Bug,
    color: "var(--t-ip)",
  },
];

function YourGraphs({ api, graphs }: { api: ApiClient; graphs: GraphProject[] }) {
  const host = useHost();
  const [ontologies, setOntologies] = useState<Record<string, Ontology | null>>({});
  // The browser's local time; nothing during server rendering.
  const hello = useSyncExternalStore(
    () => () => {},
    greeting,
    () => "",
  );

  useEffect(() => {
    let cancelled = false;
    for (const graph of graphs.slice(0, 12)) {
      api.getOntology(graph.id).then(
        (ontology) => !cancelled && setOntologies((all) => ({ ...all, [graph.id]: ontology })),
        () => !cancelled && setOntologies((all) => ({ ...all, [graph.id]: null })),
      );
    }
    return () => {
      cancelled = true;
    };
  }, [api, graphs]);

  return (
    <AppShell>
      <main className="flex flex-col gap-9 px-14 py-10">
        <header className="flex items-end gap-[10px]">
          <div className="flex flex-1 flex-col gap-1">
            <span className="min-h-[21px] text-[14px] text-text-3">{hello}</span>
            <h1 className="text-[30px] font-bold">Your Graphs</h1>
          </div>
          <Link
            href="/graphs/new"
            className="inline-flex h-8 items-center gap-2 rounded-[4px] border border-border-strong bg-raised px-3 text-[13px] font-semibold hover:bg-hover"
          >
            <Upload size={14} /> Import Data
          </Link>
          <Link
            href="/graphs/new"
            className="inline-flex h-8 items-center gap-2 rounded-[4px] bg-accent px-[14px] text-[13px] font-semibold text-on-accent hover:bg-accent/90"
          >
            <Plus size={14} /> New Graph
          </Link>
        </header>

        <section
          aria-label="Graphs"
          className="grid grid-cols-1 overflow-hidden rounded-[8px] border border-border md:grid-cols-3"
        >
          {graphs.map((graph, index) => (
            <GraphCard
              key={graph.id}
              graph={graph}
              index={index}
              ontology={index < 12 ? ontologies[graph.id] : null}
            />
          ))}
          {graphs.length % 3 !== 0 && (
            // Completes the last row of the grid.
            <Link
              href="/graphs/new"
              className="flex min-h-[282px] flex-col items-center justify-center gap-2 border-t border-border text-text-3 hover:bg-raised hover:text-text-2 md:border-t-0 md:border-l"
            >
              <span className="flex h-9 w-9 items-center justify-center rounded-[6px] border border-dashed border-border-strong">
                <Plus size={16} />
              </span>
              <span className="text-[13px] font-semibold">New Graph</span>
            </Link>
          )}
        </section>

        <section className="flex flex-col gap-3">
          <div className="flex items-center">
            <h2 className="flex-1 text-[17px] font-bold">Explore Templates</h2>
            <Kicker>SAMPLE DATA INCLUDED · RUNS LOCALLY</Kicker>
          </div>
          <ul className="flex flex-col rounded-[8px] border border-border">
            {TEMPLATES.map((template, index) => {
              const Icon = template.icon;
              return (
                <li
                  key={template.name}
                  className={`flex h-16 items-center gap-[14px] px-4 ${index ? "border-t border-border" : ""}`}
                >
                  <span
                    className="flex h-[34px] w-[34px] items-center justify-center rounded-[6px] border border-border-strong"
                    style={{ color: template.color }}
                  >
                    <Icon size={16} />
                  </span>
                  <div className="flex w-[340px] flex-col gap-px">
                    <span className="text-[14px] font-bold">{template.name}</span>
                    <span className="text-[12.5px] text-text-3">{template.description}</span>
                  </div>
                  <span className="hidden flex-1 font-mono text-[10.5px] font-medium text-text-2 lg:block">
                    {template.entities}
                  </span>
                  <Button
                    size="sm"
                    disabled
                    title={comingIn("M10")}
                    className="ml-auto h-7 border-border-strong bg-transparent px-[10px] text-[12.5px]"
                  >
                    Use template <ArrowRight size={12} />
                  </Button>
                </li>
              );
            })}
          </ul>
        </section>

        <section className="flex items-center gap-4 rounded-[8px] border border-border bg-panel px-5 py-[18px]">
          <span className="flex h-[38px] w-[38px] shrink-0 items-center justify-center rounded-[6px] bg-accent-soft text-accent">
            <Plug size={17} />
          </span>
          <div className="flex flex-1 flex-col gap-0.5">
            <span className="text-[15px] font-bold">Give your AI agents the same graph</span>
            <span className="text-[13px] text-text-3">
              Expose search, neighbors and path-finding to Claude, Codex and Cursor over MCP.
            </span>
          </div>
          <span
            title="Not running yet"
            className="hidden h-8 items-center gap-2 rounded-[4px] border border-border bg-bg px-[10px] md:flex"
          >
            <span className="h-1.5 w-1.5 rounded-full bg-text-3" />
            <span className="font-mono text-[11px] font-medium text-text-2">http://{host}/mcp</span>
          </span>
          <Button disabled title={comingIn("M8")} icon={<ArrowRight size={14} />}>
            Set up MCP
          </Button>
        </section>
      </main>
    </AppShell>
  );
}
