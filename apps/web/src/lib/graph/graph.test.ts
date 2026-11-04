import { describe, expect, it } from "vitest";

import type { Entity, GraphPath, Ontology, PropertyJson, Relationship } from "@/lib/api/types";
import { circularLayout, forceLayout, settleNewNodes } from "@/lib/graph/layout";
import { mergeSubgraph, nodeSize, removeEntities } from "@/lib/graph/merge";
import { buildSearchQuery, propertyChoices, searchSummary } from "@/lib/graph/search";
import {
  type DisplayInput,
  type InvestigationState,
  COLORS,
  collapsibleNeighbors,
  edgeAppearance,
  emphasis,
  initialInvestigation,
  investigationReducer,
  nodeAppearance,
  relationshipGroups,
  visibleEntityIds,
} from "@/lib/graph/state";
import {
  MalformedGraphError,
  createExplorerGraph,
  entityLabel,
  parsePath,
  shortId,
} from "@/lib/graph/transform";

const G = "graph-1";

function entity(id: string, type: string, properties: Record<string, PropertyJson> = {}): Entity {
  const property_types = Object.fromEntries(
    Object.entries(properties).map(([key, value]) => [
      key,
      typeof value === "number" ? "integer" : Array.isArray(value) ? "string_array" : "string",
    ]),
  ) as Entity["property_types"];
  return { id, graph_id: G, entity_type: type, properties, property_types };
}

function rel(id: string, type: string, source: string, target: string): Relationship {
  return {
    id,
    graph_id: G,
    relationship_type: type,
    source_id: source,
    target_id: target,
    properties: {},
    property_types: {},
  };
}

const alice = entity("e-alice", "Customer", { customer_id: "C001", name: "Alice Tan" });
const budi = entity("e-budi", "Customer", { customer_id: "C002", name: "Budi" });
const account = entity("e-acc", "Account", { account_id: "A100" });
const device = entity("e-dev", "Device", { device_id: "DEVICE-921" });
const owns = rel("r-owns", "OWNS", alice.id, account.id);
const usesA = rel("r-uses-a", "USES", alice.id, device.id);
const usesB = rel("r-uses-b", "USES", budi.id, device.id);

function loaded() {
  const graph = createExplorerGraph();
  mergeSubgraph(
    graph,
    { entities: [alice, account, device], relationships: [owns, usesA] },
    { anchor: alice.id, graphId: G },
  );
  return graph;
}

function display(state: InvestigationState, hovered: string | null = null): DisplayInput {
  return {
    state,
    hoveredEntityId: hovered,
    hoveredRelationshipId: null,
    showAllEdgeLabels: false,
  };
}

describe("subgraph to Graphology", () => {
  it("keys nodes by EntityId and edges by RelationshipId, keeping the API data", () => {
    const graph = loaded();
    expect(graph.order).toBe(3);
    expect(graph.size).toBe(2);
    const node = graph.getNodeAttributes(alice.id);
    expect(node).toMatchObject({
      id: alice.id,
      entityType: "Customer",
      label: "Alice Tan",
      color: "#5b9bff",
      entity: alice,
    });
    expect(node.properties).toBe(alice.properties);
    expect(graph.source(owns.id)).toBe(alice.id);
    expect(graph.target(owns.id)).toBe(account.id);
    expect(graph.getEdgeAttributes(owns.id)).toMatchObject({
      relationshipType: "OWNS",
      source: alice.id,
      target: account.id,
      relationship: owns,
      type: "straight",
    });
    // The anchor sits at the centre of a new cluster.
    expect(node).toMatchObject({ x: 0, y: 0 });
  });

  it("labels entities by name, then domain identifier, then short id", () => {
    expect(entityLabel(alice)).toBe("Alice Tan");
    expect(entityLabel(account)).toBe("A100");
    expect(entityLabel(entity("e", "IPAddress", { ip_address_id: "10.0.0.1", x_id: "z" }))).toBe(
      "10.0.0.1",
    );
    expect(entityLabel(entity("e", "Thing", { Title: "Big report" }))).toBe("Big report");
    expect(entityLabel(entity("e", "Thing", { score: 42 }))).toBe("e");
    const id = "019a2b3c-0000-7000-8000-00000000f1e2";
    expect(entityLabel(entity(id, "Thing", {}))).toBe(shortId(id));
    expect(shortId(id)).toBe("019a2b3c…f1e2");
    expect(entityLabel(entity("e", "Thing", { name: "x".repeat(80) }))).toHaveLength(32);
  });
});

describe("mergeSubgraph", () => {
  it("deduplicates entities and relationships and keeps positions", () => {
    const graph = loaded();
    graph.mergeNodeAttributes(alice.id, { x: 50, y: -20 });
    const result = mergeSubgraph(
      graph,
      { entities: [alice, device, budi, budi], relationships: [usesA, usesB, usesB] },
      { anchor: device.id },
    );
    expect(result.addedEntityIds).toEqual([budi.id]);
    expect(result.addedRelationshipIds).toEqual([usesB.id]);
    expect(graph.order).toBe(4);
    expect(graph.size).toBe(3);
    expect(graph.getNodeAttributes(alice.id)).toMatchObject({ x: 50, y: -20 });
  });

  it("sizes nodes by loaded degree, within bounds", () => {
    const graph = loaded();
    expect(graph.getNodeAttribute(alice.id, "size")).toBeCloseTo(nodeSize(2));
    expect(nodeSize(0)).toBe(4);
    expect(nodeSize(10_000)).toBe(14);
  });

  it("keeps parallel relationships as separately curved edges", () => {
    const graph = loaded();
    const again = rel("r-owns-2", "OWNS", alice.id, account.id);
    const back = rel("r-back", "CONTROLLED_BY", account.id, alice.id);
    mergeSubgraph(graph, { entities: [], relationships: [again, back] });
    const between = [owns.id, again.id, back.id].map((id) => graph.getEdgeAttributes(id));
    expect(between.map((e) => e.id)).toEqual([owns.id, again.id, back.id]);
    // Three relationships between the same pair: two curved, one straight.
    expect(between.filter((e) => e.type === "curved")).toHaveLength(2);
    // Seen from the same direction, every curvature is different.
    const sides = between.map((e) => (e.source === alice.id ? e.curvature : -e.curvature));
    expect(new Set(sides).size).toBe(3);
  });

  it("keeps self-loops", () => {
    const graph = loaded();
    const loop = rel("r-loop", "REFERS_TO", alice.id, alice.id);
    mergeSubgraph(graph, { entities: [], relationships: [loop] });
    expect(graph.hasEdge(loop.id)).toBe(true);
    expect(graph.getEdgeAttributes(loop.id)).toMatchObject({ selfLoop: true, curvature: 0 });
  });

  it("rejects malformed payloads without changing the graph", () => {
    const graph = loaded();
    const bad = [
      null,
      { entities: "nope", relationships: [] },
      { entities: [{ ...budi, id: "" }], relationships: [] },
      { entities: [{ ...budi, properties: null }], relationships: [] },
      { entities: [{ ...budi, property_types: { name: "blob" } }], relationships: [] },
      { entities: [], relationships: [rel("r-x", "USES", budi.id, "e-missing")] },
    ];
    for (const payload of bad) {
      expect(() => mergeSubgraph(graph, payload)).toThrow(MalformedGraphError);
    }
    expect(() =>
      mergeSubgraph(
        graph,
        { entities: [{ ...budi, graph_id: "other" }], relationships: [] },
        { graphId: G },
      ),
    ).toThrow(/another graph/);
    expect(graph.order).toBe(3);
    expect(graph.size).toBe(2);
  });

  it("stops at the view budget and reports what was left out", () => {
    const graph = loaded();
    const extra = Array.from({ length: 5 }, (_, i) => entity(`e-x${i}`, "Account"));
    const links = extra.map((e, i) => rel(`r-x${i}`, "OWNS", alice.id, e.id));
    const result = mergeSubgraph(
      graph,
      { entities: [alice, ...extra], relationships: links },
      { anchor: alice.id, budget: { maxEntities: 5, maxRelationships: 3 } },
    );
    expect(result.addedEntityIds).toHaveLength(2);
    expect(result.skippedEntities).toBe(3);
    expect(result.addedRelationshipIds).toHaveLength(1);
    expect(result.skippedRelationships).toBe(4);
    expect(graph.order).toBe(5);
    expect(graph.size).toBe(3);
  });

  it("removes entities from the view with their relationships", () => {
    const graph = loaded();
    expect(removeEntities(graph, [device.id, "unknown"])).toEqual([usesA.id]);
    expect(graph.hasNode(device.id)).toBe(false);
    expect(graph.getNodeAttribute(alice.id, "size")).toBeCloseTo(nodeSize(1));
  });
});

describe("parsePath", () => {
  it("accepts an ordered path and a not-found answer", () => {
    const path = {
      found: true,
      hops: 2,
      entities: [alice, device, budi],
      relationships: [usesA, usesB],
    };
    expect(parsePath(path, G).hops).toBe(2);
    expect(parsePath({ found: false, hops: 0, entities: [], relationships: [] }).found).toBe(false);
    expect(() => parsePath({ ...path, hops: 3 })).toThrow(MalformedGraphError);
    expect(() => parsePath({ ...path, relationships: [usesB, usesA] })).toThrow(/out of order/);
  });
});

describe("investigation state", () => {
  it("selects entities and relationships exclusively", () => {
    let state = investigationReducer(initialInvestigation, { type: "selectEntity", id: alice.id });
    expect(state).toMatchObject({ selectedEntityId: alice.id, focusedEntityId: alice.id });
    state = investigationReducer(state, { type: "selectRelationship", id: owns.id });
    expect(state).toMatchObject({ selectedEntityId: null, selectedRelationshipId: owns.id });
    state = investigationReducer(state, { type: "clearSelection" });
    expect(state.selectedRelationshipId).toBeNull();
  });

  it("emphasises the selection and its visible neighbours, dimming the rest", () => {
    const graph = loaded();
    mergeSubgraph(graph, { entities: [budi], relationships: [usesB] });
    const state = investigationReducer(initialInvestigation, {
      type: "selectEntity",
      id: account.id,
    });
    const input = display(state);
    const focus = emphasis(graph, input)!;
    expect([...focus.entities].sort()).toEqual([account.id, alice.id].sort());
    expect([...focus.relationships]).toEqual([owns.id]);
    const selected = nodeAppearance(account.id, graph.getNodeAttributes(account.id), input, focus);
    expect(selected).toMatchObject({ highlighted: true, forceLabel: true, zIndex: 3 });
    const dimmed = nodeAppearance(budi.id, graph.getNodeAttributes(budi.id), input, focus);
    expect(dimmed).toMatchObject({ hidden: false, label: null });
    expect(dimmed.color).toBe(`#5b9bff${COLORS.dimAlpha}`);
    const edge = edgeAppearance(graph, owns.id, graph.getEdgeAttributes(owns.id), input, focus);
    expect(edge).toMatchObject({ label: "OWNS", color: COLORS.edgeFocus });
    // Hovering another node highlights its neighbours instead.
    const hover = emphasis(graph, display(state, device.id))!;
    expect([...hover.entities].sort()).toEqual([alice.id, budi.id, device.id].sort());
  });

  it("highlights a found path and dims everything else", () => {
    const graph = loaded();
    mergeSubgraph(graph, { entities: [budi], relationships: [usesB] });
    const path: GraphPath = {
      found: true,
      hops: 2,
      entities: [alice, device, budi],
      relationships: [usesA, usesB],
    };
    const state = investigationReducer(initialInvestigation, {
      type: "showPath",
      path: { sourceId: alice.id, targetId: budi.id, direction: "both", maxDepth: 3, path },
    });
    expect([...state.highlightedEntityIds]).toEqual([alice.id, device.id, budi.id]);
    const input = display(state);
    const focus = emphasis(graph, input);
    const onPath = edgeAppearance(graph, usesB.id, graph.getEdgeAttributes(usesB.id), input, focus);
    expect(onPath).toMatchObject({ color: COLORS.path, label: "USES" });
    const off = edgeAppearance(graph, owns.id, graph.getEdgeAttributes(owns.id), input, focus);
    expect(off).toMatchObject({ color: COLORS.edgeDim, label: null, hidden: false });
    const reset = investigationReducer(state, { type: "resetHighlight" });
    expect(reset.activePath).toBeNull();
    expect(emphasis(graph, display(reset))).toBeNull();
  });

  it("records a path that was not found without highlighting anything", () => {
    const state = investigationReducer(initialInvestigation, {
      type: "showPath",
      path: {
        sourceId: alice.id,
        targetId: budi.id,
        direction: "outgoing",
        maxDepth: 4,
        path: { found: false, hops: 0, entities: [], relationships: [] },
      },
    });
    expect(state.activePath?.path.found).toBe(false);
    expect(state.highlightedEntityIds.size).toBe(0);
  });

  it("hides entities, entity types and relationship types client-side", () => {
    const graph = loaded();
    let state = investigationReducer(initialInvestigation, { type: "selectEntity", id: device.id });
    state = investigationReducer(state, { type: "hideEntity", id: device.id });
    expect(state.selectedEntityId).toBeNull();
    expect([...visibleEntityIds(graph, state)].sort()).toEqual([account.id, alice.id].sort());
    const hiddenEdge = edgeAppearance(
      graph,
      usesA.id,
      graph.getEdgeAttributes(usesA.id),
      display(state),
      null,
    );
    expect(hiddenEdge.hidden).toBe(true);

    state = investigationReducer(state, { type: "toggleEntityType", name: "Account" });
    expect([...visibleEntityIds(graph, state)]).toEqual([alice.id]);
    state = investigationReducer(state, { type: "showHidden" });
    state = investigationReducer(state, { type: "toggleEntityType", name: "Account" });
    state = investigationReducer(state, { type: "toggleRelationshipType", name: "OWNS" });
    expect(visibleEntityIds(graph, state).size).toBe(3);
    const owned = edgeAppearance(
      graph,
      owns.id,
      graph.getEdgeAttributes(owns.id),
      display(state),
      null,
    );
    expect(owned.hidden).toBe(true);
    // Hiding never removes anything from the loaded graph.
    expect(graph.order).toBe(3);
  });

  it("groups an entity's visible relationships by type and direction", () => {
    const graph = loaded();
    mergeSubgraph(graph, {
      entities: [budi],
      relationships: [usesB, rel("r-self", "REFERS_TO", device.id, device.id)],
    });
    const groups = relationshipGroups(graph, initialInvestigation, device.id);
    expect(groups.map((g) => [g.relationshipType, g.direction, g.items.length])).toEqual([
      ["REFERS_TO", "self", 1],
      ["USES", "in", 2],
    ]);
    const hidden = investigationReducer(initialInvestigation, { type: "hideEntity", id: budi.id });
    expect(relationshipGroups(graph, hidden, device.id)[1].items.map((i) => i.label)).toEqual([
      "Alice Tan",
    ]);
  });

  it("collapses only the neighbours that are not shared", () => {
    const graph = loaded();
    mergeSubgraph(graph, { entities: [budi], relationships: [usesB] });
    // Alice: the account is hers alone; the device is shared with Budi.
    expect(collapsibleNeighbors(graph, initialInvestigation, alice.id)).toEqual([account.id]);
    expect(collapsibleNeighbors(graph, initialInvestigation, device.id)).toEqual([budi.id]);
    let state = investigationReducer(initialInvestigation, {
      type: "selectEntity",
      id: account.id,
    });
    state = investigationReducer(state, {
      type: "hideEntities",
      ids: collapsibleNeighbors(graph, state, alice.id),
    });
    expect(state.selectedEntityId).toBeNull();
    expect([...visibleEntityIds(graph, state)].sort()).toEqual(
      [alice.id, budi.id, device.id].sort(),
    );
    // With Budi hidden, the device is no longer shared.
    state = investigationReducer(state, { type: "hideEntity", id: budi.id });
    expect(collapsibleNeighbors(graph, state, alice.id)).toEqual([device.id]);
    expect(graph.order).toBe(4);
  });

  it("forgets entities removed from the view, dropping a path through them", () => {
    let state = investigationReducer(initialInvestigation, { type: "hideEntity", id: device.id });
    state = investigationReducer(state, {
      type: "showPath",
      path: {
        sourceId: alice.id,
        targetId: budi.id,
        direction: "both",
        maxDepth: 3,
        path: {
          found: true,
          hops: 2,
          entities: [alice, device, budi],
          relationships: [usesA, usesB],
        },
      },
    });
    state = investigationReducer(state, {
      type: "forget",
      entityIds: [device.id],
      relationshipIds: [usesA.id],
    });
    expect(state.hiddenEntityIds.size).toBe(0);
    expect(state.activePath).toBeNull();
    expect(state.highlightedRelationshipIds.size).toBe(0);
  });
});

describe("layouts", () => {
  it("settles new nodes without moving the existing ones", () => {
    const graph = loaded();
    const before = graph.mapNodes((id, { x, y }) => ({ id, x, y }));
    const { addedEntityIds } = mergeSubgraph(
      graph,
      { entities: [budi], relationships: [usesB] },
      { anchor: device.id },
    );
    settleNewNodes(graph, addedEntityIds);
    for (const { id, x, y } of before) expect(graph.getNodeAttributes(id)).toMatchObject({ x, y });
    expect(Number.isFinite(graph.getNodeAttribute(budi.id, "x"))).toBe(true);
  });

  it("computes force and circular positions for every node", () => {
    const graph = loaded();
    expect(Object.keys(forceLayout(graph)).sort()).toEqual(graph.nodes().sort());
    const circle = circularLayout(graph);
    const radii = Object.values(circle).map(({ x, y }) => Math.round(Math.hypot(x, y)));
    expect(new Set(radii).size).toBe(1);
  });
});

const ONTOLOGY: Ontology = {
  graph_id: G,
  version: 2,
  entity_types: [
    {
      id: "t1",
      name: "Customer",
      display_name: "Customer",
      description: null,
      properties: [
        { key: "customer_id", type: "string", required: true },
        { key: "age", type: "integer", required: false },
        { key: "vip", type: "boolean", required: false },
        { key: "score", type: "float", required: false },
      ],
    },
    {
      id: "t2",
      name: "Account",
      display_name: "Account",
      description: null,
      properties: [{ key: "score", type: "integer", required: false }],
    },
  ],
  relationship_types: [],
};

describe("exact entity search", () => {
  const form = { entityType: "Customer", property: "customer_id", value: " C001 ", limit: 25 };

  it("types values from the active ontology", () => {
    expect(buildSearchQuery(form, ONTOLOGY)).toEqual({
      ok: true,
      query: { limit: 25, entity_type: "Customer", property_filters: { customer_id: "C001" } },
    });
    const age = buildSearchQuery({ ...form, property: "age", value: "9007199254740993" }, ONTOLOGY);
    expect(age).toMatchObject({ query: { property_filters: { age: "9007199254740993" } } });
    const vip = buildSearchQuery({ ...form, property: "vip", value: "TRUE" }, ONTOLOGY);
    expect(vip).toMatchObject({ query: { property_filters: { vip: true } } });
    expect(buildSearchQuery({ ...form, property: "", value: "" }, ONTOLOGY)).toEqual({
      ok: true,
      query: { limit: 25, entity_type: "Customer" },
    });
  });

  it("explains invalid searches before sending them", () => {
    const cases: [Partial<typeof form>, RegExp][] = [
      [{ property: "age", value: "3.5" }, /whole number/],
      [{ property: "vip", value: "yes" }, /true or false/],
      [{ property: "missing" }, /not a property/],
      [{ property: "", value: "C001" }, /Choose the property/],
      [{ value: "  " }, /exact value/],
      [{ entityType: "Ghost" }, /not an entity type/],
      [{ entityType: "", property: "score", value: "1" }, /different types/],
      [{ limit: 0 }, /between 1 and 1000/],
    ];
    for (const [change, error] of cases) {
      const result = buildSearchQuery({ ...form, ...change }, ONTOLOGY);
      expect(result.ok).toBe(false);
      expect(!result.ok && result.error).toMatch(error);
    }
    expect(propertyChoices(ONTOLOGY, "").find((c) => c.key === "score")?.ambiguous).toBe(true);
  });

  it("summarises outcomes without claiming totals", () => {
    const query = { limit: 2 };
    expect(searchSummary({ status: "done", query, results: [], limit: 2 })).toMatch(/No entities/);
    expect(searchSummary({ status: "done", query, results: [alice], limit: 2 })).toBe("1 match");
    expect(searchSummary({ status: "done", query, results: [alice, budi], limit: 2 })).toMatch(
      /First 2 matches/,
    );
  });
});
