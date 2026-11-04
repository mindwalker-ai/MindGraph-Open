/**
 * Merging API subgraphs into the loaded Graphology graph.
 *
 * `mergeSubgraph` validates the whole payload first (a malformed payload
 * changes nothing), then adds what is missing: entities by EntityId,
 * relationships by RelationshipId. Existing nodes keep their position and
 * size; new nodes are placed around the anchor so the view does not jump.
 * The view budget caps what the browser holds; it is a UI limit, not a
 * statement about the stored graph.
 */

import type { Subgraph } from "@/lib/api/types";
import {
  type ExplorerGraph,
  MalformedGraphError,
  entityAttributes,
  parseSubgraph,
  relationshipAttributes,
} from "@/lib/graph/transform";

export interface GraphBudget {
  maxEntities: number;
  maxRelationships: number;
}

/** At most this much is loaded in the browser at once. */
export const VIEW_BUDGET: GraphBudget = { maxEntities: 500, maxRelationships: 1500 };

export interface MergeOptions {
  budget?: GraphBudget;
  /** The entity the subgraph was loaded around; new nodes are placed near it. */
  anchor?: string | null;
  /** Reject anything that belongs to another graph. */
  graphId?: string;
}

export interface MergeResult {
  addedEntityIds: string[];
  addedRelationshipIds: string[];
  /** Entities left out because the view budget is reached. */
  skippedEntities: number;
  /** Relationships left out (budget, or an endpoint left out). */
  skippedRelationships: number;
}

const GOLDEN_ANGLE = Math.PI * (3 - Math.sqrt(5));
const SPACING = 12;
const MIN_SIZE = 4;
const MAX_SIZE = 14;
const CURVE_STEP = 0.45;

/** Size by loaded degree, bounded (it is not the entity's degree in the store). */
export function nodeSize(degree: number): number {
  return Math.min(MAX_SIZE, MIN_SIZE + 2.2 * Math.log2(1 + degree));
}

function extent(graph: ExplorerGraph) {
  let minX = Infinity;
  let maxX = -Infinity;
  let minY = Infinity;
  let maxY = -Infinity;
  graph.forEachNode((_, { x, y }) => {
    minX = Math.min(minX, x);
    maxX = Math.max(maxX, x);
    minY = Math.min(minY, y);
    maxY = Math.max(maxY, y);
  });
  return { minX, maxX, minY, maxY };
}

/** Where a cluster of new nodes goes: the anchor, else beside what is loaded. */
function clusterCentre(graph: ExplorerGraph, anchor: string | null | undefined) {
  if (anchor && graph.hasNode(anchor)) {
    const { x, y } = graph.getNodeAttributes(anchor);
    return { centre: { x, y }, first: 1 };
  }
  if (graph.order === 0) return { centre: { x: 0, y: 0 }, first: 0 };
  const { maxX, minY, maxY } = extent(graph);
  return { centre: { x: maxX + SPACING * 4, y: (minY + maxY) / 2 }, first: 0 };
}

function spiral(centre: { x: number; y: number }, index: number) {
  if (index === 0) return { ...centre };
  const radius = SPACING * Math.sqrt(index);
  const angle = index * GOLDEN_ANGLE;
  return { x: centre.x + radius * Math.cos(angle), y: centre.y + radius * Math.sin(angle) };
}

function pairKey(a: string, b: string): string {
  return a < b ? `${a}\u0000${b}` : `${b}\u0000${a}`;
}

/**
 * Gives the relationships between two entities distinct curvatures, so
 * parallel (and opposite) relationships stay visible as separate edges.
 */
export function shapePair(graph: ExplorerGraph, a: string, b: string): void {
  if (!graph.hasNode(a) || !graph.hasNode(b)) return;
  const edges = a === b ? graph.edges(a, a) : [...graph.edges(a, b), ...graph.edges(b, a)];
  const sorted = [...new Set(edges)].sort();
  const [low] = a < b ? [a, b] : [b, a];
  sorted.forEach((edge, index) => {
    const offset = sorted.length === 1 ? 0 : index - (sorted.length - 1) / 2;
    const forward = graph.source(edge) === low;
    const curvature = a === b ? 0 : offset * CURVE_STEP * (forward ? 1 : -1);
    graph.mergeEdgeAttributes(edge, { type: curvature === 0 ? "straight" : "curved", curvature });
  });
}

function resize(graph: ExplorerGraph, nodes: Iterable<string>): void {
  for (const node of nodes) {
    if (graph.hasNode(node)) graph.setNodeAttribute(node, "size", nodeSize(graph.degree(node)));
  }
}

/**
 * Adds a subgraph to the loaded graph (in place) and reports what changed.
 *
 * @throws MalformedGraphError before changing anything when the payload is
 *   malformed, belongs to another graph, or refers to unknown entities.
 */
export function mergeSubgraph(
  graph: ExplorerGraph,
  incoming: Subgraph | unknown,
  options: MergeOptions = {},
): MergeResult {
  const subgraph = parseSubgraph(incoming, options.graphId);
  const budget = options.budget ?? VIEW_BUDGET;
  const known = new Set(subgraph.entities.map((entity) => entity.id));
  for (const relationship of subgraph.relationships) {
    for (const end of [relationship.source_id, relationship.target_id]) {
      if (!known.has(end) && !graph.hasNode(end)) {
        throw new MalformedGraphError(
          `relationship ${relationship.id} refers to an entity that is not in the payload`,
        );
      }
    }
  }

  const result: MergeResult = {
    addedEntityIds: [],
    addedRelationshipIds: [],
    skippedEntities: 0,
    skippedRelationships: 0,
  };
  const touched = new Set<string>();

  // The anchor first, so it lands at the centre of a new cluster.
  const entities = [...new Map(subgraph.entities.map((e) => [e.id, e])).values()].sort(
    (a, b) => Number(b.id === options.anchor) - Number(a.id === options.anchor),
  );
  const { centre, first } = clusterCentre(graph, options.anchor);
  for (const entity of entities) {
    if (graph.hasNode(entity.id)) {
      graph.mergeNodeAttributes(entity.id, entityAttributes(entity));
    } else if (graph.order < budget.maxEntities) {
      const position = spiral(centre, first + result.addedEntityIds.length);
      graph.addNode(entity.id, { ...entityAttributes(entity), ...position, size: MIN_SIZE });
      result.addedEntityIds.push(entity.id);
      touched.add(entity.id);
    } else {
      result.skippedEntities += 1;
    }
  }

  const relationships = new Map(subgraph.relationships.map((r) => [r.id, r]));
  const pairs = new Map<string, [string, string]>();
  for (const relationship of relationships.values()) {
    const { id, source_id: source, target_id: target } = relationship;
    if (graph.hasEdge(id)) {
      graph.mergeEdgeAttributes(id, relationshipAttributes(relationship));
    } else if (
      graph.hasNode(source) &&
      graph.hasNode(target) &&
      graph.size < budget.maxRelationships
    ) {
      graph.addDirectedEdgeWithKey(id, source, target, {
        ...relationshipAttributes(relationship),
        type: "straight",
        curvature: 0,
        size: 1,
      });
      result.addedRelationshipIds.push(id);
      touched.add(source).add(target);
      pairs.set(pairKey(source, target), [source, target]);
    } else {
      result.skippedRelationships += 1;
    }
  }

  resize(graph, touched);
  for (const [a, b] of pairs.values()) shapePair(graph, a, b);
  return result;
}

/**
 * Removes entities (and their relationships) from the loaded graph; the
 * stored graph is never touched. Returns the removed relationship ids.
 */
export function removeEntities(graph: ExplorerGraph, ids: Iterable<string>): string[] {
  const removed: string[] = [];
  const neighbours = new Set<string>();
  for (const id of ids) {
    if (!graph.hasNode(id)) continue;
    graph.forEachEdge(id, (edge, _, source, target) => {
      removed.push(edge);
      neighbours.add(source).add(target);
    });
    graph.dropNode(id);
  }
  resize(graph, neighbours);
  return removed;
}
