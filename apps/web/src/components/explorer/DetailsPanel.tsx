"use client";

/**
 * The right panel of the Graph Explorer: the selected entity or
 * relationship (mockups 06 and 07). Read-only: no control here changes the
 * stored graph; Hide only affects this view.
 */

import {
  ArrowLeft,
  ArrowRight,
  Copy,
  EyeOff,
  Expand,
  Focus,
  Link2,
  LoaderCircle,
  MousePointerClick,
  Repeat,
  Route,
  Shrink,
  X,
} from "lucide-react";
import { useState } from "react";

import { IdentityCard } from "@/components/explorer/IdentityCard";
import { Button, Kicker, inputClass } from "@/components/ui/primitives";
import type { ApiClient } from "@/lib/api/client";
import {
  DIRECTIONS,
  type Direction,
  EXPLORER_LIMITS,
  type NeighborOptions,
  type PropertyJson,
  type PropertyType,
} from "@/lib/api/types";
import { typeStyle } from "@/lib/graph/palette";
import type { RelationshipGroup } from "@/lib/graph/state";
import {
  type EdgeAttributes,
  type NodeAttributes,
  formatValue,
  shortId,
} from "@/lib/graph/transform";
import { cn } from "@/lib/utils";

function CopyId({ id }: { id: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      title={`Copy ${id}`}
      aria-label="Copy ID"
      onClick={() => {
        void navigator.clipboard?.writeText(id).then(() => setCopied(true));
      }}
      className="inline-flex items-center gap-1.5 font-mono text-[11px] text-text-3 hover:text-text"
    >
      {shortId(id)}
      <Copy size={11} />
      {copied && <span className="text-ok">copied</span>}
    </button>
  );
}

function Header({
  kicker,
  color,
  title,
  id,
  onClose,
}: {
  kicker: string;
  color: string;
  title: string;
  id: string;
  onClose(): void;
}) {
  return (
    <div className="flex flex-col gap-1.5 border-b border-border px-4 pt-4 pb-3">
      <div className="flex items-center gap-2">
        <span
          aria-hidden
          className="h-2.5 w-2.5 rounded-full"
          style={{ boxShadow: `inset 0 0 0 2px ${color}` }}
        />
        <Kicker className="flex-1">
          <span style={{ color }}>{kicker}</span>
        </Kicker>
        <button
          type="button"
          aria-label="Close details"
          onClick={onClose}
          className="text-text-3 hover:text-text"
        >
          <X size={14} />
        </button>
      </div>
      <h2 className="text-[20px] leading-tight font-bold break-words">{title}</h2>
      <CopyId id={id} />
    </div>
  );
}

function Value({ value, type }: { value: PropertyJson; type: PropertyType | undefined }) {
  if (type === "string_array" && Array.isArray(value)) {
    return (
      <span className="flex flex-wrap gap-1">
        {value.map((item, index) => (
          <span key={index} className="rounded-[3px] border border-border px-1.5 text-[12px]">
            {item}
          </span>
        ))}
      </span>
    );
  }
  const mono = type && type !== "string";
  return (
    <span className={cn("break-words", mono ? "font-mono text-[12px]" : "text-[13px]")}>
      {formatValue(value)}
    </span>
  );
}

/** Typed properties; values are text (escaped by React), never HTML. */
export function PropertyList({
  properties,
  types,
}: {
  properties: Record<string, PropertyJson>;
  types: Record<string, PropertyType>;
}) {
  const keys = Object.keys(properties).sort();
  return (
    <section className="flex flex-col gap-2 border-b border-border px-4 py-3">
      <div className="flex items-center justify-between">
        <Kicker>Properties</Kicker>
        <span className="font-mono text-[10px] text-text-3">{keys.length}</span>
      </div>
      {keys.length === 0 ? (
        <p className="text-[12px] text-text-3">No properties.</p>
      ) : (
        <dl className="grid grid-cols-[minmax(0,110px)_minmax(0,1fr)] gap-x-3 gap-y-2">
          {keys.map((key) => (
            <div key={key} className="contents">
              <dt
                className="truncate font-mono text-[11px] text-text-3"
                title={`${key} · ${types[key] ?? "unknown"}`}
              >
                {key}
              </dt>
              <dd className="min-w-0 text-text">
                <Value value={properties[key]} type={types[key]} />
              </dd>
            </div>
          ))}
        </dl>
      )}
    </section>
  );
}

const DIRECTION_LABEL: Record<Direction, string> = {
  both: "Both directions",
  outgoing: "Outgoing",
  incoming: "Incoming",
};

function ExpandOptions({
  options,
  relationshipTypes,
  onChange,
}: {
  options: NeighborOptions;
  relationshipTypes: string[];
  onChange(options: NeighborOptions): void;
}) {
  return (
    <details className="group text-[12px]">
      <summary className="cursor-pointer text-text-3 select-none hover:text-text-2">
        Expand options · {DIRECTION_LABEL[options.direction ?? "both"]}
        {options.relationship_type ? ` · ${options.relationship_type}` : ""} · limit{" "}
        {options.limit ?? EXPLORER_LIMITS.neighborDefault}
      </summary>
      <div className="mt-2 grid grid-cols-2 gap-2">
        <label className="flex flex-col gap-1">
          <span className="text-text-3">Direction</span>
          <select
            className={inputClass}
            value={options.direction ?? "both"}
            onChange={(e) => onChange({ ...options, direction: e.target.value as Direction })}
          >
            {DIRECTIONS.map((direction) => (
              <option key={direction} value={direction}>
                {DIRECTION_LABEL[direction]}
              </option>
            ))}
          </select>
        </label>
        <label className="flex flex-col gap-1">
          <span className="text-text-3">Relationship</span>
          <select
            className={inputClass}
            value={options.relationship_type ?? ""}
            onChange={(e) =>
              onChange({ ...options, relationship_type: e.target.value || undefined })
            }
          >
            <option value="">Any type</option>
            {relationshipTypes.map((type) => (
              <option key={type} value={type}>
                {type}
              </option>
            ))}
          </select>
        </label>
        <label className="col-span-2 flex flex-col gap-1">
          <span className="text-text-3">
            Relationships per expansion (1 – {EXPLORER_LIMITS.neighborMax})
          </span>
          <input
            type="number"
            min={1}
            max={EXPLORER_LIMITS.neighborMax}
            className={inputClass}
            value={options.limit ?? EXPLORER_LIMITS.neighborDefault}
            onChange={(e) => {
              const limit = Math.round(Number(e.target.value));
              if (limit >= 1 && limit <= EXPLORER_LIMITS.neighborMax)
                onChange({ ...options, limit });
            }}
          />
        </label>
      </div>
    </details>
  );
}

const ARROW = { out: ArrowRight, in: ArrowLeft, self: Repeat };

export interface EntityDetailsProps {
  node: NodeAttributes;
  groups: RelationshipGroup[];
  expanding: boolean;
  connecting: boolean;
  options: NeighborOptions;
  relationshipTypes: string[];
  onOptions(options: NeighborOptions): void;
  onExpand(): void;
  onConnections(): void;
  onFindPath(): void;
  onFocus(): void;
  /** Neighbours that collapsing would hide (shared ones stay). */
  collapsible: number;
  onCollapse(): void;
  onHide(): void;
  onClose(): void;
  onOpenEntity(entityId: string): void;
  onSelectRelationship(relationshipId: string): void;
  /** With both, the panel shows the entity's logical identity and pending
   * duplicate candidates (M9). */
  graphId?: string;
  api?: ApiClient;
}

export function EntityDetails(props: EntityDetailsProps) {
  const { node, groups } = props;
  const { cssColor } = typeStyle(node.entityType);
  const count = groups.reduce((sum, group) => sum + group.items.length, 0);
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
      <Header
        kicker={node.entityType}
        color={cssColor}
        title={node.label}
        id={node.id}
        onClose={props.onClose}
      />
      <div className="flex flex-col gap-2 border-b border-border px-4 py-3">
        <div className="grid grid-cols-2 gap-2">
          <Button
            variant="primary"
            onClick={props.onExpand}
            disabled={props.expanding}
            title="Load this entity's neighbours (E)"
            icon={
              props.expanding ? (
                <LoaderCircle size={14} className="animate-spin" />
              ) : (
                <Expand size={14} />
              )
            }
          >
            {props.expanding ? "Expanding…" : "Expand"}
          </Button>
          <Button
            onClick={props.onConnections}
            disabled={props.connecting}
            title="Load the relationships between this entity and the entities already in view"
            icon={<Link2 size={14} />}
          >
            {props.connecting ? "Loading…" : "Connections"}
          </Button>
          <Button
            onClick={props.onFindPath}
            title="Find a path from this entity (P)"
            icon={<Route size={14} />}
          >
            Find Path
          </Button>
          <div className="grid grid-cols-3 gap-2">
            <Button
              onClick={props.onFocus}
              title="Centre the view on it (F)"
              icon={<Focus size={14} />}
            >
              <span className="sr-only">Focus</span>
            </Button>
            <Button
              onClick={props.onCollapse}
              disabled={props.collapsible === 0}
              title={
                props.collapsible === 0
                  ? "No neighbours to collapse: the ones in view are shared with other entities"
                  : `Collapse: hide ${props.collapsible} neighbour${props.collapsible === 1 ? "" : "s"} connected only to it (C)`
              }
              icon={<Shrink size={14} />}
            >
              <span className="sr-only">Collapse</span>
            </Button>
            <Button
              onClick={props.onHide}
              title="Hide it in this view (H)"
              icon={<EyeOff size={14} />}
            >
              <span className="sr-only">Hide</span>
            </Button>
          </div>
        </div>
        <ExpandOptions
          options={props.options}
          relationshipTypes={props.relationshipTypes}
          onChange={props.onOptions}
        />
      </div>
      {props.api && props.graphId && (
        <IdentityCard
          api={props.api}
          graphId={props.graphId}
          entityId={node.id}
          onOpenEntity={props.onOpenEntity}
        />
      )}
      <PropertyList properties={node.properties} types={node.entity.property_types} />
      <section className="flex flex-col gap-2 px-4 py-3">
        <div className="flex items-center justify-between">
          <Kicker>Relationships in view</Kicker>
          <span className="font-mono text-[10px] text-text-3">{count}</span>
        </div>
        {groups.length === 0 && (
          <p className="text-[12px] text-text-3">None loaded yet. Expand to load its neighbours.</p>
        )}
        {groups.map((group) => {
          const Arrow = ARROW[group.direction];
          return (
            <div key={group.key} className="flex flex-col gap-1">
              <div className="flex items-center gap-1.5 font-mono text-[11px] text-text-2">
                <Arrow size={11} />
                <span>{group.relationshipType}</span>
                <span className="text-text-3">· {group.items.length}</span>
              </div>
              <ul className="flex flex-col">
                {group.items.map((item) => (
                  <li key={item.relationshipId} className="flex items-center gap-1">
                    <button
                      type="button"
                      onClick={() => props.onOpenEntity(item.entityId)}
                      className="flex min-w-0 flex-1 items-center gap-2 rounded-[4px] px-2 py-1 text-left hover:bg-raised"
                    >
                      <span
                        aria-hidden
                        className="h-2 w-2 shrink-0 rounded-full"
                        style={{ background: typeStyle(item.entityType).cssColor }}
                      />
                      <span className="truncate font-mono text-[12px]">{item.label}</span>
                      <span className="ml-auto shrink-0 text-[11px] text-text-3">
                        {item.entityType}
                      </span>
                    </button>
                    <button
                      type="button"
                      title="Inspect this relationship"
                      aria-label={`Inspect ${group.relationshipType} relationship`}
                      onClick={() => props.onSelectRelationship(item.relationshipId)}
                      className="shrink-0 rounded-[4px] p-1 text-text-3 hover:bg-raised hover:text-text"
                    >
                      <MousePointerClick size={12} />
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          );
        })}
        <p className="text-[11px] text-text-3">
          Only relationships loaded in this view; the entity may have more.
        </p>
      </section>
    </div>
  );
}

function Endpoint({ node, id, onOpen }: { node?: NodeAttributes; id: string; onOpen(): void }) {
  return (
    <button
      type="button"
      onClick={onOpen}
      className="flex w-full items-center gap-2 rounded-[4px] border border-border bg-bg px-3 py-2 text-left hover:border-border-strong"
    >
      <span
        aria-hidden
        className="h-2.5 w-2.5 shrink-0 rounded-full"
        style={{ background: node ? typeStyle(node.entityType).cssColor : "var(--text-3)" }}
      />
      <span className="flex min-w-0 flex-col">
        <span className="truncate text-[13px]">{node?.label ?? shortId(id)}</span>
        <span className="text-[11px] text-text-3">{node?.entityType ?? "Not loaded"}</span>
      </span>
    </button>
  );
}

export function RelationshipDetails({
  edge,
  source,
  target,
  onOpenEntity,
  onHide,
  onClose,
}: {
  edge: EdgeAttributes;
  source?: NodeAttributes;
  target?: NodeAttributes;
  onOpenEntity(entityId: string): void;
  onHide(): void;
  onClose(): void;
}) {
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
      <Header
        kicker="Relationship"
        color="var(--edge)"
        title={edge.relationshipType}
        id={edge.id}
        onClose={onClose}
      />
      <section className="flex flex-col gap-2 border-b border-border px-4 py-3">
        <Kicker>Source</Kicker>
        <Endpoint node={source} id={edge.source} onOpen={() => onOpenEntity(edge.source)} />
        <div className="flex items-center gap-2 pl-3 font-mono text-[11px] text-text-2">
          <ArrowRight size={12} className="rotate-90" /> {edge.relationshipType}
        </div>
        <Kicker>Target</Kicker>
        <Endpoint node={target} id={edge.target} onOpen={() => onOpenEntity(edge.target)} />
        {edge.selfLoop && (
          <p className="text-[11px] text-text-3">
            A relationship of the entity with itself; it is listed here but not drawn on the canvas.
          </p>
        )}
      </section>
      <PropertyList properties={edge.properties} types={edge.relationship.property_types} />
      <div className="px-4 py-3">
        <Button onClick={onHide} icon={<EyeOff size={14} />} title="Hide it in this view">
          Hide relationship
        </Button>
      </div>
    </div>
  );
}

/** Shown when nothing is selected. */
export function DetailsHint({ empty }: { empty: boolean }) {
  return (
    <div className="flex flex-1 flex-col gap-3 px-4 py-5 text-[12.5px] text-text-2">
      <Kicker>Details</Kicker>
      <p className="text-[14px] text-text">
        {empty ? "Nothing to inspect yet." : "Select an entity to start exploring relationships."}
      </p>
      {!empty && (
        <ul className="flex flex-col gap-1.5 text-text-3">
          <li>Click an entity or relationship to inspect it.</li>
          <li>Double-click an entity to expand its neighbours.</li>
          <li>Right-click an entity for more actions.</li>
          <li>Find entities by exact property value on the left.</li>
        </ul>
      )}
    </div>
  );
}
