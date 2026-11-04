"use client";

/**
 * The right-hand inspector of the Graph Builder (mockup 05): the model
 * overview when nothing is selected, or the definition and mappings of the
 * selected entity type or relationship type.
 */

import {
  ArrowLeftRight,
  Check,
  KeyRound,
  Pencil,
  Plus,
  Trash2,
  TriangleAlert,
  X,
} from "lucide-react";
import type { ReactNode } from "react";

import { TypeDisc } from "@/components/builder/appearance";
import type { Selection } from "@/components/builder/OntologyCanvas";
import { Button, Kicker, Meter, inputClass } from "@/components/ui/primitives";
import { PROPERTY_TYPES, type GraphBuilderResult, type PropertyType } from "@/lib/api/types";
import {
  type BuilderDraft,
  type DatasetDraft,
  type DraftEntityMapping,
  type DraftProperty,
  type DraftRelationshipMapping,
  type PropertyOwner,
  addEntityMapping,
  addProperty,
  addRelationshipMapping,
  fieldOf,
  relationshipMappingsOf,
  relationshipTypeByKey,
  removeEntityMapping,
  removeProperty,
  removeRelationshipType,
  removeType,
  renameRelationshipType,
  reverseRelationshipType,
  sampleMatches,
  setEntityField,
  setRelationshipField,
  typeByKey,
  updateEntityMapping,
  updateProperty,
  updateType,
} from "@/lib/builder/draft";
import { cn } from "@/lib/utils";

interface Props {
  draft: BuilderDraft;
  selection: Selection;
  onSelect: (selection: Selection) => void;
  onChange: (draft: BuilderDraft) => void;
  errors: string[];
  result?: GraphBuilderResult;
}

function Section({
  title,
  children,
  className,
}: {
  title: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section
      className={cn("flex flex-col gap-2 border-b border-border px-[18px] py-[14px]", className)}
    >
      <Kicker>{title}</Kicker>
      {children}
    </section>
  );
}

function Confidence({ value, evidence }: { value: number | null; evidence: string }) {
  if (value === null) {
    return evidence ? (
      <p className="font-mono text-[10px] font-medium text-text-2">{evidence}</p>
    ) : null;
  }
  return (
    <div className="flex w-full flex-col gap-1.5">
      <div className="flex items-end">
        <span
          className="flex-1 text-[12.5px] text-text-2"
          title="The model's own estimate, not a guarantee"
        >
          Confidence
        </span>
        <span className="text-[22px] leading-[22px] font-bold">{Math.round(value * 100)}%</span>
      </div>
      <Meter value={value} label="Confidence" />
      {evidence && <p className="font-mono text-[10px] font-medium text-text-2">{evidence}</p>}
    </div>
  );
}

function FieldSelect({
  label,
  fields,
  value,
  onChange,
  allowNone = true,
  className,
}: {
  label: string;
  fields: string[];
  value: string | null;
  onChange: (value: string | null) => void;
  allowNone?: boolean;
  className?: string;
}) {
  return (
    <select
      aria-label={label}
      className={cn(inputClass, className)}
      value={value ?? ""}
      onChange={(event) => onChange(event.target.value || null)}
    >
      {allowNone && <option value="">— not mapped —</option>}
      {fields.map((field) => (
        <option key={field} value={field}>
          {field}
        </option>
      ))}
    </select>
  );
}

// --- overview -------------------------------------------------------------------------

function Overview({
  draft,
  errors,
  result,
}: {
  draft: BuilderDraft;
  errors: string[];
  result?: GraphBuilderResult;
}) {
  return (
    <>
      <header className="flex flex-col gap-3 border-b border-border px-[18px] py-4">
        <Kicker>MODEL</Kicker>
        <span className="text-[14px] font-bold">
          {draft.origin === "ai" ? "AI suggestion" : "Manual model"}
        </span>
        {draft.origin === "ai" && (
          <Confidence
            value={draft.confidence}
            evidence={
              result
                ? `${result.model} · ${result.prompt_version}${result.repair_attempts ? ` · ${result.repair_attempts} repair` : ""}`
                : ""
            }
          />
        )}
        <p className="text-[12px] text-text-3">
          Select an entity or a relationship to edit it. Nothing is written until you click Generate
          Graph.
        </p>
      </header>
      {draft.warnings.length > 0 && (
        <Section title="WARNINGS">
          <ul className="flex flex-col gap-1.5">
            {draft.warnings.map((warning) => (
              <li key={warning} className="flex gap-2 text-[12px] text-warn">
                <TriangleAlert size={13} className="mt-0.5 shrink-0" /> {warning}
              </li>
            ))}
          </ul>
        </Section>
      )}
      <Section title={errors.length ? `PROBLEMS (${errors.length})` : "PROBLEMS"}>
        {errors.length === 0 ? (
          <p className="flex items-center gap-2 text-[12px] text-ok">
            <Check size={13} /> Ready to generate.
          </p>
        ) : (
          <ul aria-label="Problems" className="flex flex-col gap-1.5">
            {errors.map((error) => (
              <li key={error} className="text-[12px] text-danger">
                {error}
              </li>
            ))}
          </ul>
        )}
      </Section>
    </>
  );
}

// --- properties -------------------------------------------------------------------------

function PropertiesEditor({
  draft,
  owner,
  ownerKey,
  properties,
  onChange,
}: {
  draft: BuilderDraft;
  owner: PropertyOwner;
  ownerKey: string;
  properties: DraftProperty[];
  onChange: (draft: BuilderDraft) => void;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      {properties.map((property) => (
        <div key={property.key} className="flex items-center gap-1.5">
          <input
            aria-label={`Property name ${property.name}`}
            className={cn(inputClass, "flex-1")}
            value={property.name}
            disabled={property.locked}
            onChange={(event) =>
              onChange(
                updateProperty(draft, owner, ownerKey, property.key, { name: event.target.value }),
              )
            }
          />
          <select
            aria-label={`Type of ${property.name}`}
            className={cn(inputClass, "w-[92px]")}
            value={property.type}
            disabled={property.locked}
            onChange={(event) =>
              onChange(
                updateProperty(draft, owner, ownerKey, property.key, {
                  type: event.target.value as PropertyType,
                }),
              )
            }
          >
            {PROPERTY_TYPES.map((type) => (
              <option key={type} value={type}>
                {type}
              </option>
            ))}
          </select>
          <label className="flex items-center gap-1 text-[11px] text-text-3" title="Required">
            <input
              type="checkbox"
              aria-label={`${property.name} required`}
              checked={property.required}
              disabled={property.locked}
              onChange={(event) =>
                onChange(
                  updateProperty(draft, owner, ownerKey, property.key, {
                    required: event.target.checked,
                  }),
                )
              }
            />
            req
          </label>
          <button
            type="button"
            aria-label={`Remove property ${property.name}`}
            disabled={property.locked}
            title={property.locked ? "Existing properties are kept" : undefined}
            onClick={() => onChange(removeProperty(draft, owner, ownerKey, property.key))}
            className="text-text-3 hover:text-danger disabled:opacity-30"
          >
            <X size={13} />
          </button>
        </div>
      ))}
      <Button
        variant="ghost"
        icon={<Plus size={12} />}
        onClick={() => onChange(addProperty(draft, owner, ownerKey))}
        className="self-start"
      >
        Add property
      </Button>
    </div>
  );
}

// --- entity ------------------------------------------------------------------------------

function EntityMappingCard({
  draft,
  dataset,
  entity,
  onChange,
}: {
  draft: BuilderDraft;
  dataset: DatasetDraft;
  entity: DraftEntityMapping;
  onChange: (draft: BuilderDraft) => void;
}) {
  const type = typeByKey(draft, entity.typeKey);
  const fields = dataset.profile.fields.map((f) => f.name);
  const id = dataset.dataSourceId;
  return (
    <div className="flex flex-col gap-2 rounded-[6px] border border-border bg-bg p-[10px]">
      <div className="flex items-center gap-2">
        <span className="flex-1 truncate font-mono text-[11px] font-medium">{dataset.name}</span>
        <button
          type="button"
          aria-label={`Remove mapping in ${dataset.name}`}
          onClick={() => onChange(removeEntityMapping(draft, id, entity.key))}
          className="text-text-3 hover:text-danger"
        >
          <Trash2 size={12} />
        </button>
      </div>
      <div className="flex flex-col gap-1.5">
        <select
          aria-label={`Mode in ${dataset.name}`}
          className={inputClass}
          value={entity.mode}
          onChange={(event) =>
            onChange(
              updateEntityMapping(draft, id, entity.key, {
                mode: event.target.value as DraftEntityMapping["mode"],
              }),
            )
          }
        >
          <option value="create_or_match">Create or match</option>
          <option value="match_only">Match only</option>
        </select>
        <input
          aria-label={`Alias in ${dataset.name}`}
          className={inputClass}
          value={entity.alias}
          onChange={(event) =>
            onChange(updateEntityMapping(draft, id, entity.key, { alias: event.target.value }))
          }
        />
      </div>
      {type?.properties.map((property) => {
        const field = fieldOf(entity.fields, property.key);
        const identity = Boolean(field?.identity);
        const relevant = entity.mode === "create_or_match" || identity;
        return (
          <div
            key={property.key}
            className={cn("flex items-center gap-1.5", !relevant && "opacity-50")}
          >
            <button
              type="button"
              aria-label={`${property.name} identifies ${type.name} in ${dataset.name}`}
              aria-pressed={identity}
              title={identity ? "Identity property" : "Use as identity"}
              onClick={() =>
                onChange(
                  setEntityField(draft, id, entity.key, property.key, { identity: !identity }),
                )
              }
              className={identity ? "text-warn" : "text-text-3 hover:text-text-2"}
            >
              <KeyRound size={12} />
            </button>
            <span
              title={property.name}
              className="min-w-0 flex-1 truncate font-mono text-[11px] font-medium text-text-2"
            >
              {property.name}
            </span>
            <FieldSelect
              className="w-[140px] shrink-0"
              label={`Source field of ${property.name} in ${dataset.name}`}
              fields={fields}
              value={field?.sourceField ?? null}
              onChange={(value) =>
                onChange(
                  setEntityField(draft, id, entity.key, property.key, { sourceField: value }),
                )
              }
            />
          </div>
        );
      })}
      {entity.mode === "match_only" && (
        <p className="text-[11px] text-text-3">
          Match only: finds existing entities by identity and sets no properties.
        </p>
      )}
    </div>
  );
}

function EntityInspector({ draft, typeKey, onSelect, onChange }: Props & { typeKey: string }) {
  const type = typeByKey(draft, typeKey);
  if (!type) return null;
  const mapped = draft.datasets.flatMap((dataset) =>
    dataset.entities.filter((e) => e.typeKey === typeKey).map((entity) => ({ dataset, entity })),
  );
  const unmapped = draft.datasets.filter((d) => !d.entities.some((e) => e.typeKey === typeKey));
  return (
    <>
      <header className="flex flex-col gap-3 border-b border-border px-[18px] py-4">
        <div className="flex items-center">
          <Kicker className="flex-1">ENTITY</Kicker>
          <button
            type="button"
            aria-label="Close"
            onClick={() => onSelect(null)}
            className="text-text-3 hover:text-text"
          >
            <X size={14} />
          </button>
        </div>
        <div className="flex items-center gap-2">
          <TypeDisc typeName={type.name} size={20} />
          <span className="text-[14px] font-bold">{type.name}</span>
          {type.existing && (
            <span className="rounded-[10px] border border-[var(--t-ip)] px-2 py-0.5 font-mono text-[10px] text-[var(--t-ip)]">
              existing
            </span>
          )}
        </div>
        <Confidence value={type.confidence} evidence={type.reason} />
      </header>
      <Section title="DEFINITION">
        <label className="flex flex-col gap-1">
          <span className="text-[12px] text-text-3">Entity type</span>
          <input
            aria-label="Entity type name"
            className={inputClass}
            value={type.name}
            disabled={type.existing}
            title={type.existing ? "Existing types keep their name" : undefined}
            onChange={(event) =>
              onChange(updateType(draft, type.key, { name: event.target.value }))
            }
          />
        </label>
        <label className="flex flex-col gap-1">
          <span className="text-[12px] text-text-3">Display name</span>
          <input
            aria-label="Display name"
            className={inputClass}
            value={type.displayName}
            onChange={(event) =>
              onChange(updateType(draft, type.key, { displayName: event.target.value }))
            }
          />
        </label>
      </Section>
      <Section title="PROPERTIES">
        <PropertiesEditor
          draft={draft}
          owner="entity"
          ownerKey={type.key}
          properties={type.properties}
          onChange={onChange}
        />
      </Section>
      <Section title="SOURCE FIELDS">
        {mapped.map(({ dataset, entity }) => (
          <EntityMappingCard
            key={entity.key}
            draft={draft}
            dataset={dataset}
            entity={entity}
            onChange={onChange}
          />
        ))}
        {mapped.length === 0 && <p className="text-[12px] text-text-3">Not mapped in any file.</p>}
        {unmapped.length > 0 && (
          <select
            aria-label="Map in file"
            className={inputClass}
            value=""
            onChange={(event) =>
              event.target.value && onChange(addEntityMapping(draft, event.target.value, type.key))
            }
          >
            <option value="">+ Map in another file…</option>
            {unmapped.map((d) => (
              <option key={d.dataSourceId} value={d.dataSourceId}>
                {d.name}
              </option>
            ))}
          </select>
        )}
      </Section>
      <div className="px-[18px] py-[14px]">
        <Button
          variant="danger"
          icon={<Trash2 size={13} />}
          disabled={type.existing}
          title={type.existing ? "Existing types cannot be removed" : undefined}
          onClick={() => {
            onChange(removeType(draft, type.key));
            onSelect(null);
          }}
        >
          Remove entity
        </Button>
      </div>
    </>
  );
}

// --- relationship -----------------------------------------------------------------------

function RelationshipInspector({
  draft,
  typeKey,
  onSelect,
  onChange,
}: Props & { typeKey: string }) {
  const type = relationshipTypeByKey(draft, typeKey);
  if (!type) return null;
  const source = typeByKey(draft, type.sourceTypeKey);
  const target = typeByKey(draft, type.targetTypeKey);
  const mappings = relationshipMappingsOf(draft, typeKey);
  const primary = mappings[0];
  const hosts = draft.datasets.filter(
    (d) =>
      !d.relationships.some((m) => m.typeKey === typeKey) &&
      d.entities.some((e) => e.typeKey === type.sourceTypeKey) &&
      d.entities.some((e) => e.typeKey === type.targetTypeKey),
  );

  const keyField = (
    dataset: DatasetDraft,
    mapping: DraftRelationshipMapping,
    end: "sourceKey" | "targetKey",
  ) => {
    const entity = dataset.entities.find((e) => e.key === mapping[end]);
    const identity = entity?.fields.find((f) => f.identity);
    return { entity, identity };
  };
  const setKey = (
    dataset: DatasetDraft,
    mapping: DraftRelationshipMapping,
    end: "sourceKey" | "targetKey",
    value: string | null,
  ) => {
    const { entity, identity } = keyField(dataset, mapping, end);
    if (!entity || !identity) return;
    onChange(
      setEntityField(draft, dataset.dataSourceId, entity.key, identity.propertyKey, {
        sourceField: value,
      }),
    );
  };
  const samples = primary ? sampleMatches(primary.dataset, primary.mapping) : null;

  return (
    <>
      <header className="flex flex-col gap-3 border-b border-border px-[18px] py-4">
        <div className="flex items-center">
          <Kicker className="flex-1">RELATIONSHIP</Kicker>
          <button
            type="button"
            aria-label="Close"
            onClick={() => onSelect(null)}
            className="text-text-3 hover:text-text"
          >
            <X size={14} />
          </button>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <span className="flex items-center gap-1.5">
            <TypeDisc typeName={source?.name ?? "?"} size={20} />
            <span className="text-[14px] font-bold">{source?.name ?? "?"}</span>
          </span>
          <span className="rounded-[10px] border border-accent bg-accent-deep px-2 py-0.5 font-mono text-[10px] font-medium text-accent">
            {type.name} →
          </span>
          <span className="flex items-center gap-1.5">
            <TypeDisc typeName={target?.name ?? "?"} size={20} />
            <span className="text-[14px] font-bold">{target?.name ?? "?"}</span>
          </span>
        </div>
        <Confidence value={type.confidence} evidence={type.reason} />
      </header>

      <Section title="DEFINITION">
        <label className="flex flex-col gap-1">
          <span className="text-[12px] text-text-3">Relationship name</span>
          <span className="relative">
            <input
              aria-label="Relationship name"
              className={cn(inputClass, "pr-8")}
              value={type.name}
              disabled={type.existing}
              title={type.existing ? "Existing relationships keep their name" : undefined}
              onChange={(event) =>
                onChange(renameRelationshipType(draft, type.key, event.target.value))
              }
            />
            <Pencil
              size={13}
              className="pointer-events-none absolute top-[9px] right-[10px] text-text-3"
            />
          </span>
        </label>
        {primary ? (
          (["sourceKey", "targetKey"] as const).map((end) => {
            const { identity } = keyField(primary.dataset, primary.mapping, end);
            return (
              <label key={end} className="flex flex-col gap-1">
                <span className="text-[12px] text-text-3">
                  {end === "sourceKey" ? "Source key" : "Target key"}
                </span>
                <select
                  aria-label={end === "sourceKey" ? "Source key" : "Target key"}
                  className={inputClass}
                  value={identity?.sourceField ?? ""}
                  disabled={!identity}
                  onChange={(event) =>
                    setKey(primary.dataset, primary.mapping, end, event.target.value || null)
                  }
                >
                  <option value="">— choose a field —</option>
                  {primary.dataset.profile.fields.map((f) => (
                    <option key={f.name} value={f.name}>
                      {primary.dataset.name}.{f.name}
                    </option>
                  ))}
                </select>
              </label>
            );
          })
        ) : (
          <p className="text-[12px] text-text-3">Not mapped in any file yet.</p>
        )}
        {mappings.length > 1 && (
          <p className="font-mono text-[10px] text-text-3">
            Also mapped in{" "}
            {mappings
              .slice(1)
              .map((m) => m.dataset.name)
              .join(", ")}
          </p>
        )}
        {hosts.length > 0 && (
          <select
            aria-label="Map relationship in file"
            className={inputClass}
            value=""
            onChange={(event) =>
              event.target.value &&
              onChange(addRelationshipMapping(draft, event.target.value, type.key))
            }
          >
            <option value="">+ Map in a file…</option>
            {hosts.map((d) => (
              <option key={d.dataSourceId} value={d.dataSourceId}>
                {d.name}
              </option>
            ))}
          </select>
        )}
        <div className="flex items-center pt-0.5" title="Computed from the generated graph (M6)">
          <span className="flex-1 text-[12px] text-text-3">Cardinality</span>
          <span className="font-mono text-[10.5px] font-medium text-text-3">—</span>
        </div>
      </Section>

      {type.properties.length > 0 && primary && (
        <Section title="PROPERTIES">
          {type.properties.map((property) => (
            <div key={property.key} className="flex items-center gap-1.5">
              <span className="w-[96px] truncate font-mono text-[11px] text-text-2">
                {property.name}
              </span>
              <FieldSelect
                label={`Source field of ${property.name}`}
                fields={primary.dataset.profile.fields.map((f) => f.name)}
                value={fieldOf(primary.mapping.fields, property.key)?.sourceField ?? null}
                onChange={(value) =>
                  onChange(
                    setRelationshipField(
                      draft,
                      primary.dataset.dataSourceId,
                      primary.mapping.key,
                      property.key,
                      value,
                    ),
                  )
                }
              />
            </div>
          ))}
        </Section>
      )}

      <Section title="SAMPLE MATCHES">
        {samples && samples.rows.length > 0 ? (
          <table
            aria-label="Sample matches"
            className="w-full overflow-hidden rounded-[4px] border border-border font-mono text-[10.5px] font-medium"
          >
            <thead>
              <tr className="h-7 bg-raised text-left text-text-3">
                <th className="px-[10px] font-medium">{samples.source}</th>
                <th className="px-[10px] font-medium">{samples.target}</th>
              </tr>
            </thead>
            <tbody>
              {samples.rows.map(([a, b], index) => (
                <tr key={index} className="h-7 border-t border-border">
                  <td className="truncate px-[10px]">{a}</td>
                  <td className="truncate px-[10px]">{b}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <p className="text-[12px] text-text-3">No sample rows link these two entities yet.</p>
        )}
      </Section>

      <div className="flex flex-col gap-[10px] px-[18px] py-[14px]">
        <div className="flex gap-2">
          <Button
            className="flex-1"
            icon={<ArrowLeftRight size={14} />}
            disabled={type.existing}
            title={type.existing ? "Existing relationships keep their direction" : undefined}
            onClick={() => onChange(reverseRelationshipType(draft, type.key))}
          >
            Reverse
          </Button>
          <Button className="flex-1" icon={<Check size={14} />} onClick={() => onSelect(null)}>
            Apply
          </Button>
        </div>
        <Button
          variant="danger"
          icon={<Trash2 size={13} />}
          disabled={type.existing}
          title={type.existing ? "Existing relationships cannot be removed" : undefined}
          onClick={() => {
            onChange(removeRelationshipType(draft, type.key));
            onSelect(null);
          }}
          className="self-start"
        >
          Remove relationship
        </Button>
      </div>
    </>
  );
}

export function Inspector(props: Props) {
  const { selection } = props;
  return (
    <aside
      aria-label="Inspector"
      className="flex w-[330px] shrink-0 flex-col overflow-y-auto border-l border-border bg-panel"
    >
      {selection?.kind === "entity" ? (
        <EntityInspector {...props} typeKey={selection.key} />
      ) : selection?.kind === "relationship" ? (
        <RelationshipInspector {...props} typeKey={selection.key} />
      ) : (
        <Overview draft={props.draft} errors={props.errors} result={props.result} />
      )}
    </aside>
  );
}
