/**
 * The client-side graph of the Graph Explorer: a Graphology multi-directed
 * graph holding the subgraph loaded so far (never the whole stored graph).
 *
 * Node key = EntityId, edge key = RelationshipId. Parallel relationships and
 * self-loops keep their own edges; the original API objects stay on the
 * attributes for the details panel. Nothing here touches Sigma or the DOM.
 */

import { MultiDirectedGraph } from "graphology";

import type {
  Entity,
  GraphPath,
  PropertyJson,
  PropertyType,
  Relationship,
  Subgraph,
} from "@/lib/api/types";
import { typeStyle } from "@/lib/graph/palette";

export interface NodeAttributes {
  /** The EntityId, also the node key. */
  id: string;
  entityType: string;
  properties: Entity["properties"];
  /** A presentation label; never used as an identifier. */
  label: string;
  color: string;
  size: number;
  x: number;
  y: number;
  /** The entity as MindGraph Core returned it. */
  entity: Entity;
}

/** How Sigma draws an edge: straight, or curved to separate parallel ones. */
export type EdgeShape = "straight" | "curved";

export interface EdgeAttributes {
  /** The RelationshipId, also the edge key. */
  id: string;
  relationshipType: string;
  source: string;
  target: string;
  properties: Relationship["properties"];
  /** The relationship as MindGraph Core returned it. */
  relationship: Relationship;
  type: EdgeShape;
  curvature: number;
  size: number;
  /** Source and target are the same entity (kept, but not drawn by Sigma). */
  selfLoop: boolean;
}

export type ExplorerGraph = MultiDirectedGraph<NodeAttributes, EdgeAttributes>;

export function createExplorerGraph(): ExplorerGraph {
  return new MultiDirectedGraph<NodeAttributes, EdgeAttributes>({ allowSelfLoops: true });
}

/** A graph payload that does not have the documented shape. */
export class MalformedGraphError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "MalformedGraphError";
  }
}

const PROPERTY_TYPES = new Set<PropertyType>([
  "string",
  "integer",
  "float",
  "boolean",
  "datetime",
  "string_array",
]);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function text(value: unknown, what: string): string {
  if (typeof value !== "string" || value.length === 0) {
    throw new MalformedGraphError(`${what} must be a non-empty string`);
  }
  return value;
}

function properties(value: unknown, types: unknown, what: string) {
  if (!isRecord(value) || !isRecord(types)) {
    throw new MalformedGraphError(`${what} needs properties and property_types objects`);
  }
  for (const [key, type] of Object.entries(types)) {
    if (!PROPERTY_TYPES.has(type as PropertyType)) {
      throw new MalformedGraphError(`${what} has an unknown type for property ${key}`);
    }
  }
  return {
    properties: value as Record<string, PropertyJson>,
    property_types: types as Record<string, PropertyType>,
  };
}

function graphOf(value: unknown, graphId: string | undefined, what: string): string {
  const id = text(value, `${what} graph_id`);
  if (graphId !== undefined && id !== graphId) {
    throw new MalformedGraphError(`${what} belongs to another graph`);
  }
  return id;
}

/** Checks one API entity (with `graphId`, of that graph only). */
export function parseEntity(value: unknown, graphId?: string): Entity {
  if (!isRecord(value)) throw new MalformedGraphError("an entity must be an object");
  const id = text(value.id, "entity id");
  return {
    id,
    graph_id: graphOf(value.graph_id, graphId, `entity ${id}`),
    entity_type: text(value.entity_type, `entity ${id} entity_type`),
    ...properties(value.properties, value.property_types, `entity ${id}`),
  };
}

/** Checks one API relationship (with `graphId`, of that graph only). */
export function parseRelationship(value: unknown, graphId?: string): Relationship {
  if (!isRecord(value)) throw new MalformedGraphError("a relationship must be an object");
  const id = text(value.id, "relationship id");
  return {
    id,
    graph_id: graphOf(value.graph_id, graphId, `relationship ${id}`),
    relationship_type: text(value.relationship_type, `relationship ${id} type`),
    source_id: text(value.source_id, `relationship ${id} source_id`),
    target_id: text(value.target_id, `relationship ${id} target_id`),
    ...properties(value.properties, value.property_types, `relationship ${id}`),
  };
}

function list(value: unknown, what: string): unknown[] {
  if (!Array.isArray(value)) throw new MalformedGraphError(`${what} must be an array`);
  return value;
}

/**
 * Checks an API subgraph (neighbors, subgraph or path payload). With
 * `graphId`, anything of another graph is rejected.
 *
 * @throws MalformedGraphError
 */
export function parseSubgraph(payload: unknown, graphId?: string): Subgraph {
  if (!isRecord(payload)) throw new MalformedGraphError("a subgraph must be an object");
  return {
    entities: list(payload.entities, "entities").map((e) => parseEntity(e, graphId)),
    relationships: list(payload.relationships, "relationships").map((r) =>
      parseRelationship(r, graphId),
    ),
  };
}

/** Checks a path payload: the path's chain of entities and relationships. */
export function parsePath(payload: unknown, graphId?: string): GraphPath {
  if (!isRecord(payload) || typeof payload.found !== "boolean") {
    throw new MalformedGraphError("a path needs a boolean found");
  }
  const { entities, relationships } = parseSubgraph(payload, graphId);
  const hops = payload.hops;
  if (typeof hops !== "number" || !Number.isInteger(hops) || hops < 0) {
    throw new MalformedGraphError("hops must be a non-negative integer");
  }
  if (payload.found && (relationships.length !== hops || entities.length !== hops + 1)) {
    throw new MalformedGraphError("a path needs hops + 1 entities and hops relationships");
  }
  relationships.forEach((relationship, index) => {
    const ends = [entities[index].id, entities[index + 1].id];
    if (!ends.includes(relationship.source_id) || !ends.includes(relationship.target_id)) {
      throw new MalformedGraphError(`path relationship ${relationship.id} is out of order`);
    }
  });
  return { found: payload.found, hops, entities, relationships };
}

// --- presentation --------------------------------------------------------------------------

const NAME_KEYS = ["name", "display_name", "title", "full_name", "label"];
const IDENTIFIER_KEYS = ["id", "code", "number", "key", "external_id"];
export const MAX_LABEL_LENGTH = 32;

/** `019a2b3c-…-9f1e` → `019a2b3c…9f1e`. */
export function shortId(id: string): string {
  return id.length > 14 ? `${id.slice(0, 8)}…${id.slice(-4)}` : id;
}

export function truncate(value: string, max = MAX_LABEL_LENGTH): string {
  return value.length > max ? `${value.slice(0, max - 1)}…` : value;
}

function snakeCase(name: string): string {
  return name
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/[\s-]+/g, "_")
    .toLowerCase();
}

function scalar(value: PropertyJson | undefined): string | null {
  if (typeof value === "string") return value.trim() || null;
  if (typeof value === "number") return String(value);
  return null;
}

/**
 * A readable label: a name-like property, else a domain identifier
 * (`customer_id` for a Customer, any `*_id`, ...), else the short EntityId.
 */
export function entityLabel(entity: Entity): string {
  const keys = Object.keys(entity.properties);
  const lookup = (wanted: string) => keys.find((key) => key.toLowerCase() === wanted);
  for (const wanted of NAME_KEYS) {
    const key = lookup(wanted);
    const value = key ? scalar(entity.properties[key]) : null;
    if (value) return truncate(value);
  }
  const own = lookup(`${snakeCase(entity.entity_type)}_id`);
  const identifiers = [
    ...(own ? [own] : []),
    ...keys.filter((key) => key.toLowerCase().endsWith("_id")),
    ...IDENTIFIER_KEYS.flatMap((wanted) => lookup(wanted) ?? []),
  ];
  for (const key of identifiers) {
    const value = scalar(entity.properties[key]);
    if (value) return truncate(value);
  }
  return shortId(entity.id);
}

/** A property value as text (React escapes it; nothing is rendered as HTML). */
export function formatValue(value: PropertyJson | undefined): string {
  if (value === undefined) return "";
  if (Array.isArray(value)) return value.join(", ");
  return String(value);
}

/** Node attributes describing an entity (position and size are set elsewhere). */
export function entityAttributes(entity: Entity) {
  return {
    id: entity.id,
    entityType: entity.entity_type,
    properties: entity.properties,
    label: entityLabel(entity),
    color: typeStyle(entity.entity_type).hex,
    entity,
  };
}

/** Edge attributes describing a relationship (shape is set by the merge). */
export function relationshipAttributes(relationship: Relationship) {
  return {
    id: relationship.id,
    relationshipType: relationship.relationship_type,
    source: relationship.source_id,
    target: relationship.target_id,
    properties: relationship.properties,
    relationship,
    selfLoop: relationship.source_id === relationship.target_id,
  };
}
