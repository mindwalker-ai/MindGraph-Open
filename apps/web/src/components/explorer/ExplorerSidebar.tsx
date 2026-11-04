"use client";

/**
 * The left panel of the Graph Explorer: exact entity search, the entity and
 * relationship types of the active ontology (with view filters) and the
 * view controls. Counts shown here are what the browser has loaded, never
 * totals of the stored graph.
 */

import { Eye, EyeOff, LoaderCircle, Search } from "lucide-react";
import { type FormEvent, useState } from "react";

import { Badge, Button, Kicker, inputClass } from "@/components/ui/primitives";
import type { Entity, Ontology } from "@/lib/api/types";
import { typeStyle } from "@/lib/graph/palette";
import {
  DEFAULT_SEARCH_LIMIT,
  type SearchForm,
  type SearchState,
  buildSearchQuery,
  propertyChoices,
  searchSummary,
} from "@/lib/graph/search";
import type { InvestigationState } from "@/lib/graph/state";
import { entityLabel, shortId } from "@/lib/graph/transform";
import { cn } from "@/lib/utils";

const LIMITS = [DEFAULT_SEARCH_LIMIT, 100, 500];

export interface ExplorerSidebarProps {
  ontology: Ontology;
  search: SearchState;
  state: InvestigationState;
  /** Loaded entities per type (client-side). */
  loadedByType: ReadonlyMap<string, number>;
  /** Set while the path finder waits for an endpoint ("Source" or "Target"). */
  pickLabel: string | null;
  hidden: { entities: number; relationships: number };
  loaded: number;
  onSearch(form: SearchForm): void;
  onOpenEntity(entity: Entity): void;
  onPick(entity: Entity): void;
  onToggleEntityType(name: string): void;
  onToggleRelationshipType(name: string): void;
  onShowHidden(): void;
  onRemoveHidden(): void;
  onResetView(): void;
}

function SectionTitle({ title, aside }: { title: string; aside?: string }) {
  return (
    <div className="flex items-center justify-between px-4 pb-2">
      <Kicker>{title}</Kicker>
      {aside && <span className="font-mono text-[10px] text-text-3">{aside}</span>}
    </div>
  );
}

function ToggleEye({
  shown,
  label,
  onToggle,
}: {
  shown: boolean;
  label: string;
  onToggle(): void;
}) {
  return (
    <button
      type="button"
      aria-pressed={shown}
      aria-label={`${shown ? "Hide" : "Show"} ${label} in the view`}
      title={`${shown ? "Hide" : "Show"} ${label} in the view`}
      onClick={onToggle}
      className="rounded-[4px] p-1 text-text-3 hover:bg-raised hover:text-text"
    >
      {shown ? <Eye size={13} /> : <EyeOff size={13} />}
    </button>
  );
}

function EntitySearch({
  ontology,
  search,
  pickLabel,
  form,
  onForm,
  onSearch,
  onOpenEntity,
  onPick,
}: {
  ontology: Ontology;
  search: SearchState;
  pickLabel: string | null;
  form: SearchForm;
  onForm(form: SearchForm): void;
  onSearch(form: SearchForm): void;
  onOpenEntity(entity: Entity): void;
  onPick(entity: Entity): void;
}) {
  const [error, setError] = useState<string | null>(null);
  const choices = propertyChoices(ontology, form.entityType);
  const property = choices.find((choice) => choice.key === form.property);

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const built = buildSearchQuery(form, ontology);
    setError(built.ok ? null : built.error);
    if (built.ok) onSearch(form);
  };

  const results = search.status === "done" ? search.results : [];
  return (
    <section className="flex flex-col border-b border-border pt-4 pb-3">
      <SectionTitle title="Find entities" aside="exact match" />
      <form onSubmit={submit} className="flex flex-col gap-2 px-4" aria-label="Entity search">
        <select
          aria-label="Entity type"
          className={inputClass}
          value={form.entityType}
          onChange={(e) => onForm({ ...form, entityType: e.target.value, property: "" })}
        >
          <option value="">Any entity type</option>
          {ontology.entity_types.map((type) => (
            <option key={type.id} value={type.name}>
              {type.name}
            </option>
          ))}
        </select>
        <select
          aria-label="Property"
          className={inputClass}
          value={form.property}
          onChange={(e) => onForm({ ...form, property: e.target.value })}
        >
          <option value="">No property filter</option>
          {choices.map((choice) => (
            <option key={choice.key} value={choice.key}>
              {choice.key} · {choice.ambiguous ? "several types" : choice.type}
            </option>
          ))}
        </select>
        <input
          aria-label="Value"
          className={inputClass}
          placeholder={
            form.property ? `exact ${property?.type ?? ""} value` : "choose a property first"
          }
          disabled={!form.property}
          value={form.value}
          onChange={(e) => onForm({ ...form, value: e.target.value })}
        />
        <div className="flex gap-2">
          <select
            aria-label="Result limit"
            className={cn(inputClass, "w-[84px]")}
            value={form.limit}
            onChange={(e) => onForm({ ...form, limit: Number(e.target.value) })}
          >
            {LIMITS.map((limit) => (
              <option key={limit} value={limit}>
                {limit}
              </option>
            ))}
          </select>
          <Button
            type="submit"
            variant="primary"
            className="flex-1"
            disabled={search.status === "loading"}
            icon={
              search.status === "loading" ? (
                <LoaderCircle size={13} className="animate-spin" />
              ) : (
                <Search size={13} />
              )
            }
          >
            Search
          </Button>
        </div>
        {error && <p className="text-[11.5px] text-danger">{error}</p>}
        <p
          role="status"
          className={cn("text-[11.5px]", search.status === "error" ? "text-danger" : "text-text-3")}
        >
          {searchSummary(search)}
        </p>
      </form>
      {results.length > 0 && (
        <ul
          aria-label="Search results"
          className="mt-1 flex max-h-[260px] flex-col overflow-y-auto px-2"
        >
          {results.map((entity) => (
            <li key={entity.id} className="group flex items-center">
              <button
                type="button"
                onClick={() => onOpenEntity(entity)}
                className="flex min-w-0 flex-1 items-center gap-2 rounded-[4px] px-2 py-1.5 text-left hover:bg-raised"
              >
                <span
                  aria-hidden
                  className="h-2 w-2 shrink-0 rounded-full"
                  style={{ background: typeStyle(entity.entity_type).cssColor }}
                />
                <span className="flex min-w-0 flex-col">
                  <span className="truncate text-[13px]">{entityLabel(entity)}</span>
                  <span className="truncate font-mono text-[10px] text-text-3">
                    {entity.entity_type} · {shortId(entity.id)}
                  </span>
                </span>
              </button>
              {pickLabel && (
                <button
                  type="button"
                  onClick={() => onPick(entity)}
                  className="shrink-0 rounded-[4px] border border-accent/50 px-1.5 py-0.5 font-mono text-[10px] text-accent hover:bg-accent-soft"
                >
                  {pickLabel}
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

export function ExplorerSidebar(props: ExplorerSidebarProps) {
  const { ontology, state } = props;
  const [form, setForm] = useState<SearchForm>({
    entityType: "",
    property: "",
    value: "",
    limit: DEFAULT_SEARCH_LIMIT,
  });
  const hiddenTotal = props.hidden.entities + props.hidden.relationships;

  return (
    <aside className="flex w-[260px] shrink-0 flex-col overflow-y-auto border-r border-border bg-panel">
      <EntitySearch
        ontology={ontology}
        search={props.search}
        pickLabel={props.pickLabel}
        form={form}
        onForm={setForm}
        onSearch={props.onSearch}
        onOpenEntity={props.onOpenEntity}
        onPick={props.onPick}
      />

      <section className="flex flex-col border-b border-border pt-4 pb-3">
        <SectionTitle title="Entity types" aside={`${ontology.entity_types.length} types`} />
        <ul className="flex flex-col px-2">
          {ontology.entity_types.map((type) => {
            const shown = !state.entityTypeFilters.has(type.name);
            const loaded = props.loadedByType.get(type.name) ?? 0;
            return (
              <li key={type.id} className="flex items-center gap-1">
                <button
                  type="button"
                  title={`List ${type.name} entities`}
                  onClick={() => {
                    const next = { ...form, entityType: type.name, property: "", value: "" };
                    setForm(next);
                    props.onSearch(next);
                  }}
                  className={cn(
                    "flex min-w-0 flex-1 items-center gap-2 rounded-[4px] px-2 py-1.5 text-left hover:bg-raised",
                    !shown && "opacity-50",
                  )}
                >
                  <span
                    aria-hidden
                    className="h-2.5 w-2.5 shrink-0 rounded-full"
                    style={{ boxShadow: `inset 0 0 0 2px ${typeStyle(type.name).cssColor}` }}
                  />
                  <span className="truncate text-[13px]">{type.name}</span>
                  {loaded > 0 && (
                    <span
                      className="ml-auto font-mono text-[10px] text-text-3"
                      title="Loaded in this view (not the total in the graph)"
                    >
                      {loaded} in view
                    </span>
                  )}
                </button>
                <ToggleEye
                  shown={shown}
                  label={type.name}
                  onToggle={() => props.onToggleEntityType(type.name)}
                />
              </li>
            );
          })}
        </ul>
      </section>

      <section className="flex flex-col border-b border-border pt-4 pb-3">
        <SectionTitle
          title="Relationship types"
          aside={`${ontology.relationship_types.length} types`}
        />
        {ontology.relationship_types.length === 0 && (
          <p className="px-4 text-[12px] text-text-3">The ontology defines none.</p>
        )}
        <ul className="flex flex-col px-2">
          {ontology.relationship_types.map((type) => {
            const shown = !state.relationshipTypeFilters.has(type.name);
            return (
              <li key={type.id} className="flex items-center gap-1">
                <div
                  className={cn("flex min-w-0 flex-1 flex-col px-2 py-1", !shown && "opacity-50")}
                >
                  <span className="font-mono text-[11.5px] font-medium">{type.name}</span>
                  <span className="truncate text-[11px] text-text-3">
                    {type.source_entity_type} → {type.target_entity_type}
                  </span>
                </div>
                <ToggleEye
                  shown={shown}
                  label={type.name}
                  onToggle={() => props.onToggleRelationshipType(type.name)}
                />
              </li>
            );
          })}
        </ul>
      </section>

      <section className="flex flex-col gap-2 px-4 pt-4 pb-4">
        <Kicker>View</Kicker>
        {hiddenTotal > 0 ? (
          <>
            <p className="text-[12px] text-text-2">
              <Badge>{props.hidden.entities} entities</Badge>{" "}
              <Badge>{props.hidden.relationships} relationships</Badge> hidden
            </p>
            <div className="flex gap-2">
              <Button size="sm" onClick={props.onShowHidden}>
                Show all
              </Button>
              <Button
                size="sm"
                onClick={props.onRemoveHidden}
                title="Unload hidden items from the browser to make room (the graph is unchanged)"
              >
                Unload hidden
              </Button>
            </div>
          </>
        ) : (
          <p className="text-[12px] text-text-3">Nothing hidden.</p>
        )}
        <Button
          size="sm"
          variant="ghost"
          className="self-start"
          onClick={props.onResetView}
          disabled={props.loaded === 0}
          title="Clear the canvas; the stored graph is not changed"
        >
          Reset view
        </Button>
      </section>
    </aside>
  );
}
