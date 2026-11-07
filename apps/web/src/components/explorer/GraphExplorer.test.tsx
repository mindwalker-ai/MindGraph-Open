// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { type ReactNode, useEffect } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { CanvasHandle, CanvasProps } from "@/components/explorer/canvas";
import { GraphExplorer } from "@/components/explorer/GraphExplorer";
import { type ApiClient, ApiError } from "@/lib/api/client";
import type {
  AskResponse,
  Entity,
  GraphProject,
  Ontology,
  Relationship,
  Subgraph,
} from "@/lib/api/types";

vi.mock("next/link", () => ({
  default: ({ href, children, ...props }: { href: string; children: ReactNode }) => (
    <a href={href} {...props}>
      {children}
    </a>
  ),
}));
vi.mock("next/image", () => ({
  default: ({ alt }: { alt: string }) => <span data-alt={alt} />,
}));
vi.mock("next/navigation", () => ({ usePathname: () => "/graphs/g1/explore" }));
// The real canvas needs WebGL; the tests pass FakeCanvas instead.
vi.mock("next/dynamic", () => ({ default: () => () => null }));

afterEach(cleanup);

const GRAPH: GraphProject = {
  id: "g1",
  workspace_id: "w",
  name: "Fraud Investigation",
  description: null,
  status: "active",
  created_at: "2026-11-04T00:00:00Z",
  updated_at: "2026-11-04T00:00:00Z",
};

const ONTOLOGY: Ontology = {
  graph_id: "g1",
  version: 3,
  entity_types: [
    {
      id: "t1",
      name: "Customer",
      display_name: "Customer",
      description: null,
      properties: [
        { key: "customer_id", type: "string", required: true },
        { key: "name", type: "string", required: false },
        { key: "age", type: "integer", required: false },
      ],
    },
    {
      id: "t2",
      name: "Account",
      display_name: "Account",
      description: null,
      properties: [{ key: "account_id", type: "string", required: true }],
    },
    {
      id: "t3",
      name: "Device",
      display_name: "Device",
      description: null,
      properties: [{ key: "device_id", type: "string", required: true }],
    },
  ],
  relationship_types: [
    {
      id: "r1",
      name: "OWNS",
      source_entity_type: "Customer",
      target_entity_type: "Account",
      properties: [],
    },
    {
      id: "r2",
      name: "USES",
      source_entity_type: "Customer",
      target_entity_type: "Device",
      properties: [],
    },
  ],
};

function entity(id: string, type: string, properties: Entity["properties"]): Entity {
  const property_types = Object.fromEntries(
    Object.entries(properties).map(([key, value]) => [
      key,
      typeof value === "number" ? "integer" : "string",
    ]),
  ) as Entity["property_types"];
  return { id, graph_id: "g1", entity_type: type, properties, property_types };
}

function rel(id: string, type: string, source: string, target: string): Relationship {
  return {
    id,
    graph_id: "g1",
    relationship_type: type,
    source_id: source,
    target_id: target,
    properties: {},
    property_types: {},
  };
}

const alice = entity("e-alice", "Customer", { customer_id: "C001", name: "Alice Tan", age: 34 });
const budi = entity("e-budi", "Customer", { customer_id: "C002", name: "Budi" });
const account = entity("e-acc", "Account", { account_id: "A100" });
const device = entity("e-dev", "Device", { device_id: "DEVICE-921" });
const owns = rel("r-owns", "OWNS", alice.id, account.id);
const usesA = rel("r-uses-a", "USES", alice.id, device.id);
const usesB = rel("r-uses-b", "USES", budi.id, device.id);

const NEIGHBOURS: Record<string, Subgraph> = {
  [alice.id]: { entities: [alice, account, device], relationships: [owns, usesA] },
  [device.id]: { entities: [device, alice, budi], relationships: [usesA, usesB] },
  [budi.id]: { entities: [budi, device], relationships: [usesB] },
  [account.id]: { entities: [account, alice], relationships: [owns] },
};
const ENTITIES = Object.fromEntries([alice, budi, account, device].map((e) => [e.id, e]));

/** Alice and Budi share DEVICE-921 (Budi is not loaded before the answer). */
const ANSWER: AskResponse = {
  status: "answered",
  answer: "Alice Tan [E1] and Budi [E3] both use DEVICE-921 [E2] [R1, R2].",
  answer_source: "model",
  evidence: {
    entities: [
      { ...alice, ref: "E1" },
      { ...device, ref: "E2" },
      { ...budi, ref: "E3" },
    ],
    relationships: [
      { ...usesA, ref: "R1" },
      { ...usesB, ref: "R2" },
    ],
    paths: [
      {
        ref: "P1",
        entity_ids: [alice.id, device.id, budi.id],
        relationship_ids: [usesA.id, usesB.id],
      },
    ],
  },
  citations: [
    { ref: "E1", kind: "entity", id: alice.id },
    { ref: "E2", kind: "entity", id: device.id },
    { ref: "E3", kind: "entity", id: budi.id },
    { ref: "R1", kind: "relationship", id: usesA.id },
    { ref: "R2", kind: "relationship", id: usesB.id },
  ],
  warnings: [],
  retrieval: {
    scope: "selected_entities",
    plan: {
      operation: "shared_neighbor",
      source: "selected entity S1",
      entity_type: "Customer",
      shared_entity_type: "Device",
      relationship_type: "USES",
      direction: "both",
    },
    operations: 3,
    candidates_inspected: 1,
    truncated: false,
    context_bytes: 900,
    limitations: [],
  },
  metadata: {
    interpret_prompt_version: "graphrag_interpret_v1",
    synthesize_prompt_version: "graphrag_synthesize_v1",
    provider: "openai_compatible",
    model: "test-model",
    conversation_turns: 0,
    duration_ms: 1400,
  },
};

const handle: CanvasHandle = {
  zoomIn: vi.fn(),
  zoomOut: vi.fn(),
  fit: vi.fn(),
  center: vi.fn(),
  focus: vi.fn(),
  applyLayout: vi.fn(),
  zoomPercent: () => 100,
  subscribe: () => () => {},
};

/**
 * Draws the loaded graph as buttons, so tests can click entities and
 * relationships; `data-highlight` exposes the highlight tone.
 */
function FakeCanvas(props: CanvasProps) {
  const { graph, controllerRef, onReady, state } = props;
  useEffect(() => {
    controllerRef.current = handle;
    onReady();
    return () => {
      controllerRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return (
    <div data-testid="canvas">
      {graph.mapNodes((id, attributes) => (
        <button
          key={id}
          type="button"
          data-highlight={state.highlightedEntityIds.has(id) ? state.highlightTone : undefined}
          onClick={() => props.onSelectEntity(id)}
          onDoubleClick={() => props.onExpand(id)}
        >
          node:{attributes.label}
        </button>
      ))}
      {graph.mapEdges((id, attributes) => (
        <button
          key={id}
          type="button"
          data-highlight={
            state.highlightedRelationshipIds.has(id) ? state.highlightTone : undefined
          }
          onClick={() => props.onSelectRelationship(id)}
        >
          edge:{attributes.relationshipType}
        </button>
      ))}
    </div>
  );
}

function notFound(code: string) {
  return new ApiError(404, code, "not found", "req-1");
}

function api(overrides: Partial<Record<keyof ApiClient, unknown>> = {}): ApiClient {
  return {
    listGraphs: vi.fn(),
    createGraph: vi.fn(),
    getGraph: vi.fn().mockResolvedValue(GRAPH),
    getOntology: vi.fn().mockResolvedValue(ONTOLOGY),
    createOntology: vi.fn(),
    uploadDataSource: vi.fn(),
    getDataSource: vi.fn(),
    analyze: vi.fn(),
    graphBuilderStatus: vi.fn(),
    requestSuggestion: vi.fn(),
    startIngestion: vi.fn(),
    getJob: vi.fn(),
    searchEntities: vi.fn().mockResolvedValue({ data: [alice, budi], count: 2, limit: 25 }),
    getEntity: vi.fn(async (_g: string, id: string) => {
      if (ENTITIES[id]) return ENTITIES[id];
      throw notFound("ENTITY_NOT_FOUND");
    }),
    getNeighbors: vi.fn(async (_g: string, id: string) => NEIGHBOURS[id]),
    findPath: vi.fn().mockResolvedValue({
      found: true,
      hops: 2,
      entities: [alice, device, budi],
      relationships: [usesA, usesB],
    }),
    getSubgraph: vi.fn().mockResolvedValue({ entities: [], relationships: [] }),
    graphRagStatus: vi.fn().mockResolvedValue({ status: "ready", configured: true }),
    ask: vi.fn().mockResolvedValue(ANSWER),
    runResolutionScan: vi.fn(),
    listResolutionScans: vi.fn().mockResolvedValue([]),
    listResolutionCandidates: vi.fn().mockResolvedValue({
      data: [],
      next_cursor: null,
      limit: 3,
      counts: { pending: 0, accepted: 0, rejected: 0 },
    }),
    getResolutionCandidate: vi.fn(),
    decideResolutionCandidate: vi.fn(),
    getEntityIdentity: vi.fn(async (_g: string, id: string) => ({
      entity_id: id,
      canonical_entity_id: id,
      member_entity_ids: [id],
      is_canonical: true,
      resolved: false,
      resolution_type: null,
      entity_type: null,
      members: [],
      physical_graph_merged: false,
    })),
    ...overrides,
  } as ApiClient;
}

function explore(client: ApiClient, props: { initialEntityId?: string } = {}) {
  render(<GraphExplorer graphId="g1" api={client} Canvas={FakeCanvas} {...props} />);
}

const details = () => screen.getByRole("complementary", { name: "Details" });

/** A status bar counter, e.g. "3 entities loaded". */
const loaded = (what: "entities" | "relationships") =>
  screen.getByText(
    (_, element) =>
      element?.tagName === "SPAN" &&
      new RegExp(`^\\d+ ${what} loaded$`).test(element.textContent ?? ""),
  ).textContent;

describe("GraphExplorer", () => {
  it("explains an empty graph and leads to the Graph Builder", async () => {
    const client = api({
      searchEntities: vi.fn().mockResolvedValue({ data: [], count: 0, limit: 25 }),
    });
    explore(client);
    expect(screen.getByText("Loading graph…")).toBeTruthy();
    expect(await screen.findByText("Your graph is empty")).toBeTruthy();
    expect(screen.getByRole("link", { name: "Open Graph Builder" }).getAttribute("href")).toBe(
      "/graphs/g1/builder",
    );
    expect(client.getNeighbors).not.toHaveBeenCalled();
  });

  it("loads one seed neighbourhood and shows the selected entity", async () => {
    const client = api();
    explore(client);
    expect(await screen.findByRole("heading", { name: "Alice Tan" })).toBeTruthy();
    expect(client.searchEntities).toHaveBeenCalledWith("g1", { limit: 25 });
    expect(client.getNeighbors).toHaveBeenCalledTimes(1);
    expect(client.getNeighbors).toHaveBeenCalledWith("g1", alice.id, {
      direction: "both",
      limit: 100,
    });
    const panel = within(details());
    expect(panel.getByText("C001")).toBeTruthy();
    expect(panel.getByText("34")).toBeTruthy();
    expect(panel.getAllByText("OWNS").length).toBeGreaterThan(0);
    expect(loaded("entities")).toBe("3 entities loaded");
    expect(handle.fit).toHaveBeenCalled();
    expect(screen.getByRole("link", { name: "Graph Builder" }).getAttribute("href")).toBe(
      "/graphs/g1/builder",
    );
    expect(window.location.search).toBe(`?entity=${alice.id}`);

    fireEvent.click(screen.getByText("node:A100"));
    expect(within(details()).getByRole("heading", { name: "A100" })).toBeTruthy();
  });

  it("opens relationship details with clickable endpoints", async () => {
    explore(api());
    fireEvent.click(await screen.findByText("edge:OWNS"));
    const panel = within(details());
    expect(panel.getByRole("heading", { name: "OWNS" })).toBeTruthy();
    expect(panel.getByText("Relationship")).toBeTruthy();
    fireEvent.click(panel.getByRole("button", { name: /A100/ }));
    expect(within(details()).getByRole("heading", { name: "A100" })).toBeTruthy();
  });

  it("expands an entity through the API and merges without duplicates", async () => {
    const client = api();
    explore(client);
    fireEvent.click(await screen.findByText("node:DEVICE-921"));
    fireEvent.click(within(details()).getByRole("button", { name: /Expand/ }));
    expect(await screen.findByText("node:Budi")).toBeTruthy();
    expect(client.getNeighbors).toHaveBeenLastCalledWith("g1", device.id, {
      direction: "both",
      limit: 100,
    });
    expect(loaded("entities")).toBe("4 entities loaded");
    expect(loaded("relationships")).toBe("3 relationships loaded");
    expect(screen.getByText(/neighbors\(DEVICE-921, both, limit 100\)/)).toBeTruthy();
  });

  it("finds a path through MindGraph Core and highlights it", async () => {
    const client = api();
    explore(client);
    await screen.findByRole("heading", { name: "Alice Tan" });
    fireEvent.click(within(details()).getByRole("button", { name: /Find Path/ }));
    // A nested <form> would submit natively in a real browser and reload the page.
    expect(document.querySelector("form form")).toBeNull();
    const target = screen.getByLabelText("Target entity ID");
    fireEvent.change(target, { target: { value: budi.id } });
    fireEvent.keyDown(target, { key: "Enter" });
    await waitFor(() => expect(client.getEntity).toHaveBeenCalledWith("g1", budi.id));
    fireEvent.click(await screen.findByRole("button", { name: /Find shortest path/ }));

    expect(await screen.findByText("hops")).toBeTruthy();
    expect(client.findPath).toHaveBeenCalledWith("g1", alice.id, budi.id, {
      direction: "both",
      max_depth: 3,
    });
    expect(screen.getByText("node:Budi")).toBeTruthy();
    expect(handle.focus).toHaveBeenLastCalledWith([alice.id, device.id, budi.id]);
    expect(within(details()).getAllByText("USES")).toHaveLength(2);
  });

  it("offers a deeper search when no path is found", async () => {
    const client = api({
      findPath: vi
        .fn()
        .mockResolvedValue({ found: false, hops: 0, entities: [], relationships: [] }),
    });
    explore(client);
    await screen.findByRole("heading", { name: "Alice Tan" });
    fireEvent.click(within(details()).getByRole("button", { name: /Find Path/ }));
    fireEvent.change(screen.getByLabelText("Target from the entities in view"), {
      target: { value: account.id },
    });
    fireEvent.click(screen.getByRole("button", { name: /Find shortest path/ }));
    expect(await screen.findByText("No path found")).toBeTruthy();
    expect(screen.getByText(/No connection was found within 3 hops/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Increase max depth to 4" }));
    await waitFor(() =>
      expect(client.findPath).toHaveBeenLastCalledWith("g1", alice.id, account.id, {
        direction: "both",
        max_depth: 4,
      }),
    );
  });

  it("keeps the loaded graph when a later request fails", async () => {
    const client = api({
      getNeighbors: vi.fn(async (_g: string, id: string) => {
        if (id === device.id) throw new ApiError(503, "SERVICE_UNAVAILABLE", "down");
        return NEIGHBOURS[id];
      }),
    });
    explore(client);
    fireEvent.click(await screen.findByText("node:DEVICE-921"));
    fireEvent.click(within(details()).getByRole("button", { name: /Expand/ }));
    expect((await screen.findByRole("alert")).textContent).toMatch(/graph store is unavailable/);
    expect(loaded("entities")).toBe("3 entities loaded");
    expect(screen.getByText("node:Alice Tan")).toBeTruthy();
  });

  it("shows loading feedback while a neighbourhood loads", async () => {
    let release: (value: Subgraph) => void = () => {};
    const client = api({
      getNeighbors: vi.fn(() => new Promise<Subgraph>((resolve) => (release = resolve))),
    });
    explore(client);
    expect(await screen.findByText("Loading neighbourhood…")).toBeTruthy();
    release(NEIGHBOURS[alice.id]);
    expect(await screen.findByRole("heading", { name: "Alice Tan" })).toBeTruthy();
    expect(screen.queryByText("Loading neighbourhood…")).toBeNull();
  });

  it("reports a missing graph without a retry", async () => {
    explore(api({ getGraph: vi.fn().mockRejectedValue(notFound("GRAPH_NOT_FOUND")) }));
    expect(await screen.findByText("Graph not found")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Try again" })).toBeNull();
  });

  it("searches by exact property value and opens a result", async () => {
    const client = api();
    explore(client, { initialEntityId: account.id });
    await screen.findByRole("heading", { name: "A100" });
    const searchForm = within(screen.getByRole("form", { name: "Entity search" }));
    fireEvent.change(searchForm.getByLabelText("Entity type"), { target: { value: "Customer" } });
    fireEvent.change(searchForm.getByLabelText("Property"), { target: { value: "customer_id" } });
    fireEvent.change(searchForm.getByLabelText("Value"), { target: { value: "C002" } });
    (client.searchEntities as ReturnType<typeof vi.fn>).mockResolvedValueOnce({
      data: [budi],
      count: 1,
      limit: 25,
    });
    fireEvent.click(searchForm.getByRole("button", { name: /Search/ }));
    await waitFor(() =>
      expect(client.searchEntities).toHaveBeenLastCalledWith("g1", {
        limit: 25,
        entity_type: "Customer",
        property_filters: { customer_id: "C002" },
      }),
    );
    expect(await screen.findByText("1 match")).toBeTruthy();
    fireEvent.click(
      within(screen.getByRole("list", { name: "Search results" })).getByRole("button", {
        name: /Budi/,
      }),
    );
    expect(await within(details()).findByRole("heading", { name: "Budi" })).toBeTruthy();
    expect(client.getNeighbors).toHaveBeenLastCalledWith("g1", budi.id, {
      direction: "both",
      limit: 100,
    });
  });

  it("collapses a neighbourhood without touching shared neighbours", async () => {
    const client = api();
    explore(client);
    await screen.findByRole("heading", { name: "Alice Tan" });
    // Load Budi through the shared device: the device is then shared.
    fireEvent.click(screen.getByText("node:DEVICE-921"));
    fireEvent.click(within(details()).getByRole("button", { name: /Expand/ }));
    await screen.findByText("node:Budi");
    fireEvent.click(screen.getByText("node:Alice Tan"));
    fireEvent.click(within(details()).getByRole("button", { name: "Collapse" }));
    expect(screen.getByText(/Collapsed 1 neighbour of Alice Tan/)).toBeTruthy();
    expect(screen.getByText("1 hidden")).toBeTruthy();
    expect(loaded("entities")).toBe("4 entities loaded");
  });

  it("hides entities client-side only", async () => {
    explore(api());
    await screen.findByRole("heading", { name: "Alice Tan" });
    fireEvent.click(within(details()).getByRole("button", { name: "Hide" }));
    expect(screen.getByText("1 hidden")).toBeTruthy();
    expect(loaded("entities")).toBe("3 entities loaded");
    fireEvent.click(screen.getByRole("button", { name: "Show all" }));
    expect(screen.queryByText("1 hidden")).toBeNull();
  });
});

describe("Ask MindGraph", () => {
  const panel = () => screen.getByRole("complementary", { name: "Ask MindGraph" });
  const highlighted = (label: string) =>
    screen.getByText(`node:${label}`).getAttribute("data-highlight");

  async function openAsk(client: ApiClient) {
    explore(client);
    await screen.findByRole("heading", { name: "Alice Tan" });
    fireEvent.click(screen.getByRole("button", { name: "Ask MindGraph" }));
    return within(panel());
  }

  function ask(ui: ReturnType<typeof within>, question: string) {
    fireEvent.change(ui.getByRole("textbox", { name: "Question" }), {
      target: { value: question },
    });
    fireEvent.click(ui.getByRole("button", { name: "Ask" }));
  }

  it("opens next to the graph, scoped to the selected entity, and closes", async () => {
    const ui = await openAsk(api());
    expect(screen.getByTestId("canvas")).toBeTruthy();
    expect(
      ui.getByRole("radio", { name: "Selected · Alice Tan" }).getAttribute("aria-checked"),
    ).toBe("true");
    // Suggestions follow the ontology and the selected entity's type.
    expect(
      ui.getByRole("button", {
        name: "Which other Customer entities share a Device with Alice Tan?",
      }),
    ).toBeTruthy();
    fireEvent.click(ui.getByRole("button", { name: "Close Ask MindGraph" }));
    expect(within(details()).getByRole("heading", { name: "Alice Tan" })).toBeTruthy();
  });

  it("asks about the selected entity and highlights the evidence", async () => {
    let answer!: (value: AskResponse) => void;
    const client = api({
      ask: vi.fn(() => new Promise<AskResponse>((resolve) => (answer = resolve))),
    });
    const ui = await openAsk(client);
    ask(ui, "Who else shares this device?");
    expect(client.ask).toHaveBeenCalledWith("g1", {
      question: "Who else shares this device?",
      scope: { mode: "selected_entities", entity_ids: [alice.id] },
    });
    expect(ui.getByText("Retrieving graph evidence and writing the answer…")).toBeTruthy();
    answer(ANSWER);
    expect(await ui.findByText("Answered")).toBeTruthy();
    expect(ui.getByText(/both use DEVICE-921/)).toBeTruthy();
    expect(ui.getAllByRole("button", { name: "E1" }).length).toBeGreaterThan(0);
    // The evidence was merged (Budi is new) and highlighted in place.
    expect(loaded("entities")).toBe("4 entities loaded");
    for (const label of ["Alice Tan", "Budi", "DEVICE-921"]) {
      expect(highlighted(label)).toBe("evidence");
    }
    expect(highlighted("A100")).toBeNull();
    expect(screen.getByText(/Highlighting evidence · 3 entities · 2 relationships/)).toBeTruthy();
    // The answer gets the room: suggestions only start a conversation.
    expect(ui.queryByText("Which entities are directly connected to Alice Tan?")).toBeNull();
    expect(ui.getByRole("textbox", { name: "Question" })).toHaveProperty("value", "");
  });

  it("asks follow-ups with the earlier questions and the entities they cited", async () => {
    const client = api();
    const ui = await openAsk(client);
    ask(ui, "Who else shares this device?");
    await ui.findByText("Answered");
    expect(ui.getByRole("textbox", { name: "Question" }).getAttribute("placeholder")).toBe(
      "Ask a follow-up question…",
    );
    expect(ui.getByText("Follow-ups use your last 3 questions as context.")).toBeTruthy();

    ask(ui, "Is there a path between them?");
    // The selected entity first, then the cited ones in citation order.
    expect(client.ask).toHaveBeenLastCalledWith("g1", {
      question: "Is there a path between them?",
      scope: { mode: "selected_entities", entity_ids: [alice.id] },
      history: [
        {
          question: "Who else shares this device?",
          entity_ids: [alice.id, device.id, budi.id],
        },
      ],
    });
    await waitFor(() =>
      expect(
        ui.getByRole("article", { name: "Earlier question: Who else shares this device?" }),
      ).toBeTruthy(),
    );
    await waitFor(() => expect(ui.queryByRole("status")).toBeNull());

    // Only the last three questions travel with a follow-up.
    for (const question of ["Third?", "Fourth?", "Fifth?"]) {
      ask(ui, question);
      await waitFor(() => expect(ui.queryByRole("status")).toBeNull());
    }
    const request = vi.mocked(client.ask).mock.lastCall?.[1];
    expect(request?.history?.map((turn) => turn.question)).toEqual([
      "Is there a path between them?",
      "Third?",
      "Fourth?",
    ]);
    expect(ui.getAllByRole("article")).toHaveLength(4);
  });

  it("shows an earlier answer on the graph and starts a new conversation", async () => {
    const client = api();
    const ui = await openAsk(client);
    ask(ui, "Who else shares this device?");
    await ui.findByText("Answered");
    ask(ui, "And then?");
    await ui.findByRole("article");
    await waitFor(() => expect(ui.queryByRole("status")).toBeNull());

    fireEvent.click(ui.getByRole("button", { name: "Clear Evidence Highlight" }));
    expect(highlighted("Budi")).toBeNull();
    const earlier = within(ui.getByRole("article"));
    fireEvent.click(earlier.getByRole("button", { name: "Show on graph" }));
    expect(highlighted("Budi")).toBe("evidence");

    fireEvent.click(ui.getByRole("button", { name: "New conversation" }));
    expect(ui.queryByRole("article")).toBeNull();
    expect(highlighted("Budi")).toBeNull();
    expect(loaded("entities")).toBe("4 entities loaded");
    expect(ui.getByText("Which entities are directly connected to Alice Tan?")).toBeTruthy();
    ask(ui, "Who is this?");
    expect(client.ask).toHaveBeenLastCalledWith("g1", {
      question: "Who is this?",
      scope: { mode: "selected_entities", entity_ids: [alice.id] },
    });
  });

  it("shows references on the graph and clears the highlight without losing data", async () => {
    const ui = await openAsk(api());
    ask(ui, "Who else shares this device?");
    await ui.findByText("Answered");

    fireEvent.click(ui.getByRole("button", { name: "Clear Evidence Highlight" }));
    expect(highlighted("Budi")).toBeNull();
    expect(loaded("entities")).toBe("4 entities loaded");

    vi.mocked(handle.focus).mockClear();
    fireEvent.click(ui.getByRole("button", { name: "Show on Graph" }));
    expect(highlighted("Budi")).toBe("evidence");
    expect(handle.focus).toHaveBeenCalledWith([alice.id, device.id, budi.id]);

    // A path reference highlights the whole path.
    fireEvent.click(ui.getByRole("button", { name: "P1 · show path" }));
    expect(highlighted("DEVICE-921")).toBe("evidence");
    // An entity reference focuses that entity only.
    vi.mocked(handle.center).mockClear();
    fireEvent.click(ui.getAllByRole("button", { name: "E3" })[0]);
    expect(highlighted("Budi")).toBe("evidence");
    expect(highlighted("Alice Tan")).toBeNull();
    expect(handle.center).toHaveBeenCalledWith(budi.id);
    expect(screen.getByText(/Highlighting evidence · 1 entity · 0 relationships/)).toBeTruthy();

    // A new question drops the previous answer's highlight.
    ask(ui, "And who owns accounts?");
    expect(highlighted("Budi")).toBeNull();
  });

  it("explains insufficient evidence", async () => {
    const client = api({
      ask: vi.fn().mockResolvedValue({
        ...ANSWER,
        status: "insufficient_evidence",
        answer: "No matching graph evidence was found in the retrieved scope.",
        answer_source: "no_evidence",
        evidence: { entities: [], relationships: [], paths: [] },
        citations: [],
        retrieval: {
          ...ANSWER.retrieval,
          limitations: ['No Customer where customer_id = "C9" was found (exact match).'],
        },
      }),
    });
    const ui = await openAsk(client);
    fireEvent.click(ui.getByRole("radio", { name: "Whole graph (bounded)" }));
    ask(ui, "Who is C9?");
    expect(await ui.findByText("Insufficient evidence")).toBeTruthy();
    expect(client.ask).toHaveBeenCalledWith("g1", {
      question: "Who is C9?",
      scope: { mode: "bounded_graph" },
    });
    expect(ui.getByText(/does not support a definite answer/)).toBeTruthy();
    expect(ui.getByText(/was found \(exact match\)/)).toBeTruthy();
    expect(ui.queryByRole("button", { name: "Show on Graph" })).toBeNull();
  });

  it("keeps the graph when the AI service fails", async () => {
    const client = api({
      ask: vi.fn().mockRejectedValue(new ApiError(503, "AI_UNAVAILABLE", "down", "req-9")),
    });
    const ui = await openAsk(client);
    ask(ui, "Who else shares this device?");
    expect(await ui.findByText(/The AI service or the model provider is unavailable/)).toBeTruthy();
    expect(ui.getByText("The loaded graph is unchanged.")).toBeTruthy();
    expect(loaded("entities")).toBe("3 entities loaded");
    expect(highlighted("Alice Tan")).toBeNull();
  });

  it("explains an interrupted connection instead of the browser's error", async () => {
    const client = api({ ask: vi.fn().mockRejectedValue(new TypeError("Failed to fetch")) });
    const ui = await openAsk(client);
    ask(ui, "Who else shares this device?");
    expect(await ui.findByText(/The connection to MindGraph was interrupted/)).toBeTruthy();
    expect(ui.queryByText("Failed to fetch")).toBeNull();
    expect(ui.getByRole("button", { name: "Try again" })).toBeTruthy();
  });

  it("opens from the navigation (?ask=1) without reloading the graph", async () => {
    const client = api();
    const { rerender } = render(
      <GraphExplorer graphId="g1" api={client} Canvas={FakeCanvas} initialEntityId={alice.id} />,
    );
    await screen.findByRole("heading", { name: "Alice Tan" });
    rerender(<GraphExplorer graphId="g1" api={client} Canvas={FakeCanvas} initialPanel="ask" />);
    expect(screen.getByRole("complementary", { name: "Ask MindGraph" })).toBeTruthy();
    expect(client.getNeighbors).toHaveBeenCalledTimes(1);
    expect(client.searchEntities).toHaveBeenCalledTimes(1);
  });

  it("explains that a model must be configured", async () => {
    const client = api({
      graphRagStatus: vi.fn().mockResolvedValue({ status: "not_configured", configured: false }),
    });
    const ui = await openAsk(client);
    expect(await ui.findByText("A model must be configured")).toBeTruthy();
    expect(ui.getByRole("textbox", { name: "Question" })).toHaveProperty("disabled", true);
    expect(client.ask).not.toHaveBeenCalled();
    // Exploration still works.
    fireEvent.click(screen.getByText("node:A100"));
    expect(screen.getByText("node:A100")).toBeTruthy();
  });
});
