/**
 * The Graph Builder review draft: editable client state between an AI (or
 * manual) suggestion and "Generate Graph".
 *
 * A draft has two layers, like the requests it turns into:
 *
 * - the ontology: entity types and relationship types with their properties
 *   (one `POST /graphs/{id}/ontology`);
 * - one mapping per dataset: which entities and relationships each record
 *   of that dataset produces, and from which fields (one
 *   `POST .../data-sources/{id}/ingestions` per dataset, in a safe order).
 *
 * Mappings point at types and at each other by stable keys, never by name,
 * so renaming a type or an alias can never move another one's mappings. The
 * server validates everything again; this module only keeps the UI from
 * submitting obviously invalid drafts.
 *
 * Every function is pure: it returns a new draft.
 */

import type {
  DatasetProfile,
  EntityMode,
  GraphBuilderSuggestion,
  IngestionMapping,
  Ontology,
  OntologyDraft,
  PropertyDraft,
  PropertyType,
  TypeAnnotation,
} from "@/lib/api/types";

/** Machine names of types and properties (the server's rule). */
export const MACHINE_NAME = /^[A-Za-z][A-Za-z0-9_]{0,63}$/;

export interface DraftProperty {
  key: string;
  name: string;
  type: PropertyType;
  required: boolean;
  /** Part of the base ontology: name, type and requiredness are fixed. */
  locked: boolean;
}

export interface DraftEntityType {
  key: string;
  name: string;
  displayName: string;
  /** In the base ontology: cannot be renamed or removed. */
  existing: boolean;
  confidence: number | null;
  reason: string;
  properties: DraftProperty[];
}

export interface DraftRelationshipType {
  key: string;
  name: string;
  sourceTypeKey: string;
  targetTypeKey: string;
  existing: boolean;
  confidence: number | null;
  reason: string;
  properties: DraftProperty[];
}

/** Where one property of a mapped entity or relationship comes from. */
export interface DraftFieldMap {
  propertyKey: string;
  sourceField: string | null;
  /** Entities only: the property identifies the entity. */
  identity: boolean;
}

/** One entity each record of a dataset produces (or matches). */
export interface DraftEntityMapping {
  key: string;
  alias: string;
  typeKey: string;
  mode: EntityMode;
  fields: DraftFieldMap[];
}

/** One relationship each record of a dataset produces. */
export interface DraftRelationshipMapping {
  key: string;
  typeKey: string;
  /** Key of the source entity mapping (same dataset). */
  sourceKey: string;
  /** Key of the target entity mapping (same dataset). */
  targetKey: string;
  fields: DraftFieldMap[];
}

/** An analysed dataset the draft maps. */
export interface DatasetInfo {
  dataSourceId: string;
  /** A label unique in the draft, usually the filename. */
  name: string;
  profile: DatasetProfile;
}

export interface DatasetDraft extends DatasetInfo {
  entities: DraftEntityMapping[];
  relationships: DraftRelationshipMapping[];
}

export interface BuilderDraft {
  /** The pinned active ontology the draft builds on (kept intact). */
  base: OntologyDraft;
  types: DraftEntityType[];
  relationshipTypes: DraftRelationshipType[];
  datasets: DatasetDraft[];
  warnings: string[];
  confidence: number | null;
  origin: "ai" | "manual";
}

/** What one dataset's ingestion submits. */
export interface IngestionPlan {
  dataSourceId: string;
  name: string;
  mapping: IngestionMapping;
}

let counter = 0;
function nextKey(prefix: string): string {
  counter += 1;
  return `${prefix}-${counter}`;
}

// --- conversions --------------------------------------------------------------

/** The active ontology in the draft (request) shape. */
export function ontologyToDraft(ontology: Ontology): OntologyDraft {
  const properties = (list: Ontology["entity_types"][number]["properties"]): PropertyDraft[] =>
    list.map((p) => ({ name: p.key, type: p.type, required: p.required }));
  return {
    entity_types: ontology.entity_types.map((t) => ({
      name: t.name,
      display_name: t.display_name,
      description: t.description,
      properties: properties(t.properties),
    })),
    relationship_types: ontology.relationship_types.map((t) => ({
      name: t.name,
      source_entity_type: t.source_entity_type,
      target_entity_type: t.target_entity_type,
      properties: properties(t.properties),
    })),
  };
}

function baseEntityType(base: OntologyDraft, name: string) {
  return base.entity_types.find((t) => t.name === name);
}

function baseRelationshipType(base: OntologyDraft, name: string) {
  return base.relationship_types.find((t) => t.name === name);
}

function draftProperties(properties: PropertyDraft[]): DraftProperty[] {
  return properties.map((p) => ({
    key: nextKey("p"),
    name: p.name,
    type: p.type,
    required: p.required,
    locked: false,
  }));
}

/** Recomputes `existing` and `locked` from the base ontology. */
function normalise(draft: BuilderDraft): BuilderDraft {
  const lock = (properties: DraftProperty[], fixed?: { properties: PropertyDraft[] }) =>
    properties.map((property) => {
      const known = fixed?.properties.find((p) => p.name === property.name);
      return known
        ? { ...property, type: known.type, required: known.required, locked: true }
        : { ...property, locked: false };
    });
  return {
    ...draft,
    types: draft.types.map((type) => {
      const existing = baseEntityType(draft.base, type.name);
      return { ...type, existing: Boolean(existing), properties: lock(type.properties, existing) };
    }),
    relationshipTypes: draft.relationshipTypes.map((type) => {
      const existing = baseRelationshipType(draft.base, type.name);
      return { ...type, existing: Boolean(existing), properties: lock(type.properties, existing) };
    }),
  };
}

function annotation(list: TypeAnnotation[], name: string): TypeAnnotation | undefined {
  return list.find((a) => a.name === name);
}

/** Base types the draft does not mention yet, as existing draft types. */
function missingBaseTypes(base: OntologyDraft, types: DraftEntityType[]): DraftEntityType[] {
  return base.entity_types
    .filter((t) => !types.some((d) => d.name === t.name))
    .map((t) => ({
      key: nextKey("t"),
      name: t.name,
      displayName: t.display_name || t.name,
      existing: true,
      confidence: null,
      reason: "",
      properties: draftProperties(t.properties),
    }));
}

function missingBaseRelationshipTypes(
  base: OntologyDraft,
  types: DraftEntityType[],
  relationshipTypes: DraftRelationshipType[],
): DraftRelationshipType[] {
  const keyOf = (name: string) => types.find((t) => t.name === name)?.key ?? "";
  return base.relationship_types
    .filter((t) => !relationshipTypes.some((d) => d.name === t.name))
    .map((t) => ({
      key: nextKey("r"),
      name: t.name,
      sourceTypeKey: keyOf(t.source_entity_type),
      targetTypeKey: keyOf(t.target_entity_type),
      existing: true,
      confidence: null,
      reason: "",
      properties: draftProperties(t.properties),
    }));
}

/** A draft from a validated AI suggestion over `datasets`. */
export function fromSuggestion(
  suggestion: GraphBuilderSuggestion,
  base: OntologyDraft,
  datasets: DatasetInfo[],
): BuilderDraft {
  const notes = suggestion.annotations;
  const suggested: DraftEntityType[] = suggestion.ontology.entity_types.map((t) => ({
    key: nextKey("t"),
    name: t.name,
    displayName: t.display_name || t.name,
    existing: false,
    confidence: annotation(notes.entities, t.name)?.confidence ?? null,
    reason: annotation(notes.entities, t.name)?.reason ?? "",
    properties: draftProperties(t.properties),
  }));
  const types = [...suggested, ...missingBaseTypes(base, suggested)];
  const typeOf = (name: string) => types.find((t) => t.name === name);
  const relationshipTypes: DraftRelationshipType[] = suggestion.ontology.relationship_types.map(
    (t) => ({
      key: nextKey("r"),
      name: t.name,
      sourceTypeKey: typeOf(t.source_entity_type)?.key ?? "",
      targetTypeKey: typeOf(t.target_entity_type)?.key ?? "",
      existing: false,
      confidence: annotation(notes.relationships, t.name)?.confidence ?? null,
      reason: annotation(notes.relationships, t.name)?.reason ?? "",
      properties: draftProperties(t.properties),
    }),
  );
  relationshipTypes.push(...missingBaseRelationshipTypes(base, types, relationshipTypes));

  const draftDatasets = datasets.map((info): DatasetDraft => {
    const mapping =
      suggestion.mappings.find((m) => m.data_source_id === info.dataSourceId)?.mapping ??
      suggestion.mappings.find((m) => m.dataset === info.name)?.mapping;
    const entities: DraftEntityMapping[] = (mapping?.entities ?? []).map((m) => {
      const type = typeOf(m.entity_type);
      return {
        key: nextKey("e"),
        alias: m.alias,
        typeKey: type?.key ?? "",
        mode: m.mode,
        fields: (type?.properties ?? []).map((p) => {
          const identity = m.identity.find((f) => f.property === p.name);
          const property = m.properties.find((f) => f.property === p.name);
          return {
            propertyKey: p.key,
            sourceField: identity?.source_field ?? property?.source_field ?? null,
            identity: Boolean(identity),
          };
        }),
      };
    });
    const relationships: DraftRelationshipMapping[] = (mapping?.relationships ?? []).map((m) => {
      const type = relationshipTypes.find((t) => t.name === m.relationship_type);
      return {
        key: nextKey("m"),
        typeKey: type?.key ?? "",
        sourceKey: entities.find((e) => e.alias === m.source)?.key ?? "",
        targetKey: entities.find((e) => e.alias === m.target)?.key ?? "",
        fields: (type?.properties ?? []).map((p) => ({
          propertyKey: p.key,
          sourceField: m.properties.find((f) => f.property === p.name)?.source_field ?? null,
          identity: false,
        })),
      };
    });
    return { ...info, entities, relationships };
  });

  return normalise({
    base,
    origin: "ai",
    confidence: suggestion.confidence,
    warnings: suggestion.warnings,
    types,
    relationshipTypes,
    datasets: draftDatasets,
  });
}

/** `customer_id` → `Customer`; anything else → `Record`. */
function guessTypeName(fields: string[]): string {
  const id = fields.find((f) => /_id$/i.test(f));
  const stem = id?.replace(/_id$/i, "") ?? "record";
  const name = stem
    .split(/[^A-Za-z0-9]+/)
    .filter(Boolean)
    .map((part) => part[0].toUpperCase() + part.slice(1))
    .join("");
  return MACHINE_NAME.test(name) ? name : "Record";
}

export function machineName(field: string): string {
  const cleaned = field.replace(/[^A-Za-z0-9_]/g, "_").replace(/^[^A-Za-z]+/, "");
  return (cleaned || "field").slice(0, 64);
}

function uniqueName(wanted: string, taken: (name: string) => boolean): string {
  if (!taken(wanted)) return wanted;
  for (let i = 2; ; i += 1) {
    const candidate = `${wanted}${i}`.slice(0, 64);
    if (!taken(candidate)) return candidate;
  }
}

/**
 * A manual starting point: for each dataset, one entity holding its fields
 * (an existing type when the guessed name exists already).
 */
export function manualDraft(datasets: DatasetInfo[], base: OntologyDraft): BuilderDraft {
  const types = missingBaseTypes(base, []);
  const draftDatasets = datasets.map((info): DatasetDraft => {
    const fields = info.profile.fields;
    const names = fields.map((f) => f.name);
    const identityField =
      names.find((f) => /_id$/i.test(f)) ?? names.find((f) => f.toLowerCase() === "id") ?? names[0];
    const guessed = guessTypeName(names);
    let type = types.find((t) => t.name === guessed && t.existing);
    if (!type) {
      type = {
        key: nextKey("t"),
        name: uniqueName(guessed, (n) => types.some((t) => t.name === n)),
        displayName: guessed,
        existing: false,
        confidence: null,
        reason: "",
        properties: fields.map((field) => {
          const identity = field.name === identityField;
          return {
            key: nextKey("p"),
            name: machineName(field.name),
            type: identity || /_id$/i.test(field.name) ? "string" : field.inferred_type,
            required: identity && !field.nullable,
            locked: false,
          };
        }),
      };
      types.push(type);
    }
    const mapped = type;
    return {
      ...info,
      entities: identityField
        ? [
            {
              key: nextKey("e"),
              alias: mapped.name.toLowerCase(),
              typeKey: mapped.key,
              mode: "create_or_match",
              fields: mapped.properties.map((p) => {
                const field = fields.find((f) => machineName(f.name) === p.name);
                return {
                  propertyKey: p.key,
                  sourceField: field?.name ?? null,
                  identity: field?.name === identityField,
                };
              }),
            },
          ]
        : [],
      relationships: [],
    };
  });
  return normalise({
    base,
    origin: "manual",
    confidence: null,
    warnings: [],
    types,
    relationshipTypes: missingBaseRelationshipTypes(base, types, []),
    datasets: draftDatasets,
  });
}

// --- lookups --------------------------------------------------------------------

export function typeByKey(draft: BuilderDraft, key: string): DraftEntityType | undefined {
  return draft.types.find((t) => t.key === key);
}

export function relationshipTypeByKey(
  draft: BuilderDraft,
  key: string,
): DraftRelationshipType | undefined {
  return draft.relationshipTypes.find((t) => t.key === key);
}

export function datasetById(draft: BuilderDraft, dataSourceId: string): DatasetDraft | undefined {
  return draft.datasets.find((d) => d.dataSourceId === dataSourceId);
}

/** The datasets whose records create (`create_or_match`) entities of a type. */
export function creatorsOf(draft: BuilderDraft, typeKey: string): DatasetDraft[] {
  return draft.datasets.filter((d) =>
    d.entities.some((e) => e.typeKey === typeKey && e.mode === "create_or_match"),
  );
}

/** Every mapping of a relationship type, with its dataset. */
export function relationshipMappingsOf(
  draft: BuilderDraft,
  typeKey: string,
): { dataset: DatasetDraft; mapping: DraftRelationshipMapping }[] {
  return draft.datasets.flatMap((dataset) =>
    dataset.relationships
      .filter((m) => m.typeKey === typeKey)
      .map((mapping) => ({ dataset, mapping })),
  );
}

/** The source field of a mapped property, if any. */
export function fieldOf(fields: DraftFieldMap[], propertyKey: string): DraftFieldMap | undefined {
  return fields.find((f) => f.propertyKey === propertyKey);
}

/** The first identity source field of an entity mapping. */
export function identityFieldOf(entity: DraftEntityMapping): string | null {
  return entity.fields.find((f) => f.identity && f.sourceField)?.sourceField ?? null;
}

// --- edits: entity and relationship types -------------------------------------------

function editType(
  draft: BuilderDraft,
  key: string,
  edit: (type: DraftEntityType) => DraftEntityType,
): BuilderDraft {
  return normalise({
    ...draft,
    types: draft.types.map((t) => (t.key === key ? edit(t) : t)),
  });
}

function editRelationshipType(
  draft: BuilderDraft,
  key: string,
  edit: (type: DraftRelationshipType) => DraftRelationshipType,
): BuilderDraft {
  return normalise({
    ...draft,
    relationshipTypes: draft.relationshipTypes.map((t) => (t.key === key ? edit(t) : t)),
  });
}

function editDataset(
  draft: BuilderDraft,
  dataSourceId: string,
  edit: (dataset: DatasetDraft) => DatasetDraft,
): BuilderDraft {
  return {
    ...draft,
    datasets: draft.datasets.map((d) => (d.dataSourceId === dataSourceId ? edit(d) : d)),
  };
}

/** Renames (new types only) or relabels a type. */
export function updateType(
  draft: BuilderDraft,
  key: string,
  patch: Partial<Pick<DraftEntityType, "name" | "displayName">>,
): BuilderDraft {
  return editType(draft, key, (t) => ({
    ...t,
    displayName: patch.displayName ?? t.displayName,
    name: t.existing || patch.name === undefined ? t.name : patch.name,
  }));
}

/** A new entity type, mapped (identity not yet chosen) in `dataSourceId`. */
export function addType(draft: BuilderDraft, dataSourceId?: string): BuilderDraft {
  const name = uniqueName("Entity", (n) => draft.types.some((t) => t.name === n));
  const type: DraftEntityType = {
    key: nextKey("t"),
    name,
    displayName: name,
    existing: false,
    confidence: null,
    reason: "",
    properties: [{ key: nextKey("p"), name: "id", type: "string", required: true, locked: false }],
  };
  const target = dataSourceId ?? draft.datasets[0]?.dataSourceId;
  const withType = normalise({ ...draft, types: [...draft.types, type] });
  return target ? addEntityMapping(withType, target, type.key) : withType;
}

/** Removes a new type with every mapping and relationship type using it. */
export function removeType(draft: BuilderDraft, key: string): BuilderDraft {
  const type = typeByKey(draft, key);
  if (!type || type.existing) return draft;
  const removedRelationshipTypes = new Set(
    draft.relationshipTypes
      .filter((r) => r.sourceTypeKey === key || r.targetTypeKey === key)
      .map((r) => r.key),
  );
  return normalise({
    ...draft,
    types: draft.types.filter((t) => t.key !== key),
    relationshipTypes: draft.relationshipTypes.filter((r) => !removedRelationshipTypes.has(r.key)),
    datasets: draft.datasets.map((dataset) => {
      const removedEntities = new Set(
        dataset.entities.filter((e) => e.typeKey === key).map((e) => e.key),
      );
      return {
        ...dataset,
        entities: dataset.entities.filter((e) => !removedEntities.has(e.key)),
        relationships: dataset.relationships.filter(
          (m) =>
            !removedRelationshipTypes.has(m.typeKey) &&
            !removedEntities.has(m.sourceKey) &&
            !removedEntities.has(m.targetKey),
        ),
      };
    }),
  });
}

export type PropertyOwner = "entity" | "relationship";

function editOwnerProperties(
  draft: BuilderDraft,
  owner: PropertyOwner,
  ownerKey: string,
  edit: (properties: DraftProperty[]) => DraftProperty[],
): BuilderDraft {
  return owner === "entity"
    ? editType(draft, ownerKey, (t) => ({ ...t, properties: edit(t.properties) }))
    : editRelationshipType(draft, ownerKey, (t) => ({ ...t, properties: edit(t.properties) }));
}

export function addProperty(
  draft: BuilderDraft,
  owner: PropertyOwner,
  ownerKey: string,
): BuilderDraft {
  return editOwnerProperties(draft, owner, ownerKey, (properties) => [
    ...properties,
    {
      key: nextKey("p"),
      name: uniqueName("property", (n) => properties.some((p) => p.name === n)),
      type: "string",
      required: false,
      locked: false,
    },
  ]);
}

/** Changes a new property; properties of the base ontology are fixed. */
export function updateProperty(
  draft: BuilderDraft,
  owner: PropertyOwner,
  ownerKey: string,
  propertyKey: string,
  patch: Partial<Pick<DraftProperty, "name" | "type" | "required">>,
): BuilderDraft {
  return editOwnerProperties(draft, owner, ownerKey, (properties) =>
    properties.map((p) => (p.key !== propertyKey || p.locked ? p : { ...p, ...patch })),
  );
}

/** Removes a new property and every field mapped to it. */
export function removeProperty(
  draft: BuilderDraft,
  owner: PropertyOwner,
  ownerKey: string,
  propertyKey: string,
): BuilderDraft {
  const owned =
    owner === "entity" ? typeByKey(draft, ownerKey) : relationshipTypeByKey(draft, ownerKey);
  if (owned?.properties.find((p) => p.key === propertyKey)?.locked) return draft;
  const edited = editOwnerProperties(draft, owner, ownerKey, (properties) =>
    properties.filter((p) => p.key !== propertyKey),
  );
  const drop = (fields: DraftFieldMap[]) => fields.filter((f) => f.propertyKey !== propertyKey);
  return {
    ...edited,
    datasets: edited.datasets.map((dataset) => ({
      ...dataset,
      entities: dataset.entities.map((e) => ({ ...e, fields: drop(e.fields) })),
      relationships: dataset.relationships.map((m) => ({ ...m, fields: drop(m.fields) })),
    })),
  };
}

/** Renames a new relationship type; existing ones keep their name. */
export function renameRelationshipType(
  draft: BuilderDraft,
  key: string,
  name: string,
): BuilderDraft {
  return editRelationshipType(draft, key, (t) => (t.existing ? t : { ...t, name }));
}

/** Swaps the endpoints of a new relationship type and of its mappings. */
export function reverseRelationshipType(draft: BuilderDraft, key: string): BuilderDraft {
  const type = relationshipTypeByKey(draft, key);
  if (!type || type.existing) return draft;
  const reversed = editRelationshipType(draft, key, (t) => ({
    ...t,
    sourceTypeKey: t.targetTypeKey,
    targetTypeKey: t.sourceTypeKey,
  }));
  return {
    ...reversed,
    datasets: reversed.datasets.map((dataset) => ({
      ...dataset,
      relationships: dataset.relationships.map((m) =>
        m.typeKey === key ? { ...m, sourceKey: m.targetKey, targetKey: m.sourceKey } : m,
      ),
    })),
  };
}

/**
 * A new relationship type between two entity types, mapped in the first
 * dataset that maps both of them.
 */
export function addRelationshipType(
  draft: BuilderDraft,
  sourceTypeKey: string,
  targetTypeKey: string,
): BuilderDraft {
  const type: DraftRelationshipType = {
    key: nextKey("r"),
    name: uniqueName("RELATED_TO", (n) => draft.relationshipTypes.some((t) => t.name === n)),
    sourceTypeKey,
    targetTypeKey,
    existing: false,
    confidence: null,
    reason: "",
    properties: [],
  };
  const withType = normalise({
    ...draft,
    relationshipTypes: [...draft.relationshipTypes, type],
  });
  for (const host of withType.datasets) {
    const source = host.entities.find((e) => e.typeKey === sourceTypeKey);
    const target =
      host.entities.find((e) => e.typeKey === targetTypeKey && e.key !== source?.key) ??
      host.entities.find((e) => e.typeKey === targetTypeKey);
    if (!source || !target) continue;
    return editDataset(withType, host.dataSourceId, (d) => ({
      ...d,
      relationships: [
        ...d.relationships,
        {
          key: nextKey("m"),
          typeKey: type.key,
          sourceKey: source.key,
          targetKey: target.key,
          fields: [],
        },
      ],
    }));
  }
  return withType;
}

/** Removes a new relationship type and its mappings. */
export function removeRelationshipType(draft: BuilderDraft, key: string): BuilderDraft {
  const type = relationshipTypeByKey(draft, key);
  if (!type || type.existing) return draft;
  return {
    ...draft,
    relationshipTypes: draft.relationshipTypes.filter((t) => t.key !== key),
    datasets: draft.datasets.map((dataset) => ({
      ...dataset,
      relationships: dataset.relationships.filter((m) => m.typeKey !== key),
    })),
  };
}

/**
 * Maps an existing relationship type in a dataset, between the dataset's
 * entities of the endpoint types. No change when the dataset lacks them.
 */
export function addRelationshipMapping(
  draft: BuilderDraft,
  dataSourceId: string,
  typeKey: string,
): BuilderDraft {
  const type = relationshipTypeByKey(draft, typeKey);
  const dataset = datasetById(draft, dataSourceId);
  if (!type || !dataset) return draft;
  const source = dataset.entities.find((e) => e.typeKey === type.sourceTypeKey);
  const target =
    dataset.entities.find((e) => e.typeKey === type.targetTypeKey && e.key !== source?.key) ??
    dataset.entities.find((e) => e.typeKey === type.targetTypeKey);
  if (!source || !target) return draft;
  return editDataset(draft, dataSourceId, (d) => ({
    ...d,
    relationships: [
      ...d.relationships,
      { key: nextKey("m"), typeKey, sourceKey: source.key, targetKey: target.key, fields: [] },
    ],
  }));
}

// --- edits: per-dataset mappings ----------------------------------------------------

/** Maps a type in a dataset, pre-filling fields whose names match. */
export function addEntityMapping(
  draft: BuilderDraft,
  dataSourceId: string,
  typeKey: string,
): BuilderDraft {
  const type = typeByKey(draft, typeKey);
  if (!type) return draft;
  return editDataset(draft, dataSourceId, (d) => {
    const alias = uniqueName(type.name.toLowerCase(), (a) => d.entities.some((e) => e.alias === a));
    const fieldNames = d.profile.fields.map((f) => f.name);
    return {
      ...d,
      entities: [
        ...d.entities,
        {
          key: nextKey("e"),
          alias,
          typeKey,
          mode: "create_or_match",
          fields: type.properties.map((p) => ({
            propertyKey: p.key,
            sourceField: fieldNames.find((f) => machineName(f) === p.name) ?? null,
            identity: false,
          })),
        },
      ],
    };
  });
}

export function updateEntityMapping(
  draft: BuilderDraft,
  dataSourceId: string,
  key: string,
  patch: Partial<Pick<DraftEntityMapping, "alias" | "mode">>,
): BuilderDraft {
  return editDataset(draft, dataSourceId, (d) => ({
    ...d,
    entities: d.entities.map((e) => (e.key === key ? { ...e, ...patch } : e)),
  }));
}

/** Sets where a property of an entity mapping comes from. */
export function setEntityField(
  draft: BuilderDraft,
  dataSourceId: string,
  key: string,
  propertyKey: string,
  patch: Partial<Pick<DraftFieldMap, "sourceField" | "identity">>,
): BuilderDraft {
  return editDataset(draft, dataSourceId, (d) => ({
    ...d,
    entities: d.entities.map((e) => {
      if (e.key !== key) return e;
      const current = fieldOf(e.fields, propertyKey);
      const next = {
        ...(current ?? { propertyKey, sourceField: null, identity: false }),
        ...patch,
      };
      return {
        ...e,
        fields: current
          ? e.fields.map((f) => (f.propertyKey === propertyKey ? next : f))
          : [...e.fields, next],
      };
    }),
  }));
}

/** Removes an entity mapping and the relationships it takes part in. */
export function removeEntityMapping(
  draft: BuilderDraft,
  dataSourceId: string,
  key: string,
): BuilderDraft {
  return editDataset(draft, dataSourceId, (d) => ({
    ...d,
    entities: d.entities.filter((e) => e.key !== key),
    relationships: d.relationships.filter((m) => m.sourceKey !== key && m.targetKey !== key),
  }));
}

export function updateRelationshipMapping(
  draft: BuilderDraft,
  dataSourceId: string,
  key: string,
  patch: Partial<Pick<DraftRelationshipMapping, "sourceKey" | "targetKey">>,
): BuilderDraft {
  return editDataset(draft, dataSourceId, (d) => ({
    ...d,
    relationships: d.relationships.map((m) => (m.key === key ? { ...m, ...patch } : m)),
  }));
}

export function setRelationshipField(
  draft: BuilderDraft,
  dataSourceId: string,
  key: string,
  propertyKey: string,
  sourceField: string | null,
): BuilderDraft {
  return editDataset(draft, dataSourceId, (d) => ({
    ...d,
    relationships: d.relationships.map((m) => {
      if (m.key !== key) return m;
      const next = { propertyKey, sourceField, identity: false };
      return {
        ...m,
        fields: fieldOf(m.fields, propertyKey)
          ? m.fields.map((f) => (f.propertyKey === propertyKey ? next : f))
          : [...m.fields, next],
      };
    }),
  }));
}

/** The same draft on top of a newer base ontology (after it was saved). */
export function rebase(draft: BuilderDraft, base: OntologyDraft): BuilderDraft {
  return normalise({ ...draft, base });
}

// --- ordering, validation and submission ------------------------------------------

/**
 * Mapped dataset ids in a safe ingestion order: a dataset that only matches
 * a type comes after the datasets that create it; otherwise the draft's
 * order. `null` when datasets depend on each other in a cycle.
 */
export function ingestionOrder(draft: BuilderDraft): string[] | null {
  const mapped = draft.datasets.filter((d) => d.entities.length > 0);
  const typesWith = (d: DatasetDraft, mode: EntityMode) =>
    new Set(d.entities.filter((e) => e.mode === mode).map((e) => e.typeKey));
  const creates = mapped.map((d) => typesWith(d, "create_or_match"));
  const matches = mapped.map((d) => typesWith(d, "match_only"));
  const depends = (j: number, i: number) =>
    i !== j && [...matches[j]].some((type) => creates[i].has(type));
  const done = mapped.map(() => false);
  const order: string[] = [];
  while (order.length < mapped.length) {
    const next = mapped.findIndex(
      (_, j) => !done[j] && mapped.every((__, i) => done[i] || !depends(j, i)),
    );
    if (next < 0) return null;
    done[next] = true;
    order.push(mapped[next].dataSourceId);
  }
  return order;
}

/** Problems that would make the server reject the draft. */
export function validateDraft(draft: BuilderDraft): string[] {
  const errors: string[] = [];
  if (!draft.datasets.some((d) => d.entities.length > 0)) {
    errors.push("Map at least one entity.");
  }
  const typeNames = new Set<string>();
  for (const type of draft.types) {
    if (!MACHINE_NAME.test(type.name)) {
      errors.push(
        `Entity type "${type.name}" must start with a letter and use only letters, digits and _.`,
      );
    } else if (typeNames.has(type.name)) {
      errors.push(`Entity type "${type.name}" appears twice.`);
    }
    typeNames.add(type.name);
    checkProperties(type.name, type.properties, type.existing, errors);
  }
  const relationshipNames = new Set<string>();
  for (const type of draft.relationshipTypes) {
    if (!MACHINE_NAME.test(type.name)) {
      errors.push(`Relationship "${type.name}" has an invalid name.`);
    } else if (relationshipNames.has(type.name)) {
      errors.push(`Relationship "${type.name}" appears twice.`);
    }
    relationshipNames.add(type.name);
    const source = typeByKey(draft, type.sourceTypeKey);
    const target = typeByKey(draft, type.targetTypeKey);
    if (!source || !target) {
      errors.push(`Relationship ${type.name} must connect two entity types.`);
    } else if (type.existing) {
      const fixed = baseRelationshipType(draft.base, type.name);
      if (
        fixed &&
        (fixed.source_entity_type !== source.name || fixed.target_entity_type !== target.name)
      ) {
        errors.push(
          `Existing relationship ${type.name} connects ${fixed.source_entity_type} → ${fixed.target_entity_type}.`,
        );
      }
    }
    checkProperties(type.name, type.properties, type.existing, errors);
  }
  const created = new Set(
    draft.datasets.flatMap((d) =>
      d.entities.filter((e) => e.mode === "create_or_match").map((e) => e.typeKey),
    ),
  );
  for (const dataset of draft.datasets) {
    checkDataset(draft, dataset, created, errors);
  }
  if (ingestionOrder(draft) === null) {
    errors.push(
      "Some files only match entity types that each other creates; let one of them create the type.",
    );
  }
  return [...new Set(errors)];
}

function checkProperties(
  owner: string,
  properties: DraftProperty[],
  existingOwner: boolean,
  errors: string[],
) {
  const names = new Set<string>();
  for (const property of properties) {
    if (!MACHINE_NAME.test(property.name)) {
      errors.push(`Property "${property.name}" of ${owner} has an invalid name.`);
    }
    if (names.has(property.name)) {
      errors.push(`Property "${property.name}" of ${owner} appears twice.`);
    }
    names.add(property.name);
    if (existingOwner && !property.locked && property.required) {
      errors.push(`New property "${property.name}" of existing type ${owner} must be optional.`);
    }
  }
}

function checkDataset(
  draft: BuilderDraft,
  dataset: DatasetDraft,
  created: Set<string>,
  errors: string[],
) {
  const known = new Set(dataset.profile.fields.map((f) => f.name));
  const at = `in ${dataset.name}`;
  const aliases = new Set<string>();
  for (const entity of dataset.entities) {
    const type = typeByKey(draft, entity.typeKey);
    const label = type?.name ?? "an entity";
    if (!type) errors.push(`An entity ${at} has no type.`);
    if (!entity.alias.trim()) errors.push(`Entity ${label} ${at} needs an alias.`);
    else if (aliases.has(entity.alias)) errors.push(`Alias "${entity.alias}" is used twice ${at}.`);
    aliases.add(entity.alias);
    for (const field of entity.fields) {
      if (field.sourceField !== null && !known.has(field.sourceField)) {
        errors.push(`Source field "${field.sourceField}" does not exist ${at}.`);
      }
    }
    const identity = entity.fields.filter((f) => f.identity);
    if (identity.length === 0) {
      errors.push(`Entity ${label} ${at} needs at least one identity property.`);
    }
    for (const field of identity) {
      if (field.sourceField === null) {
        const name = type?.properties.find((p) => p.key === field.propertyKey)?.name;
        errors.push(`Identity property "${name}" of ${label} ${at} needs a source field.`);
      }
    }
    if (type && entity.mode === "create_or_match") {
      for (const property of type.properties) {
        if (property.required && !fieldOf(entity.fields, property.key)?.sourceField) {
          errors.push(
            `Required property "${property.name}" of ${label} ${at} needs a source field.`,
          );
        }
      }
    }
    if (type && entity.mode === "match_only" && !type.existing && !created.has(type.key)) {
      errors.push(
        `${label} is only matched ${at}, but no file creates it; set one of its mappings to “Create or match”.`,
      );
    }
  }
  for (const relationship of dataset.relationships) {
    const type = relationshipTypeByKey(draft, relationship.typeKey);
    const label = type?.name ?? "a relationship";
    const source = dataset.entities.find((e) => e.key === relationship.sourceKey);
    const target = dataset.entities.find((e) => e.key === relationship.targetKey);
    if (!type || !source || !target) {
      errors.push(`Relationship ${label} ${at} must connect two entities of that file.`);
      continue;
    }
    if (source.typeKey !== type.sourceTypeKey || target.typeKey !== type.targetTypeKey) {
      const name = (key: string) => typeByKey(draft, key)?.name ?? "?";
      errors.push(
        `Relationship ${label} ${at} must connect ${name(type.sourceTypeKey)} → ${name(type.targetTypeKey)}.`,
      );
    }
    for (const field of relationship.fields) {
      if (field.sourceField !== null && !known.has(field.sourceField)) {
        errors.push(`Source field "${field.sourceField}" does not exist ${at}.`);
      }
    }
  }
}

function mergeProperties(base: PropertyDraft[], added: DraftProperty[]): PropertyDraft[] {
  const merged = [...base];
  for (const property of added) {
    if (!merged.some((p) => p.name === property.name)) {
      merged.push({ name: property.name, type: property.type, required: property.required });
    }
  }
  return merged;
}

function datasetMapping(draft: BuilderDraft, dataset: DatasetDraft): IngestionMapping {
  const mapped = (owner: { properties: DraftProperty[] } | undefined, fields: DraftFieldMap[]) =>
    fields.flatMap((f) => {
      const property = owner?.properties.find((p) => p.key === f.propertyKey)?.name;
      return f.sourceField !== null && property ? [{ source_field: f.sourceField, property }] : [];
    });
  return {
    entities: dataset.entities.map((entity) => {
      const type = typeByKey(draft, entity.typeKey);
      return {
        alias: entity.alias,
        entity_type: type?.name ?? "",
        mode: entity.mode,
        identity: mapped(
          type,
          entity.fields.filter((f) => f.identity),
        ),
        properties: entity.mode === "create_or_match" ? mapped(type, entity.fields) : [],
      };
    }),
    relationships: dataset.relationships.map((relationship) => {
      const type = relationshipTypeByKey(draft, relationship.typeKey);
      const alias = (key: string) => dataset.entities.find((e) => e.key === key)?.alias ?? "";
      return {
        relationship_type: type?.name ?? "",
        source: alias(relationship.sourceKey),
        target: alias(relationship.targetKey),
        properties: mapped(type, relationship.fields),
      };
    }),
  };
}

/**
 * The ontology to submit with `POST .../ontology` and one ingestion per
 * mapped dataset, in a safe order (datasets without mappings are skipped).
 */
export function toSubmission(draft: BuilderDraft): {
  ontology: OntologyDraft;
  ingestions: IngestionPlan[];
} {
  const entityTypes = draft.base.entity_types.map((t) => ({ ...t, properties: [...t.properties] }));
  for (const type of draft.types) {
    const index = entityTypes.findIndex((t) => t.name === type.name);
    if (index >= 0) {
      entityTypes[index] = {
        ...entityTypes[index],
        properties: mergeProperties(entityTypes[index].properties, type.properties),
      };
    } else {
      entityTypes.push({
        name: type.name,
        display_name: type.displayName || type.name,
        description: null,
        properties: mergeProperties([], type.properties),
      });
    }
  }
  const relationshipTypes = draft.base.relationship_types.map((t) => ({ ...t }));
  for (const type of draft.relationshipTypes) {
    const index = relationshipTypes.findIndex((t) => t.name === type.name);
    if (index >= 0) {
      relationshipTypes[index] = {
        ...relationshipTypes[index],
        properties: mergeProperties(relationshipTypes[index].properties, type.properties),
      };
    } else {
      relationshipTypes.push({
        name: type.name,
        source_entity_type: typeByKey(draft, type.sourceTypeKey)?.name ?? "",
        target_entity_type: typeByKey(draft, type.targetTypeKey)?.name ?? "",
        properties: mergeProperties([], type.properties),
      });
    }
  }
  const order = ingestionOrder(draft) ?? draft.datasets.map((d) => d.dataSourceId);
  const ingestions = order
    .map((id) => datasetById(draft, id))
    .filter((d): d is DatasetDraft => Boolean(d && d.entities.length > 0))
    .map((dataset) => ({
      dataSourceId: dataset.dataSourceId,
      name: dataset.name,
      mapping: datasetMapping(draft, dataset),
    }));
  return {
    ontology: { entity_types: entityTypes, relationship_types: relationshipTypes },
    ingestions,
  };
}

/** Whether submitting `ontology` would change nothing compared to `base`. */
export function ontologyUnchanged(base: OntologyDraft, ontology: OntologyDraft): boolean {
  const canonical = (o: OntologyDraft) =>
    JSON.stringify({
      e: o.entity_types.map((t) => [t.name, t.properties.map((p) => [p.name, p.type, p.required])]),
      r: o.relationship_types.map((t) => [
        t.name,
        t.source_entity_type,
        t.target_entity_type,
        t.properties.map((p) => [p.name, p.type, p.required]),
      ]),
    });
  return canonical(base) === canonical(ontology);
}

// --- presentation helpers -----------------------------------------------------------

export type ConfidenceLevel = "high" | "medium" | "low";

/** Display bands only: the value is the model's own estimate. */
export function confidenceLevel(confidence: number): ConfidenceLevel {
  if (confidence >= 0.85) return "high";
  if (confidence >= 0.6) return "medium";
  return "low";
}

/**
 * How a relationship mapping joins files, for example
 * `devices.csv.customer_id → customers.csv.customer_id`: the identity field
 * of a matched endpoint and that of the file that creates its type.
 */
export function joinHint(
  draft: BuilderDraft,
  dataset: DatasetDraft,
  relationship: DraftRelationshipMapping,
): { from: string; to: string } | null {
  for (const key of [relationship.sourceKey, relationship.targetKey]) {
    const entity = dataset.entities.find((e) => e.key === key);
    if (!entity || entity.mode !== "match_only") continue;
    const field = identityFieldOf(entity);
    const creator = creatorsOf(draft, entity.typeKey).find(
      (d) => d.dataSourceId !== dataset.dataSourceId,
    );
    const created = creator?.entities.find(
      (e) => e.typeKey === entity.typeKey && e.mode === "create_or_match",
    );
    const createdField = created ? identityFieldOf(created) : null;
    if (field && creator && createdField) {
      return { from: `${dataset.name}.${field}`, to: `${creator.name}.${createdField}` };
    }
  }
  return null;
}

/** Up to `limit` (source, target) identity pairs from a dataset's sample rows. */
export function sampleMatches(
  dataset: DatasetDraft,
  relationship: DraftRelationshipMapping,
  limit = 3,
): { source: string; target: string; rows: [string, string][] } | null {
  const sourceEntity = dataset.entities.find((e) => e.key === relationship.sourceKey);
  const targetEntity = dataset.entities.find((e) => e.key === relationship.targetKey);
  const source = sourceEntity ? identityFieldOf(sourceEntity) : null;
  const target = targetEntity ? identityFieldOf(targetEntity) : null;
  if (!source || !target) return null;
  const text = (value: unknown) =>
    value === null || value === undefined
      ? ""
      : typeof value === "string"
        ? value
        : JSON.stringify(value);
  const rows = dataset.profile.sample_rows
    .map((row) => [text(row[source]), text(row[target])] as [string, string])
    .filter(([a, b]) => a !== "" && b !== "")
    .slice(0, limit);
  return { source, target, rows };
}

function yamlScalar(value: string): string {
  return /^[A-Za-z_][A-Za-z0-9_.-]*$/.test(value) &&
    !/^(true|false|null|yes|no|on|off)$/i.test(value)
    ? value
    : JSON.stringify(value);
}

/** The draft as readable YAML: the ontology plus one mapping per file. */
export function toYaml(draft: BuilderDraft): string {
  const { ontology, ingestions } = toSubmission(draft);
  const lines: string[] = ["ontology:"];
  const props = (indent: string, list: PropertyDraft[]) => {
    if (list.length === 0) {
      lines.push(`${indent}properties: []`);
      return;
    }
    lines.push(`${indent}properties:`);
    for (const p of list) {
      lines.push(
        `${indent}  - { name: ${yamlScalar(p.name)}, type: ${p.type}, required: ${p.required} }`,
      );
    }
  };
  lines.push(ontology.entity_types.length ? "  entity_types:" : "  entity_types: []");
  for (const type of ontology.entity_types) {
    lines.push(`    - name: ${yamlScalar(type.name)}`);
    props("      ", type.properties);
  }
  lines.push(
    ontology.relationship_types.length ? "  relationship_types:" : "  relationship_types: []",
  );
  for (const type of ontology.relationship_types) {
    lines.push(`    - name: ${yamlScalar(type.name)}`);
    lines.push(`      source: ${yamlScalar(type.source_entity_type)}`);
    lines.push(`      target: ${yamlScalar(type.target_entity_type)}`);
    props("      ", type.properties);
  }
  lines.push(ingestions.length ? "mappings:" : "mappings: []");
  const pairs = (list: { source_field: string; property: string }[]) =>
    `{ ${list.map((f) => `${yamlScalar(f.property)}: ${yamlScalar(f.source_field)}`).join(", ")} }`;
  for (const ingestion of ingestions) {
    lines.push(`  - file: ${yamlScalar(ingestion.name)}`);
    lines.push("    entities:");
    for (const e of ingestion.mapping.entities) {
      lines.push(`      - alias: ${yamlScalar(e.alias)}`);
      lines.push(`        type: ${yamlScalar(e.entity_type)}`);
      lines.push(`        mode: ${e.mode}`);
      lines.push(`        identity: ${pairs(e.identity)}`);
      if (e.properties.length) lines.push(`        properties: ${pairs(e.properties)}`);
    }
    if (ingestion.mapping.relationships.length) {
      lines.push("    relationships:");
      for (const r of ingestion.mapping.relationships) {
        lines.push(
          `      - { type: ${yamlScalar(r.relationship_type)}, source: ${yamlScalar(r.source)}, target: ${yamlScalar(r.target)} }`,
        );
      }
    }
  }
  return `${lines.join("\n")}\n`;
}
