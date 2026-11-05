// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { type ReactNode, useEffect } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { CanvasHandle, CanvasProps } from "@/components/explorer/canvas";
import { GraphExplorer } from "@/components/explorer/GraphExplorer";
import { type ApiClient, ApiError } from "@/lib/api/client";
import type { Entity, GraphProject, Ontology, Relationship, Subgraph } from "@/lib/api/types";

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

/** Draws the loaded graph as buttons, so tests can click entities and relationships. */
function FakeCanvas(props: CanvasProps) {
  const { graph, controllerRef, onReady } = props;
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
          onClick={() => props.onSelectEntity(id)}
          onDoubleClick={() => props.onExpand(id)}
        >
          node:{attributes.label}
        </button>
      ))}
      {graph.mapEdges((id, attributes) => (
        <button key={id} type="button" onClick={() => props.onSelectRelationship(id)}>
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
    fireEvent.change(screen.getByLabelText("Target entity ID"), { target: { value: budi.id } });
    fireEvent.click(screen.getByRole("button", { name: "Use" }));
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
