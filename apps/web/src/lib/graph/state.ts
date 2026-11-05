/**
 * The investigation state of the Graph Explorer: what is selected, hidden,
 * filtered or highlighted. It is temporary UI state, kept apart from the
 * loaded graph data and never persisted (saved views are a later
 * milestone). Everything here is pure.
 *
 * Highlighting is generic (any entities and relationships): shortest
 * paths use it in blue, and Ask MindGraph (M7) uses it in orange for the
 * evidence of an answer.
 */

import type { Direction, GraphPath } from "@/lib/api/types";
import type { LayoutMode } from "@/lib/graph/layout";
import type { EdgeAttributes, ExplorerGraph, NodeAttributes } from "@/lib/graph/transform";

/** A path request and what MindGraph Core answered (found or not). */
export interface ActivePath {
  sourceId: string;
  targetId: string;
  direction: Direction;
  maxDepth: number;
  path: GraphPath;
}

/** Why entities are highlighted: a shortest path, or an answer's evidence. */
export type HighlightTone = "path" | "evidence";

export interface InvestigationState {
  selectedEntityId: string | null;
  selectedRelationshipId: string | null;
  /** The entity the camera was last brought to. */
  focusedEntityId: string | null;
  hiddenEntityIds: ReadonlySet<string>;
  hiddenRelationshipIds: ReadonlySet<string>;
  highlightedEntityIds: ReadonlySet<string>;
  highlightedRelationshipIds: ReadonlySet<string>;
  highlightTone: HighlightTone;
  activePath: ActivePath | null;
  /** Entity types switched off in the view. */
  entityTypeFilters: ReadonlySet<string>;
  /** Relationship types switched off in the view. */
  relationshipTypeFilters: ReadonlySet<string>;
  layoutMode: LayoutMode;
}

const NONE: ReadonlySet<string> = new Set();

export const initialInvestigation: InvestigationState = {
  selectedEntityId: null,
  selectedRelationshipId: null,
  focusedEntityId: null,
  hiddenEntityIds: NONE,
  hiddenRelationshipIds: NONE,
  highlightedEntityIds: NONE,
  highlightedRelationshipIds: NONE,
  highlightTone: "path",
  activePath: null,
  entityTypeFilters: NONE,
  relationshipTypeFilters: NONE,
  layoutMode: "force",
};

export type InvestigationAction =
  | { type: "selectEntity"; id: string }
  | { type: "selectRelationship"; id: string }
  | { type: "clearSelection" }
  | { type: "focusEntity"; id: string }
  | { type: "hideEntity"; id: string }
  /** Hides several entities at once (collapsing a neighbourhood). */
  | { type: "hideEntities"; ids: string[] }
  /** Shows one hidden entity again (e.g. when it is opened from a search). */
  | { type: "revealEntity"; id: string }
  | { type: "hideRelationship"; id: string }
  | { type: "showHidden" }
  /** Entities and relationships removed from the loaded graph. */
  | { type: "forget"; entityIds: string[]; relationshipIds: string[] }
  | { type: "toggleEntityType"; name: string }
  | { type: "toggleRelationshipType"; name: string }
  | { type: "showPath"; path: ActivePath }
  | { type: "highlight"; entityIds: string[]; relationshipIds: string[] }
  /** Highlights the evidence of an answer (or one reference of it). */
  | { type: "showEvidence"; entityIds: string[]; relationshipIds: string[] }
  | { type: "resetHighlight" }
  | { type: "setLayout"; mode: LayoutMode }
  | { type: "reset" };

function toggled(set: ReadonlySet<string>, value: string): ReadonlySet<string> {
  const next = new Set(set);
  if (!next.delete(value)) next.add(value);
  return next;
}

function added(set: ReadonlySet<string>, value: string): ReadonlySet<string> {
  return set.has(value) ? set : new Set(set).add(value);
}

function without(set: ReadonlySet<string>, values: ReadonlySet<string>): ReadonlySet<string> {
  return [...set].some((value) => values.has(value))
    ? new Set([...set].filter((value) => !values.has(value)))
    : set;
}

export function investigationReducer(
  state: InvestigationState,
  action: InvestigationAction,
): InvestigationState {
  switch (action.type) {
    case "selectEntity":
      return {
        ...state,
        selectedEntityId: action.id,
        selectedRelationshipId: null,
        focusedEntityId: action.id,
      };
    case "selectRelationship":
      return { ...state, selectedEntityId: null, selectedRelationshipId: action.id };
    case "clearSelection":
      return { ...state, selectedEntityId: null, selectedRelationshipId: null };
    case "focusEntity":
      return { ...state, focusedEntityId: action.id };
    case "hideEntity":
      return {
        ...state,
        hiddenEntityIds: added(state.hiddenEntityIds, action.id),
        selectedEntityId: state.selectedEntityId === action.id ? null : state.selectedEntityId,
        focusedEntityId: state.focusedEntityId === action.id ? null : state.focusedEntityId,
      };
    case "hideEntities": {
      if (action.ids.length === 0) return state;
      const ids = new Set(action.ids);
      return {
        ...state,
        hiddenEntityIds: new Set([...state.hiddenEntityIds, ...ids]),
        selectedEntityId:
          state.selectedEntityId && ids.has(state.selectedEntityId) ? null : state.selectedEntityId,
        focusedEntityId:
          state.focusedEntityId && ids.has(state.focusedEntityId) ? null : state.focusedEntityId,
      };
    }
    case "revealEntity":
      return { ...state, hiddenEntityIds: without(state.hiddenEntityIds, new Set([action.id])) };
    case "hideRelationship":
      return {
        ...state,
        hiddenRelationshipIds: added(state.hiddenRelationshipIds, action.id),
        selectedRelationshipId:
          state.selectedRelationshipId === action.id ? null : state.selectedRelationshipId,
      };
    case "showHidden":
      return { ...state, hiddenEntityIds: NONE, hiddenRelationshipIds: NONE };
    case "forget": {
      const entities = new Set(action.entityIds);
      const relationships = new Set(action.relationshipIds);
      const pathGone = state.activePath?.path.entities.some((e) => entities.has(e.id)) ?? false;
      return {
        ...state,
        selectedEntityId:
          state.selectedEntityId && entities.has(state.selectedEntityId)
            ? null
            : state.selectedEntityId,
        selectedRelationshipId:
          state.selectedRelationshipId && relationships.has(state.selectedRelationshipId)
            ? null
            : state.selectedRelationshipId,
        focusedEntityId:
          state.focusedEntityId && entities.has(state.focusedEntityId)
            ? null
            : state.focusedEntityId,
        hiddenEntityIds: without(state.hiddenEntityIds, entities),
        hiddenRelationshipIds: without(state.hiddenRelationshipIds, relationships),
        highlightedEntityIds: pathGone ? NONE : without(state.highlightedEntityIds, entities),
        highlightedRelationshipIds: pathGone
          ? NONE
          : without(state.highlightedRelationshipIds, relationships),
        activePath: pathGone ? null : state.activePath,
      };
    }
    case "toggleEntityType":
      return { ...state, entityTypeFilters: toggled(state.entityTypeFilters, action.name) };
    case "toggleRelationshipType":
      return {
        ...state,
        relationshipTypeFilters: toggled(state.relationshipTypeFilters, action.name),
      };
    case "showPath": {
      const { path } = action.path;
      return {
        ...state,
        activePath: action.path,
        highlightTone: "path",
        highlightedEntityIds: new Set(path.found ? path.entities.map((e) => e.id) : []),
        highlightedRelationshipIds: new Set(path.found ? path.relationships.map((r) => r.id) : []),
      };
    }
    case "highlight":
      return {
        ...state,
        activePath: null,
        highlightTone: "path",
        highlightedEntityIds: new Set(action.entityIds),
        highlightedRelationshipIds: new Set(action.relationshipIds),
      };
    case "showEvidence":
      return {
        ...state,
        activePath: null,
        highlightTone: "evidence",
        highlightedEntityIds: new Set(action.entityIds),
        highlightedRelationshipIds: new Set(action.relationshipIds),
      };
    case "resetHighlight":
      return {
        ...state,
        activePath: null,
        highlightedEntityIds: NONE,
        highlightedRelationshipIds: NONE,
      };
    case "setLayout":
      return { ...state, layoutMode: action.mode };
    case "reset":
      return { ...initialInvestigation, layoutMode: state.layoutMode };
  }
}

// --- visibility ----------------------------------------------------------------------------

export function isEntityVisible(
  state: InvestigationState,
  id: string,
  attributes: Pick<NodeAttributes, "entityType">,
): boolean {
  return !state.hiddenEntityIds.has(id) && !state.entityTypeFilters.has(attributes.entityType);
}

export function isRelationshipVisible(
  graph: ExplorerGraph,
  state: InvestigationState,
  edge: string,
): boolean {
  const { relationshipType } = graph.getEdgeAttributes(edge);
  if (
    state.hiddenRelationshipIds.has(edge) ||
    state.relationshipTypeFilters.has(relationshipType)
  ) {
    return false;
  }
  return [graph.source(edge), graph.target(edge)].every((node) =>
    isEntityVisible(state, node, graph.getNodeAttributes(node)),
  );
}

/** The loaded entities currently shown (not hidden, not filtered out). */
export function visibleEntityIds(graph: ExplorerGraph, state: InvestigationState): Set<string> {
  return new Set(graph.filterNodes((id, attributes) => isEntityVisible(state, id, attributes)));
}

// --- appearance (Sigma reducers) ---------------------------------------------------------------

export interface DisplayInput {
  state: InvestigationState;
  hoveredEntityId: string | null;
  hoveredRelationshipId: string | null;
  /** The camera is close enough for every relationship label to be legible. */
  showAllEdgeLabels: boolean;
}

/** What stands out; everything else is dimmed (but stays visible). */
export interface Emphasis {
  entities: ReadonlySet<string>;
  relationships: ReadonlySet<string>;
}

function around(graph: ExplorerGraph, state: InvestigationState, node: string): Emphasis {
  const entities = new Set([node]);
  const relationships = new Set<string>();
  graph.forEachEdge(node, (edge, _, source, target) => {
    if (!isRelationshipVisible(graph, state, edge)) return;
    relationships.add(edge);
    entities.add(source).add(target);
  });
  return { entities, relationships };
}

/** Hover first, then a highlight (path), then the selection. */
export function emphasis(graph: ExplorerGraph, input: DisplayInput): Emphasis | null {
  const { state, hoveredEntityId } = input;
  if (hoveredEntityId && graph.hasNode(hoveredEntityId)) {
    return around(graph, state, hoveredEntityId);
  }
  if (state.highlightedEntityIds.size || state.highlightedRelationshipIds.size) {
    return {
      entities: state.highlightedEntityIds,
      relationships: state.highlightedRelationshipIds,
    };
  }
  if (state.selectedEntityId && graph.hasNode(state.selectedEntityId)) {
    return around(graph, state, state.selectedEntityId);
  }
  if (state.selectedRelationshipId && graph.hasEdge(state.selectedRelationshipId)) {
    const edge = state.selectedRelationshipId;
    return {
      entities: new Set([graph.source(edge), graph.target(edge)]),
      relationships: new Set([edge]),
    };
  }
  return null;
}

export const COLORS = {
  edge: "#4a5360",
  edgeDim: "#1c2129",
  edgeFocus: "#9aa2ad",
  path: "#3d8bff",
  /** Evidence of an Ask MindGraph answer (the design's risk orange). */
  evidence: "#ff7a45",
  /** Appended to a node colour to dim it. */
  dimAlpha: "33",
};

export interface NodeAppearance {
  hidden: boolean;
  color: string;
  label: string | null;
  size: number;
  zIndex: number;
  forceLabel: boolean;
  highlighted: boolean;
}

export function nodeAppearance(
  id: string,
  attributes: NodeAttributes,
  input: DisplayInput,
  focus: Emphasis | null,
): NodeAppearance {
  const { state } = input;
  const selected = state.selectedEntityId === id;
  const strong = selected || !focus || focus.entities.has(id);
  return {
    hidden: !isEntityVisible(state, id, attributes),
    color: strong ? attributes.color : `${attributes.color}${COLORS.dimAlpha}`,
    label: strong ? attributes.label : null,
    size: selected ? attributes.size * 1.3 : attributes.size,
    zIndex: selected ? 3 : focus?.entities.has(id) ? 2 : strong ? 1 : 0,
    forceLabel: selected || (focus?.entities.has(id) ?? false),
    highlighted: selected,
  };
}

export interface EdgeAppearance {
  hidden: boolean;
  color: string;
  label: string | null;
  size: number;
  zIndex: number;
}

export function edgeAppearance(
  graph: ExplorerGraph,
  edge: string,
  attributes: EdgeAttributes,
  input: DisplayInput,
  focus: Emphasis | null,
): EdgeAppearance {
  const { state } = input;
  const hidden = !isRelationshipVisible(graph, state, edge);
  const selected = state.selectedRelationshipId === edge || input.hoveredRelationshipId === edge;
  const highlighted = state.highlightedRelationshipIds.has(edge);
  const strong = selected || (focus?.relationships.has(edge) ?? false);
  const label = attributes.relationshipType;
  if (highlighted && strong && state.highlightTone === "evidence") {
    return { hidden, color: COLORS.evidence, label, size: 2.5, zIndex: 3 };
  }
  if (selected || (highlighted && strong)) {
    return { hidden, color: COLORS.path, label, size: 2.5, zIndex: 3 };
  }
  if (strong) return { hidden, color: COLORS.edgeFocus, label, size: 1.6, zIndex: 2 };
  if (focus) return { hidden, color: COLORS.edgeDim, label: null, size: 1, zIndex: 0 };
  return {
    hidden,
    color: COLORS.edge,
    label: input.showAllEdgeLabels ? label : null,
    size: 1,
    zIndex: 1,
  };
}

// --- details ---------------------------------------------------------------------------------

export interface RelationshipGroup {
  key: string;
  relationshipType: string;
  /** Seen from the entity: it is the source (`out`), the target (`in`) or both (`self`). */
  direction: "out" | "in" | "self";
  items: { relationshipId: string; entityId: string; label: string; entityType: string }[];
}

/**
 * The visible loaded relationships of an entity, grouped by type and
 * direction. Only what the view holds: never the entity's full degree.
 */
export function relationshipGroups(
  graph: ExplorerGraph,
  state: InvestigationState,
  entityId: string,
): RelationshipGroup[] {
  if (!graph.hasNode(entityId)) return [];
  const groups = new Map<string, RelationshipGroup>();
  graph.forEachEdge(entityId, (edge, attributes, source, target) => {
    if (!isRelationshipVisible(graph, state, edge)) return;
    const direction = source === target ? "self" : source === entityId ? "out" : "in";
    const other = direction === "in" ? source : target;
    const key = `${direction}:${attributes.relationshipType}`;
    const group = groups.get(key) ?? {
      key,
      relationshipType: attributes.relationshipType,
      direction,
      items: [],
    };
    const { label, entityType } = graph.getNodeAttributes(other);
    group.items.push({ relationshipId: edge, entityId: other, label, entityType });
    groups.set(key, group);
  });
  return [...groups.values()].sort(
    (a, b) =>
      a.relationshipType.localeCompare(b.relationshipType) ||
      a.direction.localeCompare(b.direction),
  );
}

/**
 * The neighbours an entity can collapse: visible neighbours whose every
 * visible relationship leads to that entity. Neighbours that are also
 * connected to another visible entity are shared and stay.
 */
export function collapsibleNeighbors(
  graph: ExplorerGraph,
  state: InvestigationState,
  entityId: string,
): string[] {
  if (!graph.hasNode(entityId)) return [];
  const result: string[] = [];
  graph.forEachNeighbor(entityId, (neighbor, attributes) => {
    if (neighbor === entityId || !isEntityVisible(state, neighbor, attributes)) return;
    let shared = false;
    graph.forEachEdge(neighbor, (edge, _, source, target) => {
      if (shared || !isRelationshipVisible(graph, state, edge)) return;
      const other = source === neighbor ? target : source;
      if (other !== entityId && other !== neighbor) shared = true;
    });
    if (!shared) result.push(neighbor);
  });
  return result.sort();
}
