"use client";

/**
 * The Graph Explorer (mockups 06 and 07): a read-only investigation
 * workspace over one graph.
 *
 * Discovery first: it loads the graph, its active ontology and a bounded
 * page of entities, then the neighbourhood of one seed entity. Everything
 * after that is incremental: expanding, connecting and path finding merge
 * bounded subgraphs from MindGraph Core into the loaded Graphology graph,
 * which Sigma.js draws. The browser never holds more than the view budget
 * and never asks for the whole graph. Hiding and resetting are client-side.
 */

import {
  ChevronRight,
  Copy,
  EyeOff,
  Expand,
  Focus,
  Link2,
  LoaderCircle,
  type LucideIcon,
  PanelLeftClose,
  PanelLeftOpen,
  Route,
  Search,
  Shrink,
  TerminalSquare,
  X,
} from "lucide-react";
import dynamic from "next/dynamic";
import Link from "next/link";
import {
  type ComponentType,
  type FormEvent,
  type ReactNode,
  useEffect,
  useEffectEvent,
  useReducer,
  useRef,
  useState,
} from "react";

import type { CanvasHandle, CanvasProps } from "@/components/explorer/canvas";
import {
  DetailsHint,
  EntityDetails,
  RelationshipDetails,
} from "@/components/explorer/DetailsPanel";
import { ExplorerSidebar } from "@/components/explorer/ExplorerSidebar";
import { GraphToolbar } from "@/components/explorer/GraphToolbar";
import { type Endpoint, PathFinder, type PathForm } from "@/components/explorer/PathFinder";
import { Rail } from "@/components/shell/AppShell";
import { Button, Kicker, inputClass } from "@/components/ui/primitives";
import { type ApiClient, ApiError, createApiClient } from "@/lib/api/client";
import {
  EXPLORER_LIMITS,
  type Entity,
  type GraphProject,
  type NeighborOptions,
  type Ontology,
} from "@/lib/api/types";
import { type LayoutMode, layoutPositions, settleNewNodes } from "@/lib/graph/layout";
import {
  type GraphBudget,
  type MergeResult,
  VIEW_BUDGET,
  mergeSubgraph,
  removeEntities,
} from "@/lib/graph/merge";
import {
  DEFAULT_SEARCH_LIMIT,
  type SearchForm,
  type SearchState,
  buildSearchQuery,
} from "@/lib/graph/search";
import {
  initialInvestigation,
  investigationReducer,
  collapsibleNeighbors,
  isEntityVisible,
  relationshipGroups,
} from "@/lib/graph/state";
import {
  MalformedGraphError,
  createExplorerGraph,
  entityLabel,
  parseEntity,
  parsePath,
  shortId,
} from "@/lib/graph/transform";
import { cn } from "@/lib/utils";

function CanvasMessage({ children }: { children: ReactNode }) {
  return (
    <div className="absolute inset-0 flex items-center justify-center text-[13px] text-text-3">
      {children}
    </div>
  );
}

/** Sigma needs WebGL and `window`: loaded in the browser only. */
const SigmaCanvas = dynamic(() => import("@/components/explorer/GraphCanvas"), {
  ssr: false,
  loading: () => <CanvasMessage>Loading the canvas…</CanvasMessage>,
});

type Phase =
  | { status: "loading" }
  | { status: "failed"; title: string; message: string; retry: boolean }
  | { status: "ready"; project: GraphProject; ontology: Ontology; empty: boolean };

interface Notice {
  tone: "info" | "warn" | "error";
  text: string;
  /** Offer ways to make room in the view. */
  budget?: boolean;
}

function describe(error: unknown): string {
  if (error instanceof ApiError) {
    if (error.status === 503) {
      return `The graph store is unavailable right now; try again shortly (${error.code}).`;
    }
    return `${error.message} (${error.code})`;
  }
  if (error instanceof MalformedGraphError) {
    return `MindGraph Core sent an unexpected graph payload: ${error.message}`;
  }
  return error instanceof Error ? error.message : String(error);
}

function failure(error: unknown): Phase {
  if (error instanceof ApiError && error.code === "GRAPH_NOT_FOUND") {
    return {
      status: "failed",
      title: "Graph not found",
      message: "This graph does not exist in your workspace.",
      retry: false,
    };
  }
  if (error instanceof ApiError && error.code === "INVALID_GRAPH_ID") {
    return {
      status: "failed",
      title: "Invalid graph ID",
      message: "The address does not contain a valid graph ID.",
      retry: false,
    };
  }
  return {
    status: "failed",
    title:
      error instanceof ApiError && error.status === 503
        ? "Graph store unavailable"
        : "The explorer could not load",
    message: describe(error),
    retry: true,
  };
}

function budgetNotice(result: MergeResult, budget: GraphBudget): Notice | null {
  if (!result.skippedEntities && !result.skippedRelationships) return null;
  return {
    tone: "warn",
    budget: true,
    text:
      `The view holds at most ${budget.maxEntities} entities and ${budget.maxRelationships} ` +
      `relationships, so ${result.skippedEntities} entities and ${result.skippedRelationships} ` +
      "relationships were not added. Hide and unload entities, or reset the view, to load more.",
  };
}

const NOTICE_TONE = {
  info: "border-border-strong text-text-2",
  warn: "border-warn/50 text-warn",
  error: "border-danger/50 text-danger",
};

/** Measures an operation for the operation chip (duration only, no data). */
function stopwatch(): () => number {
  const started = performance.now();
  return () => Math.round(performance.now() - started);
}

type MenuAction = "expand" | "path" | "connections" | "focus" | "collapse" | "hide" | "copy";

/** The entity context menu (mockup 07); every entry acts on the view only. */
const MENU: [MenuAction, string, string, LucideIcon][] = [
  ["expand", "Expand neighbours", "E", Expand],
  ["path", "Find path from here", "P", Route],
  ["connections", "Connections in view", "", Link2],
  ["focus", "Focus", "F", Focus],
  ["collapse", "Collapse neighbours", "C", Shrink],
  ["hide", "Hide", "H", EyeOff],
  ["copy", "Copy ID", "", Copy],
];

export interface GraphExplorerProps {
  graphId: string;
  /** The entity to open first (`?entity=`); otherwise the first one found. */
  initialEntityId?: string;
  /** Injectable for tests; defaults to the same-origin API. */
  api?: ApiClient;
  /** The canvas; tests pass a stand-in because jsdom has no WebGL. */
  Canvas?: ComponentType<CanvasProps>;
  budget?: GraphBudget;
}

export function GraphExplorer({
  graphId,
  initialEntityId,
  api: injected,
  Canvas = SigmaCanvas,
  budget = VIEW_BUDGET,
}: GraphExplorerProps) {
  const [api] = useState(() => injected ?? createApiClient());
  // The loaded subgraph; mutated in place by merges, so React re-renders on `revision`.
  const [graph] = useState(createExplorerGraph);
  const [, setRevision] = useState(0);
  const [state, dispatch] = useReducer(investigationReducer, initialInvestigation);
  const [phase, setPhase] = useState<Phase>({ status: "loading" });
  const [attempt, setAttempt] = useState(0);
  const [search, setSearch] = useState<SearchState>({ status: "idle" });
  const [expanding, setExpanding] = useState<ReadonlySet<string>>(new Set());
  const [connecting, setConnecting] = useState(false);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [lastOperation, setLastOperation] = useState<string | null>(null);
  const [panel, setPanel] = useState<"details" | "path">("details");
  const [pathForm, setPathForm] = useState<PathForm>({
    source: null,
    target: null,
    direction: "both",
    maxDepth: EXPLORER_LIMITS.pathDefaultDepth,
  });
  const [picking, setPicking] = useState<Endpoint | null>(null);
  const [pathRun, setPathRun] = useState<{ running: boolean; error: string | null }>({
    running: false,
    error: null,
  });
  const [menu, setMenu] = useState<{ entityId: string; x: number; y: number } | null>(null);
  const [options, setOptions] = useState<NeighborOptions>({
    direction: "both",
    limit: EXPLORER_LIMITS.neighborDefault,
  });
  const [ready, setReady] = useState(false);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [lookup, setLookup] = useState({ value: "", error: null as string | null, busy: false });
  const controllerRef = useRef<CanvasHandle | null>(null);
  const inFlight = useRef(new Set<string>());

  const ontology = phase.status === "ready" ? phase.ontology : null;
  const changed = () => setRevision((revision) => revision + 1);
  const labelOf = (entityId: string) =>
    graph.hasNode(entityId) ? graph.getNodeAttribute(entityId, "label") : shortId(entityId);
  const track = (operation: string, elapsed: () => number, outcome: string) =>
    setLastOperation(`${operation} → ${outcome} · ${elapsed()} ms`);

  // --- operations (each one leaves the loaded graph intact when it fails) ----------------

  async function expand(
    entityId: string,
    how: { select?: boolean; fit?: boolean; neighbors?: NeighborOptions } = {},
  ) {
    if (inFlight.current.has(entityId)) return;
    inFlight.current.add(entityId);
    setExpanding(new Set(inFlight.current));
    const neighbors = how.neighbors ?? options;
    const limit = neighbors.limit ?? EXPLORER_LIMITS.neighborDefault;
    const elapsed = stopwatch();
    try {
      const subgraph = await api.getNeighbors(graphId, entityId, neighbors);
      const result = mergeSubgraph(graph, subgraph, { anchor: entityId, graphId, budget });
      settleNewNodes(graph, result.addedEntityIds);
      changed();
      if (how.select && graph.hasNode(entityId)) {
        dispatch({ type: "revealEntity", id: entityId });
        dispatch({ type: "selectEntity", id: entityId });
      }
      const filter = neighbors.relationship_type ? `, ${neighbors.relationship_type}` : "";
      track(
        `neighbors(${labelOf(entityId)}, ${neighbors.direction ?? "both"}${filter}, limit ${limit})`,
        elapsed,
        `+${result.addedEntityIds.length} entities, +${result.addedRelationshipIds.length} relationships`,
      );
      setNotice(
        budgetNotice(result, budget) ??
          (subgraph.relationships.length >= limit
            ? {
                tone: "info",
                text: `Loaded the first ${limit} relationships of ${labelOf(entityId)}; it may have more. Raise the limit in the expand options to load others.`,
              }
            : null),
      );
      if (how.fit) controllerRef.current?.fit();
      else if (how.select) controllerRef.current?.center(entityId);
    } catch (error) {
      setNotice({
        tone: "error",
        text: `Could not expand ${labelOf(entityId)}: ${describe(error)}`,
      });
    } finally {
      inFlight.current.delete(entityId);
      setExpanding(new Set(inFlight.current));
    }
  }

  function openEntity(entityId: string) {
    setMenu(null);
    setPanel("details");
    if (!graph.hasNode(entityId)) {
      void expand(entityId, { select: true });
      return;
    }
    dispatch({ type: "revealEntity", id: entityId });
    dispatch({ type: "selectEntity", id: entityId });
    controllerRef.current?.center(entityId);
  }

  async function connections(entityId: string) {
    setMenu(null);
    setConnecting(true);
    const elapsed = stopwatch();
    const ids = [entityId, ...graph.nodes().filter((id) => id !== entityId)].slice(
      0,
      EXPLORER_LIMITS.subgraphMax,
    );
    try {
      const subgraph = await api.getSubgraph(graphId, ids);
      const result = mergeSubgraph(graph, subgraph, { graphId, budget });
      changed();
      track(
        `subgraph(${ids.length} entities)`,
        elapsed,
        `+${result.addedRelationshipIds.length} relationships`,
      );
      setNotice(
        budgetNotice(result, budget) ??
          (result.addedRelationshipIds.length === 0
            ? { tone: "info", text: "No further relationships among the entities in view." }
            : null),
      );
    } catch (error) {
      setNotice({ tone: "error", text: `Could not load connections: ${describe(error)}` });
    } finally {
      setConnecting(false);
    }
  }

  async function runSearch(form: SearchForm) {
    const built = buildSearchQuery(form, ontology);
    if (!built.ok) {
      setSearch({ status: "error", query: null, message: built.error });
      return;
    }
    setSearch({ status: "loading", query: built.query });
    try {
      const result = await api.searchEntities(graphId, built.query);
      setSearch({
        status: "done",
        query: built.query,
        results: result.data.map((entity) => parseEntity(entity, graphId)),
        limit: result.limit,
      });
    } catch (error) {
      setSearch({ status: "error", query: built.query, message: describe(error) });
    }
  }

  function openPathFinder(sourceId: string | null = state.selectedEntityId) {
    setMenu(null);
    const source =
      sourceId && graph.hasNode(sourceId) ? graph.getNodeAttribute(sourceId, "entity") : null;
    const next = {
      ...pathForm,
      source: source ?? pathForm.source,
      target: source && pathForm.target?.id === source.id ? null : pathForm.target,
    };
    setPathForm(next);
    setPicking(!next.source ? "source" : !next.target ? "target" : null);
    setPanel("path");
  }

  function pickEndpoint(endpoint: Endpoint, entity: Entity) {
    setPathForm((form) => ({ ...form, [endpoint]: entity }));
    setPicking(endpoint === "source" && !pathForm.target ? "target" : null);
  }

  async function lookupEndpoint(endpoint: Endpoint, entityId: string): Promise<string | null> {
    try {
      pickEndpoint(endpoint, parseEntity(await api.getEntity(graphId, entityId), graphId));
      return null;
    } catch (error) {
      return error instanceof ApiError && error.code === "ENTITY_NOT_FOUND"
        ? "No entity with this ID in this graph."
        : describe(error);
    }
  }

  async function runPath(patch: Partial<PathForm> = {}) {
    const form = { ...pathForm, ...patch };
    setPathForm(form);
    const { source, target, direction, maxDepth } = form;
    if (!source || !target) return;
    setPicking(null);
    setPathRun({ running: true, error: null });
    const elapsed = stopwatch();
    try {
      const path = parsePath(
        await api.findPath(graphId, source.id, target.id, { direction, max_depth: maxDepth }),
        graphId,
      );
      if (path.found) {
        const result = mergeSubgraph(
          graph,
          { entities: path.entities, relationships: path.relationships },
          { anchor: source.id, graphId, budget },
        );
        settleNewNodes(graph, result.addedEntityIds);
        for (const entity of path.entities) dispatch({ type: "revealEntity", id: entity.id });
        changed();
        setNotice(budgetNotice(result, budget));
      }
      dispatch({
        type: "showPath",
        path: { sourceId: source.id, targetId: target.id, direction, maxDepth, path },
      });
      setPathRun({ running: false, error: null });
      track(
        `path(${entityLabel(source)} → ${entityLabel(target)}, ${direction}, max ${maxDepth})`,
        elapsed,
        path.found ? `${path.hops} ${path.hops === 1 ? "hop" : "hops"}` : "no path",
      );
      if (path.found) controllerRef.current?.focus(path.entities.map((entity) => entity.id));
    } catch (error) {
      setPathRun({ running: false, error: `Could not find a path: ${describe(error)}` });
    }
  }

  function hideEntity(entityId: string) {
    setMenu(null);
    dispatch({ type: "hideEntity", id: entityId });
  }

  function collapse(entityId: string) {
    setMenu(null);
    const ids = collapsibleNeighbors(graph, state, entityId);
    dispatch({ type: "hideEntities", ids });
    setNotice({
      tone: "info",
      text:
        ids.length === 0
          ? `Nothing to collapse: the neighbours of ${labelOf(entityId)} in view are shared with other entities.`
          : `Collapsed ${ids.length} neighbour${ids.length === 1 ? "" : "s"} of ${labelOf(entityId)}; shared neighbours stay. "Show all" brings them back.`,
    });
  }

  function runMenu(action: MenuAction, entityId: string) {
    setMenu(null);
    switch (action) {
      case "expand":
        return void expand(entityId, { select: true });
      case "path":
        return openPathFinder(entityId);
      case "connections":
        return void connections(entityId);
      case "focus":
        return controllerRef.current?.center(entityId);
      case "collapse":
        return collapse(entityId);
      case "hide":
        return hideEntity(entityId);
      case "copy":
        return void navigator.clipboard?.writeText(entityId);
    }
  }

  function unloadHidden() {
    const relationshipIds = removeEntities(graph, state.hiddenEntityIds);
    for (const edge of state.hiddenRelationshipIds) {
      if (graph.hasEdge(edge)) {
        graph.dropEdge(edge);
        relationshipIds.push(edge);
      }
    }
    dispatch({ type: "forget", entityIds: [...state.hiddenEntityIds], relationshipIds });
    setNotice(null);
    changed();
  }

  function resetView() {
    graph.clear();
    dispatch({ type: "reset" });
    setPathForm((form) => ({ ...form, source: null, target: null }));
    setPicking(null);
    setMenu(null);
    setNotice(null);
    setLastOperation(null);
    changed();
  }

  function resetHighlight() {
    setMenu(null);
    setPicking(null);
    dispatch({ type: "resetHighlight" });
    dispatch({ type: "clearSelection" });
  }

  function layout(mode: LayoutMode) {
    dispatch({ type: "setLayout", mode });
    const positions = layoutPositions(graph, mode);
    if (controllerRef.current) {
      controllerRef.current.applyLayout(positions);
    } else {
      for (const [id, position] of Object.entries(positions))
        graph.mergeNodeAttributes(id, position);
    }
  }

  async function lookupEntity(event: FormEvent) {
    event.preventDefault();
    const id = lookup.value.trim();
    if (!id) return;
    setLookup({ value: lookup.value, error: null, busy: true });
    try {
      parseEntity(await api.getEntity(graphId, id), graphId);
      setLookup({ value: "", error: null, busy: false });
      openEntity(id);
    } catch (error) {
      const message =
        error instanceof ApiError && error.code === "ENTITY_NOT_FOUND"
          ? "No entity with this ID in this graph"
          : error instanceof ApiError && error.code === "INVALID_ENTITY_ID"
            ? "Not a valid entity ID"
            : describe(error);
      setLookup({ value: id, error: message, busy: false });
    }
  }

  // --- canvas events -----------------------------------------------------------------------

  const canvasHandlers = {
    onSelectEntity(entityId: string) {
      setMenu(null);
      if (picking) {
        pickEndpoint(picking, graph.getNodeAttribute(entityId, "entity"));
        return;
      }
      dispatch({ type: "selectEntity", id: entityId });
      setPanel("details");
    },
    onSelectRelationship(relationshipId: string) {
      setMenu(null);
      dispatch({ type: "selectRelationship", id: relationshipId });
      setPanel("details");
    },
    onClearSelection() {
      setMenu(null);
      if (!picking) dispatch({ type: "clearSelection" });
    },
    onExpand(entityId: string) {
      void expand(entityId, { select: true });
    },
    onContextMenu(entityId: string, position: { x: number; y: number }) {
      dispatch({ type: "selectEntity", id: entityId });
      setPanel("details");
      setMenu({ entityId, ...position });
    },
    onReady() {
      setReady(true);
      if (graph.order > 0) controllerRef.current?.fit();
    },
  };

  // --- effects -------------------------------------------------------------------------------

  const openSeed = useEffectEvent((entityId: string) => {
    void expand(entityId, { select: true, fit: true });
  });

  useEffect(() => {
    let cancelled = false;
    Promise.all([api.getGraph(graphId), api.getOntology(graphId)])
      .then(async ([project, active]) => {
        const first = await api.searchEntities(graphId, { limit: DEFAULT_SEARCH_LIMIT });
        if (cancelled) return;
        const results = first.data.map((entity) => parseEntity(entity, graphId));
        setSearch({
          status: "done",
          query: { limit: DEFAULT_SEARCH_LIMIT },
          results,
          limit: first.limit,
        });
        setPhase({ status: "ready", project, ontology: active, empty: results.length === 0 });
        const seed = initialEntityId ?? results[0]?.id;
        if (seed) openSeed(seed);
      })
      .catch((error: unknown) => {
        if (!cancelled) setPhase(failure(error));
      });
    return () => {
      cancelled = true;
    };
  }, [api, graphId, initialEntityId, attempt]);

  // The inspected entity is kept in the address, so a refresh reopens it.
  useEffect(() => {
    const id = state.selectedEntityId;
    if (!id) return;
    const url = new URL(window.location.href);
    if (url.searchParams.get("entity") === id) return;
    url.searchParams.set("entity", id);
    window.history.replaceState(null, "", url);
  }, [state.selectedEntityId]);

  const onKey = useEffectEvent((event: KeyboardEvent) => {
    const target = event.target as HTMLElement | null;
    if (target?.closest("input, select, textarea, [contenteditable='true']")) return;
    if (event.metaKey || event.ctrlKey || event.altKey || phase.status !== "ready") return;
    const selected = state.selectedEntityId;
    switch (event.key.toLowerCase()) {
      case "escape":
        if (menu) setMenu(null);
        else if (picking) setPicking(null);
        else resetHighlight();
        return;
      case "e":
        if (selected) void expand(selected);
        return;
      case "p":
        openPathFinder();
        return;
      case "h":
        if (selected) hideEntity(selected);
        return;
      case "c":
        if (selected) collapse(selected);
        return;
      case "f":
        if (selected) controllerRef.current?.center(selected);
        return;
    }
  });

  useEffect(() => {
    const listener = (event: KeyboardEvent) => onKey(event);
    window.addEventListener("keydown", listener);
    return () => window.removeEventListener("keydown", listener);
  }, []);

  // --- render --------------------------------------------------------------------------------

  if (phase.status === "loading") {
    return (
      <Frame graphId={graphId}>
        <CanvasMessage>
          <LoaderCircle size={16} className="mr-2 animate-spin" /> Loading graph…
        </CanvasMessage>
      </Frame>
    );
  }

  if (phase.status === "failed") {
    return (
      <Frame graphId={graphId}>
        <div className="flex flex-1 items-center justify-center p-6">
          <div
            role="alert"
            className="flex max-w-md flex-col gap-3 rounded-[6px] border border-border bg-panel p-6"
          >
            <h1 className="text-[18px] font-bold">{phase.title}</h1>
            <p className="text-[13px] text-text-2">{phase.message}</p>
            <div className="flex gap-2">
              {phase.retry && (
                <Button
                  variant="primary"
                  onClick={() => {
                    setPhase({ status: "loading" });
                    setAttempt((n) => n + 1);
                  }}
                >
                  Try again
                </Button>
              )}
              <Link
                href="/"
                className="inline-flex h-8 items-center rounded-[4px] border border-border-strong bg-raised px-3 text-[13px] font-semibold hover:bg-hover"
              >
                All graphs
              </Link>
            </div>
          </div>
        </div>
      </Frame>
    );
  }

  const { project, empty } = phase;
  const selectedNode =
    state.selectedEntityId && graph.hasNode(state.selectedEntityId)
      ? graph.getNodeAttributes(state.selectedEntityId)
      : null;
  const selectedEdge =
    state.selectedRelationshipId && graph.hasEdge(state.selectedRelationshipId)
      ? graph.getEdgeAttributes(state.selectedRelationshipId)
      : null;
  const loadedByType = new Map<string, number>();
  const visible: Entity[] = [];
  graph.forEachNode((id, attributes) => {
    loadedByType.set(attributes.entityType, (loadedByType.get(attributes.entityType) ?? 0) + 1);
    if (isEntityVisible(state, id, attributes)) visible.push(attributes.entity);
  });
  const busy = expanding.size > 0 || pathRun.running || connecting;
  const canReset =
    Boolean(state.selectedEntityId || state.selectedRelationshipId || state.activePath) ||
    state.highlightedEntityIds.size > 0;
  const menuNode =
    menu && graph.hasNode(menu.entityId) ? graph.getNodeAttributes(menu.entityId) : null;

  return (
    <Frame graphId={graphId}>
      {/* Top bar: where we are, entity lookup by ID, the way back to the builder. */}
      <header className="flex h-12 shrink-0 items-center gap-3 border-b border-border bg-panel px-3">
        <button
          type="button"
          onClick={() => setSidebarOpen((open) => !open)}
          aria-label={sidebarOpen ? "Hide the side panel" : "Show the side panel"}
          className="rounded-[4px] p-1 text-text-3 hover:bg-raised hover:text-text"
        >
          {sidebarOpen ? <PanelLeftClose size={16} /> : <PanelLeftOpen size={16} />}
        </button>
        <nav aria-label="Breadcrumb" className="flex min-w-0 items-center gap-1.5 text-[13px]">
          <Link href="/" className="truncate text-text-2 hover:text-text">
            {project.name}
          </Link>
          <ChevronRight size={13} className="text-text-3" />
          <h1 className="font-semibold">Graph Explorer</h1>
          <span className="font-mono text-[10px] text-text-3">
            ontology v{phase.ontology.version} · read-only
          </span>
        </nav>
        <form
          onSubmit={lookupEntity}
          className="relative z-20 mx-auto flex w-full max-w-[420px] flex-col"
        >
          <label className="relative flex items-center">
            <Search size={13} className="pointer-events-none absolute left-2.5 text-text-3" />
            <input
              aria-label="Open entity by ID"
              placeholder="Open an entity by ID…"
              className={cn(inputClass, "pl-8")}
              value={lookup.value}
              onChange={(e) => setLookup({ value: e.target.value, error: null, busy: false })}
            />
            {lookup.busy && (
              <LoaderCircle size={13} className="absolute right-2.5 animate-spin text-text-3" />
            )}
          </label>
          {lookup.error && (
            <span
              role="alert"
              className="absolute top-full left-0 mt-1 rounded-[4px] border border-danger/40 bg-panel px-2 py-1 text-[11.5px] text-danger shadow-lg shadow-black/40"
            >
              {lookup.error}
            </span>
          )}
        </form>
        <Link
          href={`/graphs/${graphId}/builder`}
          className="inline-flex h-8 shrink-0 items-center gap-2 rounded-[4px] border border-border-strong bg-raised px-3 text-[13px] font-semibold hover:bg-hover"
        >
          Graph Builder
        </Link>
      </header>

      <div className="flex min-h-0 flex-1">
        {sidebarOpen && ontology && (
          <ExplorerSidebar
            ontology={ontology}
            search={search}
            state={state}
            loadedByType={loadedByType}
            pickLabel={
              panel === "path" && picking ? (picking === "source" ? "Source" : "Target") : null
            }
            hidden={{
              entities: state.hiddenEntityIds.size,
              relationships: state.hiddenRelationshipIds.size,
            }}
            loaded={graph.order}
            onSearch={(form) => void runSearch(form)}
            onOpenEntity={(entity) => openEntity(entity.id)}
            onPick={(entity) => picking && pickEndpoint(picking, entity)}
            onToggleEntityType={(name) => dispatch({ type: "toggleEntityType", name })}
            onToggleRelationshipType={(name) => dispatch({ type: "toggleRelationshipType", name })}
            onShowHidden={() => dispatch({ type: "showHidden" })}
            onRemoveHidden={unloadHidden}
            onResetView={resetView}
          />
        )}

        <main className="relative min-w-0 flex-1 overflow-hidden bg-bg" aria-label="Graph canvas">
          <Canvas graph={graph} state={state} controllerRef={controllerRef} {...canvasHandlers} />

          <div className="pointer-events-none absolute inset-x-0 top-3 flex flex-col items-center gap-2 px-4">
            <GraphToolbar
              controllerRef={controllerRef}
              ready={ready}
              hasGraph={graph.order > 0}
              layoutMode={state.layoutMode}
              selectedEntity={state.selectedEntityId}
              expanding={state.selectedEntityId ? expanding.has(state.selectedEntityId) : false}
              canReset={canReset}
              onLayout={layout}
              onExpand={() => state.selectedEntityId && void expand(state.selectedEntityId)}
              onFindPath={() => openPathFinder()}
              onResetHighlight={resetHighlight}
            />
            {busy && (
              <span
                role="status"
                className="pointer-events-auto inline-flex items-center gap-2 rounded-full border border-border-strong bg-panel px-3 py-1 text-[12px] text-text-2"
              >
                <LoaderCircle size={12} className="animate-spin" />
                {pathRun.running
                  ? "Finding path…"
                  : connecting
                    ? "Loading connections…"
                    : "Loading neighbourhood…"}
              </span>
            )}
            {picking && panel === "path" && (
              <span className="pointer-events-auto rounded-full border border-accent/60 bg-accent-deep px-3 py-1 text-[12px] text-accent">
                Click an entity to use as the path {picking} · Esc to cancel
              </span>
            )}
            {notice && (
              <div
                role={notice.tone === "error" ? "alert" : "status"}
                className={cn(
                  "pointer-events-auto flex max-w-[640px] items-start gap-3 rounded-[6px] border bg-panel px-3 py-2 text-[12.5px]",
                  NOTICE_TONE[notice.tone],
                )}
              >
                <span className="flex-1">{notice.text}</span>
                {notice.budget && state.hiddenEntityIds.size > 0 && (
                  <Button size="sm" onClick={unloadHidden}>
                    Unload hidden
                  </Button>
                )}
                {notice.budget && (
                  <Button size="sm" onClick={resetView}>
                    Reset view
                  </Button>
                )}
                <button
                  type="button"
                  aria-label="Dismiss"
                  onClick={() => setNotice(null)}
                  className="text-text-3 hover:text-text"
                >
                  <X size={13} />
                </button>
              </div>
            )}
          </div>

          {empty && (
            <div className="absolute inset-0 flex items-center justify-center p-6">
              <div className="flex max-w-sm flex-col items-center gap-3 text-center">
                <h2 className="text-[18px] font-bold">Your graph is empty</h2>
                <p className="text-[13px] text-text-2">
                  Upload a dataset or use the Graph Builder to create your first knowledge graph.
                </p>
                <div className="flex gap-2">
                  <Link
                    href={`/graphs/${graphId}/builder`}
                    className="inline-flex h-9 items-center rounded-[4px] bg-accent px-[18px] text-[14px] font-semibold text-on-accent hover:bg-accent/90"
                  >
                    Open Graph Builder
                  </Link>
                  <Link
                    href={`/graphs/${graphId}/upload`}
                    className="inline-flex h-9 items-center rounded-[4px] border border-border-strong bg-raised px-[18px] text-[14px] font-semibold hover:bg-hover"
                  >
                    Upload data
                  </Link>
                </div>
              </div>
            </div>
          )}
          {!empty && graph.order === 0 && expanding.size === 0 && (
            <div className="pointer-events-none absolute inset-0 flex items-center justify-center p-6">
              <div className="flex max-w-sm flex-col items-center gap-2 text-center">
                <h2 className="text-[16px] font-semibold">
                  Select an entity to start exploring relationships.
                </h2>
                <p className="text-[12.5px] text-text-3">
                  Search by exact property value on the left, or open an entity by its ID above.
                </p>
              </div>
            </div>
          )}

          {menu && menuNode && (
            <div
              role="menu"
              aria-label={`Actions for ${menuNode.label}`}
              className="absolute z-10 flex w-[220px] flex-col rounded-[6px] border border-border-strong bg-panel p-1 shadow-xl shadow-black/40"
              style={{ left: menu.x, top: menu.y }}
            >
              {MENU.map(([action, label, key, Icon]) => (
                <button
                  key={action}
                  type="button"
                  role="menuitem"
                  onClick={() => runMenu(action, menu.entityId)}
                  className="flex items-center gap-2.5 rounded-[4px] px-2.5 py-1.5 text-left text-[13px] hover:bg-raised"
                >
                  <Icon size={14} className="text-text-3" />
                  <span className="flex-1">{label}</span>
                  {key && <span className="font-mono text-[10px] text-text-3">{key}</span>}
                </button>
              ))}
            </div>
          )}

          {lastOperation && (
            <div className="absolute bottom-3 left-3 flex max-w-[70%] items-center gap-2 rounded-[4px] border border-border bg-panel/95 px-2.5 py-1 font-mono text-[10.5px] text-text-2">
              <TerminalSquare size={12} className="shrink-0 text-text-3" />
              <span className="truncate">{lastOperation}</span>
            </div>
          )}
        </main>

        <aside
          aria-label="Details"
          className="flex w-[330px] shrink-0 flex-col border-l border-border bg-panel"
        >
          {panel === "path" ? (
            <PathFinder
              form={pathForm}
              picking={picking}
              visible={visible}
              running={pathRun.running}
              error={pathRun.error}
              result={state.activePath}
              onChange={(patch) => setPathForm((form) => ({ ...form, ...patch }))}
              onPick={setPicking}
              onLookup={lookupEndpoint}
              onRun={(patch) => void runPath(patch)}
              onOpenEntity={openEntity}
              onClear={() => {
                setPathForm((form) => ({ ...form, source: null, target: null }));
                setPicking("source");
                dispatch({ type: "resetHighlight" });
              }}
              onClose={() => {
                setPicking(null);
                setPanel("details");
              }}
            />
          ) : selectedNode ? (
            <EntityDetails
              node={selectedNode}
              groups={relationshipGroups(graph, state, selectedNode.id)}
              expanding={expanding.has(selectedNode.id)}
              connecting={connecting}
              options={options}
              relationshipTypes={(ontology?.relationship_types ?? []).map((type) => type.name)}
              onOptions={setOptions}
              onExpand={() => void expand(selectedNode.id)}
              onConnections={() => void connections(selectedNode.id)}
              onFindPath={() => openPathFinder(selectedNode.id)}
              onFocus={() => controllerRef.current?.center(selectedNode.id)}
              collapsible={collapsibleNeighbors(graph, state, selectedNode.id).length}
              onCollapse={() => collapse(selectedNode.id)}
              onHide={() => hideEntity(selectedNode.id)}
              onClose={() => dispatch({ type: "clearSelection" })}
              onOpenEntity={openEntity}
              onSelectRelationship={(id) => dispatch({ type: "selectRelationship", id })}
            />
          ) : selectedEdge ? (
            <RelationshipDetails
              edge={selectedEdge}
              source={
                graph.hasNode(selectedEdge.source)
                  ? graph.getNodeAttributes(selectedEdge.source)
                  : undefined
              }
              target={
                graph.hasNode(selectedEdge.target)
                  ? graph.getNodeAttributes(selectedEdge.target)
                  : undefined
              }
              onOpenEntity={openEntity}
              onHide={() => dispatch({ type: "hideRelationship", id: selectedEdge.id })}
              onClose={() => dispatch({ type: "clearSelection" })}
            />
          ) : (
            <DetailsHint empty={empty} />
          )}
        </aside>
      </div>

      {/* Status bar: client-side numbers only, never totals of the stored graph. */}
      <footer className="flex h-7 shrink-0 items-center gap-4 border-t border-border bg-panel px-3 font-mono text-[10.5px] text-text-3">
        <span>
          <b className="font-medium text-text-2">{graph.order}</b> entities loaded
        </span>
        <span>
          <b className="font-medium text-text-2">{graph.size}</b> relationships loaded
        </span>
        <span title="Browser limit for this view, not a limit of the graph">
          view budget {budget.maxEntities} / {budget.maxRelationships}
        </span>
        <span>layout: {state.layoutMode}</span>
        {(state.hiddenEntityIds.size > 0 || state.hiddenRelationshipIds.size > 0) && (
          <span>{state.hiddenEntityIds.size + state.hiddenRelationshipIds.size} hidden</span>
        )}
        <span className="ml-auto">
          <Kicker>read-only</Kicker>
        </span>
      </footer>
    </Frame>
  );
}

/** The page frame: the app rail and a full-height column. */
function Frame({ graphId, children }: { graphId: string; children: ReactNode }) {
  return (
    <div className="flex h-screen overflow-hidden bg-bg">
      <Rail graphId={graphId} />
      <div className="relative flex min-w-0 flex-1 flex-col">{children}</div>
    </div>
  );
}
