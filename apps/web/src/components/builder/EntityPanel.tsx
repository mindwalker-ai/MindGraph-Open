"use client";

/**
 * "Detected Entities" (mockup 05, left): one card per entity type with the
 * file that creates it, the model's confidence and its fields (identity,
 * references to other types, plain properties).
 */

import { Dot, KeyRound, Link2, Pencil, Plus, Trash2 } from "lucide-react";

import { TypeDisc } from "@/components/builder/appearance";
import { Button } from "@/components/ui/primitives";
import {
  type BuilderDraft,
  type DraftEntityType,
  creatorsOf,
  identityFieldOf,
} from "@/lib/builder/draft";
import { cn } from "@/lib/utils";

export interface FieldLine {
  name: string;
  kind: "id" | "ref" | "property";
  /** "id", "ref" or the property type. */
  detail: string;
}

/** What a type's card lists: identity first, then references, then properties. */
export function fieldLines(draft: BuilderDraft, type: DraftEntityType): FieldLine[] {
  const creators = creatorsOf(draft, type.key);
  const identity = new Set(
    creators.flatMap((d) =>
      d.entities
        .filter((e) => e.typeKey === type.key && e.mode === "create_or_match")
        .flatMap((e) => e.fields.filter((f) => f.identity).map((f) => f.propertyKey)),
    ),
  );
  const lines: FieldLine[] = type.properties
    .filter((p) => identity.has(p.key))
    .map((p) => ({ name: p.name, kind: "id", detail: "id" }));
  // Other types this type's files refer to, e.g. customer_id in accounts.csv.
  const refs = new Set<string>();
  for (const dataset of creators) {
    for (const entity of dataset.entities) {
      if (entity.typeKey === type.key || entity.mode !== "match_only") continue;
      const field = identityFieldOf(entity);
      if (field && !refs.has(field)) {
        refs.add(field);
        lines.push({ name: field, kind: "ref", detail: "ref" });
      }
    }
  }
  for (const property of type.properties) {
    if (!identity.has(property.key)) {
      lines.push({ name: property.name, kind: "property", detail: property.type });
    }
  }
  return lines;
}

/** The file a type comes from, as shown under its name. */
export function sourceLabel(draft: BuilderDraft, type: DraftEntityType): string {
  const creators = creatorsOf(draft, type.key);
  if (creators.length > 0) {
    return creators.length === 1 ? creators[0].name : `${creators.length} files`;
  }
  if (type.existing) return "existing";
  const matched = draft.datasets.some((d) => d.entities.some((e) => e.typeKey === type.key));
  return matched ? "matched only" : "not mapped";
}

export function EntityPanel({
  draft,
  selectedKey,
  onSelect,
  onAdd,
  onRemove,
}: {
  draft: BuilderDraft;
  selectedKey: string | null;
  onSelect: (key: string) => void;
  onAdd: () => void;
  onRemove: (key: string) => void;
}) {
  const mappedColumns = new Set(
    draft.datasets.flatMap((d) =>
      [...d.entities, ...d.relationships].flatMap((m) =>
        m.fields.flatMap((f) => (f.sourceField ? [`${d.dataSourceId}:${f.sourceField}`] : [])),
      ),
    ),
  ).size;
  const files = draft.datasets.length;
  return (
    <section
      aria-label="Detected entities"
      className="flex w-[330px] shrink-0 flex-col overflow-y-auto border-r border-border bg-panel"
    >
      <header className="flex flex-col gap-1 px-4 pt-4 pb-3">
        <div className="flex items-center">
          <h2 className="flex-1 text-[15px] font-bold">Detected Entities</h2>
          <Button size="sm" icon={<Plus size={12} />} onClick={onAdd}>
            Add Entity
          </Button>
        </div>
        <span className="font-mono text-[10px] font-medium text-text-3">
          {draft.types.length} entit{draft.types.length === 1 ? "y" : "ies"}
          {files > 0 &&
            ` · ${mappedColumns} column${mappedColumns === 1 ? "" : "s"} mapped from ${files} file${files === 1 ? "" : "s"}`}
        </span>
      </header>
      <ul className="flex flex-col gap-1.5 px-[10px] pb-4" aria-label="Entities">
        {draft.types.map((type) => {
          const selected = type.key === selectedKey;
          return (
            <li
              key={type.key}
              className={cn(
                "flex flex-col rounded-[6px] border",
                selected ? "border-border-strong bg-raised" : "border-border",
              )}
            >
              <div className="flex items-center gap-2 px-[10px] py-2">
                <button
                  type="button"
                  onClick={() => onSelect(type.key)}
                  className="flex min-w-0 flex-1 items-center gap-2 text-left"
                >
                  <TypeDisc typeName={type.name} />
                  <span className="truncate text-[13.5px] font-bold">{type.name}</span>
                  <span className="truncate font-mono text-[10px] font-medium text-text-3">
                    {sourceLabel(draft, type)}
                  </span>
                </button>
                {type.confidence !== null && (
                  <span
                    title="The model's own estimate, not a guarantee"
                    className="font-mono text-[10px] font-medium text-text-2"
                  >
                    {Math.round(type.confidence * 100)}%
                  </span>
                )}
                <button
                  type="button"
                  aria-label={`Edit ${type.name}`}
                  onClick={() => onSelect(type.key)}
                  className="text-text-3 hover:text-text"
                >
                  <Pencil size={12} />
                </button>
                <button
                  type="button"
                  aria-label={`Delete ${type.name}`}
                  title={type.existing ? "Existing types cannot be removed" : undefined}
                  disabled={type.existing}
                  onClick={() => onRemove(type.key)}
                  className="text-text-3 hover:text-danger disabled:opacity-40 disabled:hover:text-text-3"
                >
                  <Trash2 size={12} />
                </button>
              </div>
              <ul className="flex flex-col pr-[10px] pb-2 pl-10">
                {fieldLines(draft, type).map((line) => (
                  <li key={`${line.kind}-${line.name}`} className="flex h-5 items-center gap-1.5">
                    {line.kind === "id" ? (
                      <KeyRound size={11} className="text-warn" />
                    ) : line.kind === "ref" ? (
                      <Link2 size={11} className="text-accent" />
                    ) : (
                      <Dot size={11} className="text-text-3" />
                    )}
                    <span
                      className={cn(
                        "flex-1 truncate font-mono text-[11px] font-medium",
                        line.kind === "property" ? "text-text-2" : "text-text",
                      )}
                    >
                      {line.name}
                    </span>
                    <span className="font-mono text-[9.5px] font-medium text-text-3">
                      {line.detail}
                    </span>
                  </li>
                ))}
              </ul>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
