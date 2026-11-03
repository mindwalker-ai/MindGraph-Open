/**
 * The Graph Builder review draft: editable client state between an AI (or
 * manual) suggestion and "Generate Graph".
 *
 * A draft is a list of entities (one per mapping alias, with their type's
 * properties and source fields) and relationships between aliases. It turns
 * into exactly the two requests of the M4 pipeline: the next ontology version
 * (`POST /graphs/{id}/ontology`) and an ingestion mapping
 * (`POST .../ingestions`). The server validates both again; this module only
 * keeps the UI from submitting obviously invalid drafts.
 *
 * Every function is pure: it returns a new draft.
 */

import type {
  DatasetProfile,
  EntityMode,
  FieldMapping,
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
  /** The dataset field, or `null` when the property is not mapped. */
  sourceField: string | null;
  identity: boolean;
  /** Defined by the existing ontology: name, type and requiredness are fixed. */
  locked: boolean;
}

export interface DraftEntity {
  key: string;
  alias: string;
  typeName: string;
  displayName: string;
  mode: EntityMode;
  existing: boolean;
  confidence: number | null;
  reason: string;
  properties: DraftProperty[];
}

export interface DraftRelationship {
  key: string;
  typeName: string;
  sourceAlias: string;
  targetAlias: string;
  existing: boolean;
  confidence: number | null;
  reason: string;
  properties: DraftProperty[];
}

export interface BuilderDraft {
  /** The pinned active ontology the draft builds on (kept intact). */
  base: OntologyDraft;
  entities: DraftEntity[];
  relationships: DraftRelationship[];
  warnings: string[];
  confidence: number | null;
  origin: "ai" | "manual";
}

let counter = 0;
function nextKey(prefix: string): string {
  counter += 1;
  return `${prefix}-${counter}`;
}

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

/** Recomputes `existing` and `locked` from the base ontology. */
function normalise(draft: BuilderDraft): BuilderDraft {
  const lock = (properties: DraftProperty[], existing?: { properties: PropertyDraft[] }) =>
    properties.map((property) => {
      const fixed = existing?.properties.find((p) => p.name === property.name);
      return fixed
        ? { ...property, type: fixed.type, required: fixed.required, locked: true }
        : { ...property, locked: false };
    });
  return {
    ...draft,
    entities: draft.entities.map((entity) => {
      const existing = baseEntityType(draft.base, entity.typeName);
      return {
        ...entity,
        existing: Boolean(existing),
        properties: lock(entity.properties, existing),
      };
    }),
    relationships: draft.relationships.map((relationship) => {
      const existing = baseRelationshipType(draft.base, relationship.typeName);
      return {
        ...relationship,
        existing: Boolean(existing),
        properties: lock(relationship.properties, existing),
      };
    }),
  };
}

function draftProperties(
  properties: PropertyDraft[],
  mapped: FieldMapping[],
  identity: FieldMapping[] = [],
): DraftProperty[] {
  return properties.map((p) => ({
    key: nextKey("p"),
    name: p.name,
    type: p.type,
    required: p.required,
    sourceField:
      mapped.find((f) => f.property === p.name)?.source_field ??
      identity.find((f) => f.property === p.name)?.source_field ??
      null,
    identity: identity.some((f) => f.property === p.name),
    locked: false,
  }));
}

function annotation(list: TypeAnnotation[], name: string): TypeAnnotation | undefined {
  return list.find((a) => a.name === name);
}

/** A draft from a validated AI suggestion. */
export function fromSuggestion(
  suggestion: GraphBuilderSuggestion,
  base: OntologyDraft,
): BuilderDraft {
  const typeOf = (name: string) =>
    suggestion.ontology.entity_types.find((t) => t.name === name) ?? baseEntityType(base, name);
  const relationshipTypeOf = (name: string) =>
    suggestion.ontology.relationship_types.find((t) => t.name === name) ??
    baseRelationshipType(base, name);
  return normalise({
    base,
    origin: "ai",
    confidence: suggestion.confidence,
    warnings: suggestion.warnings,
    entities: suggestion.mapping.entities.map((mapping) => {
      const type = typeOf(mapping.entity_type);
      const note = annotation(suggestion.annotations.entities, mapping.entity_type);
      return {
        key: nextKey("e"),
        alias: mapping.alias,
        typeName: mapping.entity_type,
        displayName: type?.display_name ?? mapping.entity_type,
        mode: mapping.mode,
        existing: false,
        confidence: note?.confidence ?? null,
        reason: note?.reason ?? "",
        properties: draftProperties(type?.properties ?? [], mapping.properties, mapping.identity),
      };
    }),
    relationships: suggestion.mapping.relationships.map((mapping) => {
      const type = relationshipTypeOf(mapping.relationship_type);
      const note = annotation(suggestion.annotations.relationships, mapping.relationship_type);
      return {
        key: nextKey("r"),
        typeName: mapping.relationship_type,
        sourceAlias: mapping.source,
        targetAlias: mapping.target,
        existing: false,
        confidence: note?.confidence ?? null,
        reason: note?.reason ?? "",
        properties: draftProperties(type?.properties ?? [], mapping.properties),
      };
    }),
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

function machineName(field: string): string {
  const cleaned = field.replace(/[^A-Za-z0-9_]/g, "_").replace(/^[^A-Za-z]+/, "");
  return (cleaned || "field").slice(0, 64);
}

/** A manual starting point: one entity holding every field of the profile. */
export function manualDraft(profile: DatasetProfile, base: OntologyDraft): BuilderDraft {
  const names = profile.fields.map((f) => f.name);
  const identityField =
    names.find((f) => /_id$/i.test(f)) ?? names.find((f) => f.toLowerCase() === "id") ?? names[0];
  const typeName = guessTypeName(names);
  return normalise({
    base,
    origin: "manual",
    confidence: null,
    warnings: [],
    entities: [
      {
        key: nextKey("e"),
        alias: typeName.toLowerCase(),
        typeName,
        displayName: typeName,
        mode: "create_or_match",
        existing: false,
        confidence: null,
        reason: "",
        properties: profile.fields.map((field) => {
          const identity = field.name === identityField;
          return {
            key: nextKey("p"),
            name: machineName(field.name),
            type: identity || /_id$/i.test(field.name) ? "string" : field.inferred_type,
            required: identity && !field.nullable,
            sourceField: field.name,
            identity,
            locked: false,
          };
        }),
      },
    ],
    relationships: [],
  });
}

// --- edits --------------------------------------------------------------------

/** The same draft on top of a newer base ontology (after it was saved). */
export function rebase(draft: BuilderDraft, base: OntologyDraft): BuilderDraft {
  return normalise({ ...draft, base });
}

export function updateEntity(
  draft: BuilderDraft,
  key: string,
  patch: Partial<Pick<DraftEntity, "alias" | "typeName" | "displayName" | "mode">>,
): BuilderDraft {
  const before = draft.entities.find((e) => e.key === key);
  if (!before) return draft;
  const alias = patch.alias ?? before.alias;
  return normalise({
    ...draft,
    entities: draft.entities.map((e) => (e.key === key ? { ...e, ...patch } : e)),
    // A renamed alias keeps its relationships.
    relationships: draft.relationships.map((r) => ({
      ...r,
      sourceAlias: r.sourceAlias === before.alias ? alias : r.sourceAlias,
      targetAlias: r.targetAlias === before.alias ? alias : r.targetAlias,
    })),
  });
}

/** Removes an entity and every relationship that uses its alias. */
export function removeEntity(draft: BuilderDraft, key: string): BuilderDraft {
  const removed = draft.entities.find((e) => e.key === key);
  if (!removed) return draft;
  return {
    ...draft,
    entities: draft.entities.filter((e) => e.key !== key),
    relationships: draft.relationships.filter(
      (r) => r.sourceAlias !== removed.alias && r.targetAlias !== removed.alias,
    ),
  };
}

export function addEntity(draft: BuilderDraft): BuilderDraft {
  let n = draft.entities.length + 1;
  while (draft.entities.some((e) => e.alias === `entity${n}`)) n += 1;
  return normalise({
    ...draft,
    entities: [
      ...draft.entities,
      {
        key: nextKey("e"),
        alias: `entity${n}`,
        typeName: `Entity${n}`,
        displayName: `Entity${n}`,
        mode: "create_or_match",
        existing: false,
        confidence: null,
        reason: "",
        properties: [],
      },
    ],
  });
}

export type PropertyOwner = "entity" | "relationship";

function editProperties(
  draft: BuilderDraft,
  owner: PropertyOwner,
  ownerKey: string,
  edit: (properties: DraftProperty[]) => DraftProperty[],
): BuilderDraft {
  if (owner === "entity") {
    return normalise({
      ...draft,
      entities: draft.entities.map((e) =>
        e.key === ownerKey ? { ...e, properties: edit(e.properties) } : e,
      ),
    });
  }
  return normalise({
    ...draft,
    relationships: draft.relationships.map((r) =>
      r.key === ownerKey ? { ...r, properties: edit(r.properties) } : r,
    ),
  });
}

export function updateProperty(
  draft: BuilderDraft,
  owner: PropertyOwner,
  ownerKey: string,
  propertyKey: string,
  patch: Partial<Pick<DraftProperty, "name" | "type" | "required" | "sourceField" | "identity">>,
): BuilderDraft {
  return editProperties(draft, owner, ownerKey, (properties) =>
    properties.map((p) => {
      if (p.key !== propertyKey) return p;
      if (!p.locked) return { ...p, ...patch };
      // Name, type and requiredness of existing properties are fixed.
      const free: Partial<DraftProperty> = {};
      if (patch.sourceField !== undefined) free.sourceField = patch.sourceField;
      if (patch.identity !== undefined) free.identity = patch.identity;
      return { ...p, ...free };
    }),
  );
}

export function addProperty(
  draft: BuilderDraft,
  owner: PropertyOwner,
  ownerKey: string,
): BuilderDraft {
  return editProperties(draft, owner, ownerKey, (properties) => [
    ...properties,
    {
      key: nextKey("p"),
      name: `property${properties.length + 1}`,
      type: "string",
      required: false,
      sourceField: null,
      identity: false,
      locked: false,
    },
  ]);
}

export function removeProperty(
  draft: BuilderDraft,
  owner: PropertyOwner,
  ownerKey: string,
  propertyKey: string,
): BuilderDraft {
  return editProperties(draft, owner, ownerKey, (properties) =>
    properties.filter((p) => p.key !== propertyKey || p.locked),
  );
}

export function addRelationship(draft: BuilderDraft): BuilderDraft {
  const source = draft.entities[0];
  const target = draft.entities[1] ?? draft.entities[0];
  return normalise({
    ...draft,
    relationships: [
      ...draft.relationships,
      {
        key: nextKey("r"),
        typeName: "RELATED_TO",
        sourceAlias: source?.alias ?? "",
        targetAlias: target?.alias ?? "",
        existing: false,
        confidence: null,
        reason: "",
        properties: [],
      },
    ],
  });
}

export function updateRelationship(
  draft: BuilderDraft,
  key: string,
  patch: Partial<Pick<DraftRelationship, "typeName" | "sourceAlias" | "targetAlias">>,
): BuilderDraft {
  return normalise({
    ...draft,
    relationships: draft.relationships.map((r) => (r.key === key ? { ...r, ...patch } : r)),
  });
}

export function removeRelationship(draft: BuilderDraft, key: string): BuilderDraft {
  return { ...draft, relationships: draft.relationships.filter((r) => r.key !== key) };
}

// --- validation and submission ------------------------------------------------

/** Problems that would make the server reject the draft. */
export function validateDraft(draft: BuilderDraft, fields: string[]): string[] {
  const errors: string[] = [];
  const known = new Set(fields);
  if (draft.entities.length === 0) errors.push("Add at least one entity.");
  const aliases = new Set<string>();
  const typeOfAlias = new Map<string, string>();
  for (const entity of draft.entities) {
    const label = entity.typeName || "an entity";
    if (!MACHINE_NAME.test(entity.typeName)) {
      errors.push(
        `Entity type "${entity.typeName}" must start with a letter and use only letters, digits and _.`,
      );
    }
    if (!entity.alias.trim()) errors.push(`Entity ${label} needs an alias.`);
    else if (aliases.has(entity.alias)) errors.push(`Alias "${entity.alias}" is used twice.`);
    aliases.add(entity.alias);
    typeOfAlias.set(entity.alias, entity.typeName);
    const names = new Set<string>();
    for (const property of entity.properties) {
      if (!MACHINE_NAME.test(property.name)) {
        errors.push(`Property "${property.name}" of ${label} has an invalid name.`);
      }
      if (names.has(property.name)) {
        errors.push(`Property "${property.name}" of ${label} appears twice.`);
      }
      names.add(property.name);
      if (property.sourceField !== null && !known.has(property.sourceField)) {
        errors.push(`Source field "${property.sourceField}" does not exist in the dataset.`);
      }
      if (property.identity && property.sourceField === null) {
        errors.push(`Identity property "${property.name}" of ${label} needs a source field.`);
      }
      if (entity.mode === "create_or_match" && property.required && property.sourceField === null) {
        errors.push(`Required property "${property.name}" of ${label} needs a source field.`);
      }
      if (entity.existing && !property.locked && property.required) {
        errors.push(`New property "${property.name}" of existing type ${label} must be optional.`);
      }
    }
    if (!entity.properties.some((p) => p.identity)) {
      errors.push(`Entity ${label} needs at least one identity property.`);
    }
  }
  const relationshipTypes = new Map<string, [string, string]>();
  for (const relationship of draft.relationships) {
    const label = relationship.typeName || "a relationship";
    if (!MACHINE_NAME.test(relationship.typeName)) {
      errors.push(`Relationship "${relationship.typeName}" has an invalid name.`);
    }
    const source = typeOfAlias.get(relationship.sourceAlias);
    const target = typeOfAlias.get(relationship.targetAlias);
    if (!source || !target) {
      errors.push(`Relationship ${label} must connect two entities of this draft.`);
      continue;
    }
    const existing = baseRelationshipType(draft.base, relationship.typeName);
    if (
      existing &&
      (existing.source_entity_type !== source || existing.target_entity_type !== target)
    ) {
      errors.push(
        `Existing relationship ${label} connects ${existing.source_entity_type} → ${existing.target_entity_type}.`,
      );
    }
    const seen = relationshipTypes.get(relationship.typeName);
    if (seen && (seen[0] !== source || seen[1] !== target)) {
      errors.push(`Relationship ${label} is used with different entity types.`);
    }
    relationshipTypes.set(relationship.typeName, [source, target]);
    for (const property of relationship.properties) {
      if (property.sourceField !== null && !known.has(property.sourceField)) {
        errors.push(`Source field "${property.sourceField}" does not exist in the dataset.`);
      }
    }
  }
  return [...new Set(errors)];
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

const mapped = (properties: DraftProperty[]): FieldMapping[] =>
  properties
    .filter((p) => p.sourceField !== null)
    .map((p) => ({ source_field: p.sourceField as string, property: p.name }));

/** The ontology and mapping to submit with the existing M4 endpoints. */
export function toSubmission(draft: BuilderDraft): {
  ontology: OntologyDraft;
  mapping: IngestionMapping;
} {
  const entityTypes = draft.base.entity_types.map((t) => ({ ...t, properties: [...t.properties] }));
  for (const entity of draft.entities) {
    const index = entityTypes.findIndex((t) => t.name === entity.typeName);
    if (index >= 0) {
      entityTypes[index] = {
        ...entityTypes[index],
        properties: mergeProperties(entityTypes[index].properties, entity.properties),
      };
    } else {
      entityTypes.push({
        name: entity.typeName,
        display_name: entity.displayName || entity.typeName,
        description: null,
        properties: mergeProperties([], entity.properties),
      });
    }
  }
  const typeOfAlias = new Map(draft.entities.map((e) => [e.alias, e.typeName]));
  const relationshipTypes = draft.base.relationship_types.map((t) => ({ ...t }));
  for (const relationship of draft.relationships) {
    const index = relationshipTypes.findIndex((t) => t.name === relationship.typeName);
    if (index >= 0) {
      relationshipTypes[index] = {
        ...relationshipTypes[index],
        properties: mergeProperties(relationshipTypes[index].properties, relationship.properties),
      };
    } else {
      relationshipTypes.push({
        name: relationship.typeName,
        source_entity_type: typeOfAlias.get(relationship.sourceAlias) ?? "",
        target_entity_type: typeOfAlias.get(relationship.targetAlias) ?? "",
        properties: mergeProperties([], relationship.properties),
      });
    }
  }
  return {
    ontology: { entity_types: entityTypes, relationship_types: relationshipTypes },
    mapping: {
      entities: draft.entities.map((entity) => ({
        alias: entity.alias,
        entity_type: entity.typeName,
        mode: entity.mode,
        identity: mapped(entity.properties.filter((p) => p.identity)),
        properties: entity.mode === "create_or_match" ? mapped(entity.properties) : [],
      })),
      relationships: draft.relationships.map((relationship) => ({
        relationship_type: relationship.typeName,
        source: relationship.sourceAlias,
        target: relationship.targetAlias,
        properties: mapped(relationship.properties),
      })),
    },
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

export type ConfidenceLevel = "high" | "medium" | "low";

/** Model confidence bands for display; never a guarantee. */
export function confidenceLevel(confidence: number): ConfidenceLevel {
  if (confidence >= 0.85) return "high";
  if (confidence >= 0.6) return "medium";
  return "low";
}
