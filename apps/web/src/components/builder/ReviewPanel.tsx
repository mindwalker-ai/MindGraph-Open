"use client";

import { useState } from "react";

import { Badge, Button, ConfidenceBadge, Panel, inputClass } from "@/components/ui/primitives";
import { PROPERTY_TYPES, type EntityMode, type PropertyType } from "@/lib/api/types";
import {
  type BuilderDraft,
  type DraftProperty,
  type PropertyOwner,
  addEntity,
  addProperty,
  addRelationship,
  removeEntity,
  removeProperty,
  removeRelationship,
  updateEntity,
  updateProperty,
  updateRelationship,
} from "@/lib/builder/draft";
import { cn } from "@/lib/utils";

const MODES: { value: EntityMode; label: string }[] = [
  { value: "create_or_match", label: "Create or match" },
  { value: "match_only", label: "Match existing only" },
];

type Selection = { kind: "entity" | "relationship"; key: string } | null;

interface Props {
  draft: BuilderDraft;
  fields: string[];
  onChange: (draft: BuilderDraft) => void;
}

/** Review and edit a draft: entities and relationships on the left, the
 * selected item's ontology and mapping on the right. */
export function ReviewPanel({ draft, fields, onChange }: Props) {
  const [selection, setSelection] = useState<Selection>(
    draft.entities[0] ? { kind: "entity", key: draft.entities[0].key } : null,
  );
  const typeOf = (alias: string) => draft.entities.find((e) => e.alias === alias)?.typeName ?? "?";
  const entity =
    selection?.kind === "entity" ? draft.entities.find((e) => e.key === selection.key) : undefined;
  const relationship =
    selection?.kind === "relationship"
      ? draft.relationships.find((r) => r.key === selection.key)
      : undefined;

  const add = (kind: "entity" | "relationship") => {
    const next = kind === "entity" ? addEntity(draft) : addRelationship(draft);
    const list = kind === "entity" ? next.entities : next.relationships;
    onChange(next);
    setSelection({ kind, key: list[list.length - 1].key });
  };

  return (
    <div className="grid gap-4 lg:grid-cols-[20rem_1fr]">
      <div className="flex flex-col gap-4">
        <Panel
          title="Entities"
          actions={
            <Button variant="ghost" onClick={() => add("entity")}>
              + Add entity
            </Button>
          }
        >
          <ul className="flex flex-col gap-1" aria-label="Entities">
            {draft.entities.map((e) => (
              <li key={e.key}>
                <button
                  type="button"
                  onClick={() => setSelection({ kind: "entity", key: e.key })}
                  className={cn(
                    "flex w-full items-center justify-between gap-2 rounded px-2 py-1.5 text-left text-sm hover:bg-background",
                    selection?.key === e.key && "bg-background ring-1 ring-border",
                  )}
                >
                  <span className="truncate">
                    <span className="font-medium">{e.typeName}</span>{" "}
                    <span className="font-mono text-xs text-muted">{e.alias}</span>
                  </span>
                  <span className="flex shrink-0 items-center gap-1">
                    <ConfidenceBadge value={e.confidence} />
                    {e.existing ? <Badge>Existing</Badge> : <Badge tone="accent">New</Badge>}
                  </span>
                </button>
              </li>
            ))}
            {draft.entities.length === 0 && <li className="text-sm text-muted">No entities.</li>}
          </ul>
        </Panel>
        <Panel
          title="Relationships"
          actions={
            <Button
              variant="ghost"
              disabled={draft.entities.length === 0}
              onClick={() => add("relationship")}
            >
              + Add relationship
            </Button>
          }
        >
          <ul className="flex flex-col gap-1" aria-label="Relationships">
            {draft.relationships.map((r) => (
              <li key={r.key} className="flex items-center gap-1">
                <button
                  type="button"
                  onClick={() => setSelection({ kind: "relationship", key: r.key })}
                  className={cn(
                    "flex min-w-0 flex-1 items-center justify-between gap-2 rounded px-2 py-1.5 text-left text-sm hover:bg-background",
                    selection?.key === r.key && "bg-background ring-1 ring-border",
                  )}
                >
                  <span className="truncate font-mono text-xs">
                    {typeOf(r.sourceAlias)} ─{r.typeName}→ {typeOf(r.targetAlias)}
                  </span>
                  <span className="flex shrink-0 items-center gap-1">
                    <ConfidenceBadge value={r.confidence} />
                    {r.existing && <Badge>Existing</Badge>}
                  </span>
                </button>
                <Button
                  variant="danger"
                  aria-label={`Delete relationship ${r.typeName}`}
                  onClick={() => {
                    onChange(removeRelationship(draft, r.key));
                    if (selection?.key === r.key) setSelection(null);
                  }}
                >
                  ×
                </Button>
              </li>
            ))}
            {draft.relationships.length === 0 && (
              <li className="text-sm text-muted">No relationships.</li>
            )}
          </ul>
        </Panel>
        {draft.warnings.length > 0 && (
          <Panel title="Warnings">
            <ul
              className="flex max-h-48 flex-col gap-1 overflow-y-auto text-sm"
              aria-label="Warnings"
            >
              {draft.warnings.map((warning) => (
                <li key={warning} className="text-warning">
                  ⚠ {warning}
                </li>
              ))}
            </ul>
          </Panel>
        )}
      </div>

      <div>
        {entity && (
          <Panel
            title={`Entity · ${entity.typeName}`}
            actions={
              <Button
                variant="danger"
                onClick={() => {
                  onChange(removeEntity(draft, entity.key));
                  setSelection(null);
                }}
              >
                Delete entity
              </Button>
            }
          >
            {entity.reason && <p className="mb-3 text-sm text-muted">{entity.reason}</p>}
            <div className="mb-4 grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
              <label className="text-xs text-muted">
                Entity type
                <input
                  className={inputClass}
                  value={entity.typeName}
                  onChange={(event) =>
                    onChange(updateEntity(draft, entity.key, { typeName: event.target.value }))
                  }
                />
              </label>
              <label className="text-xs text-muted">
                Display name
                <input
                  className={inputClass}
                  value={entity.displayName}
                  disabled={entity.existing}
                  onChange={(event) =>
                    onChange(updateEntity(draft, entity.key, { displayName: event.target.value }))
                  }
                />
              </label>
              <label className="text-xs text-muted">
                Alias
                <input
                  className={inputClass}
                  value={entity.alias}
                  onChange={(event) =>
                    onChange(updateEntity(draft, entity.key, { alias: event.target.value }))
                  }
                />
              </label>
              <label className="text-xs text-muted">
                Mode
                <select
                  className={inputClass}
                  value={entity.mode}
                  onChange={(event) =>
                    onChange(
                      updateEntity(draft, entity.key, { mode: event.target.value as EntityMode }),
                    )
                  }
                >
                  {MODES.map((mode) => (
                    <option key={mode.value} value={mode.value}>
                      {mode.label}
                    </option>
                  ))}
                </select>
              </label>
            </div>
            <PropertiesEditor
              owner="entity"
              ownerKey={entity.key}
              properties={entity.properties}
              fields={fields}
              withIdentity
              draft={draft}
              onChange={onChange}
            />
          </Panel>
        )}
        {relationship && (
          <Panel title={`Relationship · ${relationship.typeName}`}>
            {relationship.reason && (
              <p className="mb-3 text-sm text-muted">{relationship.reason}</p>
            )}
            <div className="mb-4 grid gap-3 sm:grid-cols-3">
              <label className="text-xs text-muted">
                Relationship type
                <input
                  className={inputClass}
                  value={relationship.typeName}
                  onChange={(event) =>
                    onChange(
                      updateRelationship(draft, relationship.key, { typeName: event.target.value }),
                    )
                  }
                />
              </label>
              {(["sourceAlias", "targetAlias"] as const).map((end) => (
                <label key={end} className="text-xs text-muted">
                  {end === "sourceAlias" ? "Source" : "Target"}
                  <select
                    className={inputClass}
                    value={relationship[end]}
                    onChange={(event) =>
                      onChange(
                        updateRelationship(draft, relationship.key, { [end]: event.target.value }),
                      )
                    }
                  >
                    {draft.entities.map((e) => (
                      <option key={e.key} value={e.alias}>
                        {e.alias} ({e.typeName})
                      </option>
                    ))}
                  </select>
                </label>
              ))}
            </div>
            <PropertiesEditor
              owner="relationship"
              ownerKey={relationship.key}
              properties={relationship.properties}
              fields={fields}
              draft={draft}
              onChange={onChange}
            />
          </Panel>
        )}
        {!entity && !relationship && (
          <Panel>
            <p className="text-sm text-muted">Select an entity or relationship to edit it.</p>
          </Panel>
        )}
      </div>
    </div>
  );
}

function PropertiesEditor({
  owner,
  ownerKey,
  properties,
  fields,
  withIdentity = false,
  draft,
  onChange,
}: {
  owner: PropertyOwner;
  ownerKey: string;
  properties: DraftProperty[];
  fields: string[];
  withIdentity?: boolean;
  draft: BuilderDraft;
  onChange: (draft: BuilderDraft) => void;
}) {
  const update = (key: string, patch: Parameters<typeof updateProperty>[4]) =>
    onChange(updateProperty(draft, owner, ownerKey, key, patch));
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-left text-sm">
        <thead className="font-mono text-[11px] uppercase tracking-wider text-muted">
          <tr className="border-b border-border">
            <th className="py-1.5 pr-2 font-normal">Property</th>
            <th className="py-1.5 pr-2 font-normal">Type</th>
            <th className="py-1.5 pr-2 font-normal">Required</th>
            {withIdentity && <th className="py-1.5 pr-2 font-normal">Identity</th>}
            <th className="py-1.5 pr-2 font-normal">Source field</th>
            <th className="py-1.5 font-normal" />
          </tr>
        </thead>
        <tbody>
          {properties.map((p) => (
            <tr key={p.key} className="border-b border-border/60">
              <td className="py-1 pr-2">
                <input
                  aria-label={`Property name ${p.name}`}
                  className={cn(inputClass, "font-mono")}
                  value={p.name}
                  disabled={p.locked}
                  onChange={(event) => update(p.key, { name: event.target.value })}
                />
              </td>
              <td className="py-1 pr-2">
                <select
                  aria-label={`Type of ${p.name}`}
                  className={inputClass}
                  value={p.type}
                  disabled={p.locked}
                  onChange={(event) => update(p.key, { type: event.target.value as PropertyType })}
                >
                  {PROPERTY_TYPES.map((type) => (
                    <option key={type} value={type}>
                      {type}
                    </option>
                  ))}
                </select>
              </td>
              <td className="py-1 pr-2">
                <input
                  type="checkbox"
                  aria-label={`${p.name} required`}
                  checked={p.required}
                  disabled={p.locked}
                  onChange={(event) => update(p.key, { required: event.target.checked })}
                />
              </td>
              {withIdentity && (
                <td className="py-1 pr-2">
                  <input
                    type="checkbox"
                    aria-label={`${p.name} identity`}
                    checked={p.identity}
                    onChange={(event) => update(p.key, { identity: event.target.checked })}
                  />
                </td>
              )}
              <td className="py-1 pr-2">
                <select
                  aria-label={`Source field of ${p.name}`}
                  className={cn(inputClass, "font-mono")}
                  value={p.sourceField ?? ""}
                  onChange={(event) => update(p.key, { sourceField: event.target.value || null })}
                >
                  <option value="">— not mapped —</option>
                  {fields.map((field) => (
                    <option key={field} value={field}>
                      {field}
                    </option>
                  ))}
                </select>
              </td>
              <td className="py-1 text-right">
                {p.locked ? (
                  <Badge>Existing</Badge>
                ) : (
                  <Button
                    variant="danger"
                    aria-label={`Delete property ${p.name}`}
                    onClick={() => onChange(removeProperty(draft, owner, ownerKey, p.key))}
                  >
                    ×
                  </Button>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <Button
        variant="ghost"
        className="mt-2"
        onClick={() => onChange(addProperty(draft, owner, ownerKey))}
      >
        + Add property
      </Button>
    </div>
  );
}
