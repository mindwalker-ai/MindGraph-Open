"use client";

/**
 * Shortest path between two entities. MindGraph Core computes the path
 * (bounded by the maximum depth); the browser only shows and highlights
 * it. Endpoints come from the canvas, the search results, the entities in
 * view or a pasted EntityId.
 */

import { ArrowLeft, ArrowRight, Crosshair, LoaderCircle, Route, X } from "lucide-react";
import { useState } from "react";

import { Button, ErrorNote, Kicker, inputClass } from "@/components/ui/primitives";
import { DIRECTIONS, type Direction, type Entity, EXPLORER_LIMITS } from "@/lib/api/types";
import { typeStyle } from "@/lib/graph/palette";
import type { ActivePath } from "@/lib/graph/state";
import { entityLabel, shortId } from "@/lib/graph/transform";
import { cn } from "@/lib/utils";

export type Endpoint = "source" | "target";

export interface PathForm {
  source: Entity | null;
  target: Entity | null;
  direction: Direction;
  maxDepth: number;
}

export interface PathFinderProps {
  form: PathForm;
  picking: Endpoint | null;
  /** Entities in view, to choose an endpoint from. */
  visible: Entity[];
  running: boolean;
  error: string | null;
  result: ActivePath | null;
  onChange(patch: Partial<PathForm>): void;
  onPick(endpoint: Endpoint | null): void;
  /** Resolves a pasted EntityId; returns an error message, or null. */
  onLookup(endpoint: Endpoint, entityId: string): Promise<string | null>;
  onRun(patch?: Partial<PathForm>): void;
  onOpenEntity(entityId: string): void;
  onClear(): void;
  onClose(): void;
}

const DIRECTION_LABEL: Record<Direction, string> = {
  both: "Both",
  outgoing: "Outgoing",
  incoming: "Incoming",
};

function EndpointField({
  endpoint,
  entity,
  picking,
  visible,
  onChange,
  onPick,
  onLookup,
}: {
  endpoint: Endpoint;
  entity: Entity | null;
  picking: boolean;
  visible: Entity[];
  onChange(entity: Entity | null): void;
  onPick(): void;
  onLookup(entityId: string): Promise<string | null>;
}) {
  const [id, setId] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const title = endpoint === "source" ? "Source" : "Target";

  // Not a <form>: this field sits inside the path finder's own form, and a
  // nested form would submit natively and reload the page.
  const lookup = async () => {
    if (!id.trim() || busy) return;
    setBusy(true);
    const failure = await onLookup(id.trim());
    setBusy(false);
    setError(failure);
    if (!failure) setId("");
  };

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center justify-between">
        <Kicker>{title}</Kicker>
        <button
          type="button"
          onClick={onPick}
          aria-pressed={picking}
          className={cn(
            "inline-flex items-center gap-1 text-[11px]",
            picking ? "text-accent" : "text-text-3 hover:text-text",
          )}
        >
          <Crosshair size={11} /> {picking ? "Click an entity…" : "Pick on canvas"}
        </button>
      </div>
      {entity ? (
        <div className="flex items-center gap-2 rounded-[4px] border border-border bg-bg px-3 py-2">
          <span
            aria-hidden
            className="h-2.5 w-2.5 shrink-0 rounded-full"
            style={{ background: typeStyle(entity.entity_type).cssColor }}
          />
          <span className="flex min-w-0 flex-1 flex-col">
            <span className="truncate text-[13px]">{entityLabel(entity)}</span>
            <span className="text-[11px] text-text-3">
              {entity.entity_type} · {shortId(entity.id)}
            </span>
          </span>
          <button
            type="button"
            aria-label={`Clear ${endpoint}`}
            onClick={() => onChange(null)}
            className="text-text-3 hover:text-text"
          >
            <X size={13} />
          </button>
        </div>
      ) : (
        <>
          <select
            aria-label={`${title} from the entities in view`}
            className={inputClass}
            value=""
            onChange={(e) => onChange(visible.find((v) => v.id === e.target.value) ?? null)}
          >
            <option value="">Choose an entity in view…</option>
            {visible.map((v) => (
              <option key={v.id} value={v.id}>
                {entityLabel(v)} · {v.entity_type}
              </option>
            ))}
          </select>
          <div className="flex gap-1.5">
            <input
              aria-label={`${title} entity ID`}
              placeholder="or paste an entity ID"
              className={inputClass}
              value={id}
              onChange={(e) => setId(e.target.value)}
              onKeyDown={(e) => {
                // Enter uses the ID instead of submitting the path search.
                if (e.key !== "Enter") return;
                e.preventDefault();
                void lookup();
              }}
            />
            <Button size="md" disabled={busy || !id.trim()} onClick={() => void lookup()}>
              Use
            </Button>
          </div>
          {error && <p className="text-[11.5px] text-danger">{error}</p>}
        </>
      )}
    </div>
  );
}

function PathResult({
  result,
  onOpenEntity,
  onRun,
  onClear,
}: {
  result: ActivePath;
  onOpenEntity(entityId: string): void;
  onRun(patch?: Partial<PathForm>): void;
  onClear(): void;
}) {
  const { path } = result;
  if (!path.found) {
    return (
      <section className="flex flex-col gap-2 rounded-[6px] border border-border bg-bg p-3">
        <p className="text-[14px] font-semibold">No path found</p>
        <p className="text-[12.5px] text-text-2">
          No connection was found within {result.maxDepth} {result.maxDepth === 1 ? "hop" : "hops"}
          {result.direction === "both" ? "" : ` following ${result.direction} relationships`}. The
          entities may still be connected through a longer path
          {result.direction === "both" ? "" : " or in the other direction"}.
        </p>
        <div className="flex flex-wrap gap-2">
          {result.maxDepth < EXPLORER_LIMITS.pathMaxDepth && (
            <Button size="sm" onClick={() => onRun({ maxDepth: result.maxDepth + 1 })}>
              Increase max depth to {result.maxDepth + 1}
            </Button>
          )}
          {result.direction !== "both" && (
            <Button size="sm" onClick={() => onRun({ direction: "both" })}>
              Search both directions
            </Button>
          )}
          <Button size="sm" variant="ghost" onClick={onClear}>
            Clear selection
          </Button>
        </div>
      </section>
    );
  }
  return (
    <section className="flex flex-col gap-3">
      <div className="flex gap-4 font-mono text-[11px] text-text-2">
        <span>
          <b className="text-text">{path.hops}</b> {path.hops === 1 ? "hop" : "hops"}
        </span>
        <span>
          <b className="text-text">{path.entities.length}</b> entities
        </span>
        <span>
          <b className="text-text">{path.relationships.length}</b> relationships
        </span>
      </div>
      <ol className="flex flex-col">
        {path.entities.map((entity, index) => {
          const relationship = path.relationships[index];
          const forward = relationship?.source_id === entity.id;
          return (
            <li key={`${entity.id}-${index}`} className="flex flex-col">
              <button
                type="button"
                onClick={() => onOpenEntity(entity.id)}
                className="flex items-center gap-2 rounded-[4px] px-2 py-1.5 text-left hover:bg-raised"
              >
                <span
                  aria-hidden
                  className="h-2.5 w-2.5 shrink-0 rounded-full"
                  style={{ background: typeStyle(entity.entity_type).cssColor }}
                />
                <span className="truncate text-[13px]">{entityLabel(entity)}</span>
                <span className="ml-auto text-[11px] text-text-3">{entity.entity_type}</span>
              </button>
              {relationship && (
                <span className="ml-[13px] flex items-center gap-1.5 border-l border-accent/60 py-1 pl-3 font-mono text-[10.5px] text-accent">
                  {forward ? <ArrowRight size={11} /> : <ArrowLeft size={11} />}
                  {relationship.relationship_type}
                </span>
              )}
            </li>
          );
        })}
      </ol>
    </section>
  );
}

export function PathFinder(props: PathFinderProps) {
  const { form, picking } = props;
  const ready = Boolean(form.source && form.target) && !props.running;
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
      <div className="flex items-center gap-2 border-b border-border px-4 pt-4 pb-3">
        <Route size={15} className="text-accent" />
        <h2 className="flex-1 text-[15px] font-bold">Find Path</h2>
        <button
          type="button"
          aria-label="Close path finder"
          onClick={props.onClose}
          className="text-text-3 hover:text-text"
        >
          <X size={14} />
        </button>
      </div>
      <form
        className="flex flex-col gap-3 border-b border-border px-4 py-3"
        onSubmit={(event) => {
          event.preventDefault();
          if (ready) props.onRun();
        }}
      >
        <EndpointField
          endpoint="source"
          entity={form.source}
          picking={picking === "source"}
          visible={props.visible}
          onChange={(source) => props.onChange({ source })}
          onPick={() => props.onPick(picking === "source" ? null : "source")}
          onLookup={(id) => props.onLookup("source", id)}
        />
        <EndpointField
          endpoint="target"
          entity={form.target}
          picking={picking === "target"}
          visible={props.visible}
          onChange={(target) => props.onChange({ target })}
          onPick={() => props.onPick(picking === "target" ? null : "target")}
          onLookup={(id) => props.onLookup("target", id)}
        />
        <div className="grid grid-cols-2 gap-2">
          <label className="flex flex-col gap-1 text-[12px]">
            <span className="text-text-3">Direction</span>
            <select
              className={inputClass}
              value={form.direction}
              onChange={(e) => props.onChange({ direction: e.target.value as Direction })}
            >
              {DIRECTIONS.map((direction) => (
                <option key={direction} value={direction}>
                  {DIRECTION_LABEL[direction]}
                </option>
              ))}
            </select>
          </label>
          <label className="flex flex-col gap-1 text-[12px]">
            <span className="text-text-3">Max depth (hops)</span>
            <select
              className={inputClass}
              value={form.maxDepth}
              onChange={(e) => props.onChange({ maxDepth: Number(e.target.value) })}
            >
              {Array.from({ length: EXPLORER_LIMITS.pathMaxDepth }, (_, i) => i + 1).map((d) => (
                <option key={d} value={d}>
                  {d}
                </option>
              ))}
            </select>
          </label>
        </div>
        <Button
          type="submit"
          variant="primary"
          disabled={!ready}
          icon={
            props.running ? (
              <LoaderCircle size={14} className="animate-spin" />
            ) : (
              <Route size={14} />
            )
          }
        >
          {props.running ? "Finding path…" : "Find shortest path"}
        </Button>
        {props.error && <ErrorNote>{props.error}</ErrorNote>}
      </form>
      <div className="flex flex-col gap-3 px-4 py-3">
        {props.result ? (
          <PathResult
            result={props.result}
            onOpenEntity={props.onOpenEntity}
            onRun={props.onRun}
            onClear={props.onClear}
          />
        ) : (
          <p className="text-[12px] text-text-3">
            MindGraph Core finds the path with the fewest hops, up to the maximum depth (at most{" "}
            {EXPLORER_LIMITS.pathMaxDepth}).
          </p>
        )}
      </div>
    </div>
  );
}
