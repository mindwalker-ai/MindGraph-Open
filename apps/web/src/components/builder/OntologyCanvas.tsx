"use client";

/**
 * "Ontology Preview" (mockup 05, centre): entity types as nodes and
 * relationship types as labelled edges on a drafting grid, or the same
 * draft as YAML. Clicking a node or an edge label selects it.
 */

import { Plus } from "lucide-react";
import { useMemo, useState } from "react";

import { sourceLabel } from "@/components/builder/EntityPanel";
import { TypeDisc } from "@/components/builder/appearance";
import { Button, inputClass } from "@/components/ui/primitives";
import { type BuilderDraft, joinHint, relationshipMappingsOf, toYaml } from "@/lib/builder/draft";
import { NODE_HEIGHT, NODE_WIDTH, edgeEnds, layoutGraph } from "@/lib/builder/layout";
import { cn } from "@/lib/utils";

export type Selection = { kind: "entity" | "relationship"; key: string } | null;

export function OntologyCanvas({
  draft,
  selection,
  onSelect,
  onAddRelationship,
  locked,
}: {
  draft: BuilderDraft;
  selection: Selection;
  onSelect: (selection: Selection) => void;
  onAddRelationship: (sourceTypeKey: string, targetTypeKey: string) => void;
  locked: boolean;
}) {
  const [view, setView] = useState<"graph" | "yaml">("graph");
  const [adding, setAdding] = useState<{ source: string; target: string } | null>(null);
  const layout = useMemo(
    () =>
      layoutGraph(
        draft.types.map((t) => t.key),
        draft.relationshipTypes.map((r) => ({ source: r.sourceTypeKey, target: r.targetTypeKey })),
      ),
    [draft.types, draft.relationshipTypes],
  );
  const files = draft.datasets.length;

  // Several relationship types between the same pair are drawn side by side.
  const pairIndex = new Map<string, number>();
  const edges = draft.relationshipTypes.flatMap((type) => {
    const from = layout.positions.get(type.sourceTypeKey);
    const to = layout.positions.get(type.targetTypeKey);
    if (!from || !to) return [];
    const pair = [type.sourceTypeKey, type.targetTypeKey].sort().join("|");
    const index = pairIndex.get(pair) ?? 0;
    pairIndex.set(pair, index + 1);
    const self = type.sourceTypeKey === type.targetTypeKey;
    const ends = self
      ? { x1: from.x + NODE_WIDTH, y1: from.y + 14, x2: from.x + NODE_WIDTH, y2: from.y + 44 }
      : edgeEnds(from, to);
    const shift = index * 18;
    return [{ type, ends, shift, self }];
  });

  const selectedEdge =
    selection?.kind === "relationship"
      ? edges.find((e) => e.type.key === selection.key)
      : undefined;
  const hint = (() => {
    if (!selectedEdge) return null;
    for (const { dataset, mapping } of relationshipMappingsOf(draft, selectedEdge.type.key)) {
      const found = joinHint(draft, dataset, mapping);
      if (found) return found;
    }
    return null;
  })();

  return (
    <section
      aria-label="Ontology preview"
      className="relative flex min-w-0 flex-1 flex-col overflow-hidden bg-bg"
    >
      <header className="z-10 flex items-center gap-2 px-5 pt-[18px]">
        <div className="flex flex-1 flex-col gap-0.5">
          <h2 className="text-[15px] font-bold">Ontology Preview</h2>
          <span className="font-mono text-[10px] font-medium text-text-3">
            {draft.types.length} entit{draft.types.length === 1 ? "y" : "ies"} ·{" "}
            {draft.relationshipTypes.length} relationship
            {draft.relationshipTypes.length === 1 ? "" : "s"}
            {files > 0 && ` · ${files} file${files === 1 ? "" : "s"}`}
          </span>
        </div>
        <Button
          icon={<Plus size={14} />}
          disabled={locked || draft.types.length === 0}
          onClick={() =>
            setAdding({
              source: draft.types[0]?.key ?? "",
              target: draft.types[1]?.key ?? draft.types[0]?.key ?? "",
            })
          }
          className="h-[30px] text-[12.5px]"
        >
          Add Relationship
        </Button>
        <div
          role="tablist"
          aria-label="View"
          className="flex gap-0.5 rounded-[5px] border border-border bg-panel p-0.5"
        >
          {(["graph", "yaml"] as const).map((mode) => (
            <button
              key={mode}
              type="button"
              role="tab"
              aria-selected={view === mode}
              onClick={() => setView(mode)}
              className={cn(
                "h-6 rounded-[3px] px-[10px] text-[12px] font-semibold",
                view === mode ? "bg-hover text-text" : "text-text-3 hover:text-text-2",
              )}
            >
              {mode === "graph" ? "Graph" : "YAML"}
            </button>
          ))}
        </div>
      </header>

      {adding && (
        <div className="z-10 mx-5 mt-3 flex flex-wrap items-center gap-2 rounded-[6px] border border-border-strong bg-panel p-3">
          <span className="text-[12px] text-text-3">From</span>
          <select
            aria-label="Relationship source type"
            className={cn(inputClass, "w-40")}
            value={adding.source}
            onChange={(event) => setAdding({ ...adding, source: event.target.value })}
          >
            {draft.types.map((t) => (
              <option key={t.key} value={t.key}>
                {t.name}
              </option>
            ))}
          </select>
          <span className="text-[12px] text-text-3">to</span>
          <select
            aria-label="Relationship target type"
            className={cn(inputClass, "w-40")}
            value={adding.target}
            onChange={(event) => setAdding({ ...adding, target: event.target.value })}
          >
            {draft.types.map((t) => (
              <option key={t.key} value={t.key}>
                {t.name}
              </option>
            ))}
          </select>
          <Button
            variant="primary"
            size="sm"
            onClick={() => {
              onAddRelationship(adding.source, adding.target);
              setAdding(null);
            }}
          >
            Add
          </Button>
          <Button variant="ghost" onClick={() => setAdding(null)}>
            Cancel
          </Button>
        </div>
      )}

      {view === "yaml" ? (
        <pre
          aria-label="Draft as YAML"
          className="m-5 flex-1 overflow-auto rounded-[6px] border border-border bg-panel p-4 font-mono text-[11.5px] leading-[18px] text-text-2"
        >
          {toYaml(draft)}
        </pre>
      ) : (
        <div className="absolute inset-0 overflow-auto bg-[linear-gradient(var(--panel)_1px,transparent_1px),linear-gradient(90deg,var(--panel)_1px,transparent_1px)] bg-[size:32px_32px]">
          <div
            className="relative mx-auto"
            style={{ width: layout.width, height: Math.max(layout.height, 560) }}
          >
            <svg
              className="absolute top-0 left-0"
              width={layout.width}
              height={Math.max(layout.height, 560)}
              aria-hidden
            >
              <defs>
                {[
                  ["arrow", "var(--edge)"],
                  ["arrow-selected", "var(--accent)"],
                  ["arrow-existing", "var(--t-ip)"],
                ].map(([id, color]) => (
                  <marker
                    key={id}
                    id={id}
                    viewBox="0 0 10 10"
                    refX="9"
                    refY="5"
                    markerWidth="7"
                    markerHeight="7"
                    orient="auto-start-reverse"
                  >
                    <path d="M0,0 L10,5 L0,10 z" fill={color} />
                  </marker>
                ))}
              </defs>
              {edges.map(({ type, ends, shift, self }) => {
                const selected = selection?.key === type.key;
                const color = selected
                  ? "var(--accent)"
                  : type.existing
                    ? "var(--t-ip)"
                    : "var(--edge)";
                const marker = selected
                  ? "arrow-selected"
                  : type.existing
                    ? "arrow-existing"
                    : "arrow";
                if (self) {
                  return (
                    <path
                      key={type.key}
                      d={`M${ends.x1},${ends.y1} C${ends.x1 + 50 + shift},${ends.y1 - 20} ${ends.x2 + 50 + shift},${ends.y2 + 20} ${ends.x2},${ends.y2}`}
                      fill="none"
                      stroke={color}
                      strokeWidth={selected ? 1.5 : 1}
                      markerEnd={`url(#${marker})`}
                    />
                  );
                }
                return (
                  <line
                    key={type.key}
                    x1={ends.x1 + shift}
                    y1={ends.y1}
                    x2={ends.x2 + shift}
                    y2={ends.y2}
                    stroke={color}
                    strokeWidth={selected ? 1.5 : 1}
                    strokeDasharray={type.existing && !selected ? "4 4" : undefined}
                    markerEnd={`url(#${marker})`}
                  />
                );
              })}
            </svg>

            {edges.map(({ type, ends, shift, self }) => {
              const selected = selection?.key === type.key;
              const x = self ? ends.x1 + 56 + shift : (ends.x1 + ends.x2) / 2 + shift;
              const y = self ? (ends.y1 + ends.y2) / 2 : (ends.y1 + ends.y2) / 2;
              return (
                <button
                  key={type.key}
                  type="button"
                  aria-label={`Relationship ${type.name}`}
                  aria-pressed={selected}
                  onClick={() => onSelect({ kind: "relationship", key: type.key })}
                  className={cn(
                    "absolute flex h-[22px] -translate-x-1/2 -translate-y-1/2 items-center gap-2 rounded-[11px] border px-2 font-mono text-[10px] font-medium whitespace-nowrap",
                    selected
                      ? "border-accent bg-accent-deep text-accent"
                      : "border-border-strong bg-panel text-text hover:border-text-3",
                  )}
                  style={{ left: x, top: y }}
                >
                  {type.name}
                  {type.confidence !== null && (
                    <span className={selected ? "text-accent" : "text-text-3"}>
                      {Math.round(type.confidence * 100)}%
                    </span>
                  )}
                </button>
              );
            })}

            {selectedEdge && hint && (
              <div
                className="absolute flex flex-col gap-[3px] rounded-[4px] border border-border-strong bg-panel px-[10px] py-[7px] font-mono text-[10px] font-medium whitespace-nowrap"
                style={{
                  left: (selectedEdge.ends.x1 + selectedEdge.ends.x2) / 2 + selectedEdge.shift + 70,
                  top: (selectedEdge.ends.y1 + selectedEdge.ends.y2) / 2 - 14,
                }}
              >
                <span>{hint.from}</span>
                <span className="text-text-2">→ {hint.to}</span>
              </div>
            )}

            {draft.types.map((type) => {
              const position = layout.positions.get(type.key);
              if (!position) return null;
              const selected = selection?.kind === "entity" && selection.key === type.key;
              const related =
                selectedEdge &&
                (selectedEdge.type.sourceTypeKey === type.key ||
                  selectedEdge.type.targetTypeKey === type.key);
              return (
                <button
                  key={type.key}
                  type="button"
                  aria-label={`Entity ${type.name}`}
                  aria-pressed={selected}
                  onClick={() => onSelect({ kind: "entity", key: type.key })}
                  className={cn(
                    "absolute flex items-center gap-[10px] rounded-[6px] border bg-panel px-3 text-left",
                    selected || related
                      ? "border-accent/50"
                      : "border-border-strong hover:border-text-3",
                  )}
                  style={{
                    left: position.x,
                    top: position.y,
                    width: NODE_WIDTH,
                    height: NODE_HEIGHT,
                  }}
                >
                  <TypeDisc typeName={type.name} size={30} />
                  <span className="flex min-w-0 flex-1 flex-col gap-px">
                    <span className="truncate text-[14px] font-bold">{type.name}</span>
                    <span className="truncate font-mono text-[9.5px] font-medium text-text-3">
                      {sourceLabel(draft, type)}
                    </span>
                  </span>
                  <span className="font-mono text-[9.5px] font-medium text-text-3">
                    {type.properties.length} field{type.properties.length === 1 ? "" : "s"}
                  </span>
                </button>
              );
            })}

            {draft.types.length === 0 && (
              <p className="absolute top-40 w-full text-center text-[13px] text-text-3">
                No entity types yet. Add one from the left panel.
              </p>
            )}
          </div>
        </div>
      )}

      {view === "graph" && (
        <div className="pointer-events-none absolute bottom-5 left-5 z-10 flex items-center gap-4">
          {[
            ["Detected", "bg-edge"],
            ["Selected", "bg-accent"],
            ["Existing", "bg-[var(--t-ip)]"],
          ].map(([label, color]) => (
            <span key={label} className="flex items-center gap-1.5">
              <span className={cn("h-0.5 w-4", color)} />
              <span className="font-mono text-[10px] font-medium text-text-3">{label}</span>
            </span>
          ))}
        </div>
      )}
    </section>
  );
}
