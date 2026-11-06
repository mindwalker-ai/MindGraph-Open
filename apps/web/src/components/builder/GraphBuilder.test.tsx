// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { type ApiClient, ApiError } from "@/lib/api/client";
import type {
  DataSource,
  GraphBuilderStatus,
  GraphBuilderSuggestion,
  Job,
  Ontology,
  OntologyDraft,
} from "@/lib/api/types";

import { GraphBuilder } from "./GraphBuilder";

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
vi.mock("next/navigation", () => ({
  usePathname: () => "/graphs/g1/builder",
  useRouter: () => ({ push: vi.fn() }),
}));

afterEach(cleanup);

const ONTOLOGY: Ontology = {
  graph_id: "g1",
  version: 2,
  entity_types: [],
  relationship_types: [],
};

function source(
  id: string,
  filename: string,
  fields: string[],
  rows: Record<string, unknown>[],
): DataSource {
  return {
    id,
    graph_id: "g1",
    kind: "csv",
    filename,
    status: "ready",
    size_bytes: 100,
    content_sha256: "0".repeat(64),
    profile: {
      record_count: rows.length,
      fields: fields.map((name) => ({
        name,
        inferred_type: "string",
        nullable: false,
        non_null_count: rows.length,
        sample_values: [],
      })),
      sample_rows: rows,
    },
    created_at: "",
    updated_at: "",
  };
}

const DEVICES = source(
  "ds-devices",
  "devices.csv",
  ["device_id", "customer_id", "device_type"],
  [
    { device_id: "DEVICE-921", customer_id: "CUST-18291", device_type: "phone" },
    { device_id: "DEVICE-921", customer_id: "CUST-20417", device_type: "phone" },
  ],
);
const CUSTOMERS = source(
  "ds-customers",
  "customers.csv",
  ["customer_id", "name"],
  [{ customer_id: "CUST-18291", name: "Ann" }],
);

const SUGGESTION: GraphBuilderSuggestion = {
  ontology: {
    entity_types: [
      {
        name: "Customer",
        display_name: "Customer",
        properties: [
          { name: "customer_id", type: "string", required: true },
          { name: "name", type: "string", required: false },
        ],
      },
      {
        name: "Device",
        display_name: "Device",
        properties: [
          { name: "device_id", type: "string", required: true },
          { name: "device_type", type: "string", required: false },
        ],
      },
    ],
    relationship_types: [
      {
        name: "USES",
        source_entity_type: "Customer",
        target_entity_type: "Device",
        properties: [],
      },
    ],
  },
  mappings: [
    {
      dataset: "customers.csv",
      data_source_id: "ds-customers",
      mapping: {
        entities: [
          {
            alias: "customer",
            entity_type: "Customer",
            mode: "create_or_match",
            identity: [{ source_field: "customer_id", property: "customer_id" }],
            properties: [
              { source_field: "customer_id", property: "customer_id" },
              { source_field: "name", property: "name" },
            ],
          },
        ],
        relationships: [],
      },
    },
    {
      dataset: "devices.csv",
      data_source_id: "ds-devices",
      mapping: {
        entities: [
          {
            alias: "customer",
            entity_type: "Customer",
            mode: "match_only",
            identity: [{ source_field: "customer_id", property: "customer_id" }],
            properties: [],
          },
          {
            alias: "device",
            entity_type: "Device",
            mode: "create_or_match",
            identity: [{ source_field: "device_id", property: "device_id" }],
            properties: [
              { source_field: "device_id", property: "device_id" },
              { source_field: "device_type", property: "device_type" },
            ],
          },
        ],
        relationships: [
          { relationship_type: "USES", source: "customer", target: "device", properties: [] },
        ],
      },
    },
  ],
  confidence: 0.92,
  annotations: {
    entities: [
      { name: "Customer", confidence: 0.99, reason: "customer_id is unique", existing: false },
      { name: "Device", confidence: 0.98, reason: "device_id is unique", existing: false },
    ],
    relationships: [
      { name: "USES", confidence: 0.96, reason: "devices name their customer", existing: false },
    ],
  },
  warnings: ["device_type is sometimes empty"],
};

function job(id: string, kind: Job["kind"], status: Job["status"], extra: Partial<Job> = {}): Job {
  return {
    id,
    graph_id: "g1",
    data_source_id: "ds-devices",
    data_source_ids: ["ds-devices"],
    kind,
    status,
    progress: status === "completed" ? 1 : 0.4,
    ontology_version: 2,
    error: null,
    result: null,
    created_at: "",
    updated_at: "",
    ...extra,
  };
}

/** What the server stores for a submitted ontology (version 3). */
function stored(draft: OntologyDraft): Ontology {
  const properties = (list: { name: string; type: string; required: boolean }[]) =>
    list.map((p) => ({ key: p.name, type: p.type, required: p.required }));
  return {
    graph_id: "g1",
    version: 3,
    entity_types: draft.entity_types.map((t) => ({
      id: t.name,
      name: t.name,
      display_name: t.display_name ?? t.name,
      description: null,
      properties: properties(t.properties),
    })),
    relationship_types: draft.relationship_types.map((t) => ({
      id: t.name,
      ...t,
      properties: properties(t.properties),
    })),
  } as Ontology;
}

type FakeApi = { [K in keyof ApiClient]: ReturnType<typeof vi.fn> };

function fakeApi(status: GraphBuilderStatus["status"] = "ready"): FakeApi {
  const polls = [
    job("ai", "ai_suggest", "running"),
    job("ai", "ai_suggest", "completed", {
      result: {
        prompt_version: "graph_builder_v2",
        provider: "openai_compatible",
        model: "test-model",
        ontology_version_base: 2,
        repair_attempts: 0,
        suggestion: SUGGESTION,
      },
    }),
  ];
  return {
    listGraphs: vi.fn(),
    createGraph: vi.fn(),
    uploadDataSource: vi.fn(),
    analyze: vi.fn(),
    getGraph: vi.fn().mockResolvedValue({
      id: "g1",
      workspace_id: "w",
      name: "Fraud",
      description: null,
      status: "draft",
      created_at: "",
      updated_at: "",
    }),
    getOntology: vi.fn().mockResolvedValue(ONTOLOGY),
    getDataSource: vi.fn(async (_graph: string, id: string) =>
      id === "ds-devices" ? DEVICES : CUSTOMERS,
    ),
    createOntology: vi.fn(async (_graph: string, draft: OntologyDraft) => stored(draft)),
    graphBuilderStatus: vi.fn().mockResolvedValue({
      status,
      configured: status !== "not_configured",
      provider: "openai_compatible",
      model: "test-model",
    }),
    requestSuggestion: vi.fn().mockResolvedValue({ job_id: "ai", status: "queued" }),
    startIngestion: vi.fn(async (_graph: string, dataSourceId: string) => ({
      job_id: `ingest-${dataSourceId}`,
      status: "queued",
    })),
    getJob: vi.fn(async (id: string) => {
      if (id === "ai") return polls.length > 1 ? polls.shift() : polls[0];
      return job(id, "ingest", "completed", {
        result: {
          records_processed: 2,
          entities_created: 2,
          entities_matched: id.endsWith("devices") ? 2 : 0,
          relationships_created: id.endsWith("devices") ? 2 : 0,
        },
      });
    }),
    searchEntities: vi.fn(),
    getEntity: vi.fn(),
    getNeighbors: vi.fn(),
    findPath: vi.fn(),
    getSubgraph: vi.fn(),
    ask: vi.fn(),
    graphRagStatus: vi.fn(),
    runResolutionScan: vi.fn(),
    listResolutionScans: vi.fn(),
    listResolutionCandidates: vi.fn(),
    getResolutionCandidate: vi.fn(),
    decideResolutionCandidate: vi.fn(),
    getEntityIdentity: vi.fn(),
  };
}

const SOURCES = ["ds-devices", "ds-customers"];

function open(api: FakeApi, sourceIds = SOURCES) {
  render(
    <GraphBuilder
      graphId="g1"
      sourceIds={sourceIds}
      api={api as unknown as ApiClient}
      pollMs={1}
    />,
  );
}

async function opened(api: FakeApi) {
  open(api);
  await screen.findByText("Build the graph model");
}

async function reviewed(api: FakeApi) {
  await opened(api);
  const suggest = await screen.findByRole("button", { name: "Generate Graph Model" });
  await waitFor(() => expect((suggest as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(suggest);
  await screen.findByRole("list", { name: "Entities" });
}

describe("GraphBuilder", () => {
  it("lists the analysed files and what the model receives", async () => {
    await opened(fakeApi());
    const files = screen.getByRole("list", { name: "Files" });
    expect(within(files).getByText("devices.csv")).toBeDefined();
    expect(within(files).getByText("customers.csv")).toBeDefined();
    expect(screen.getByText(/Never the files themselves/)).toBeDefined();
    expect(screen.getByText("2 files · 3 rows · 5 columns analyzed")).toBeDefined();
  });

  it("offers the manual path when AI is not configured", async () => {
    const api = fakeApi("not_configured");
    await opened(api);
    await screen.findByText(/AI Graph Builder is not configured/);
    expect(
      (screen.getByRole("button", { name: "Generate Graph Model" }) as HTMLButtonElement).disabled,
    ).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Build manually" }));
    const entities = await screen.findByRole("list", { name: "Entities" });
    expect(within(entities).getByText("Device")).toBeDefined();
    expect(within(entities).getByText("Customer")).toBeDefined();
    expect(api.requestSuggestion).not.toHaveBeenCalled();
  });

  it("asks for one suggestion over every file and shows it for review", async () => {
    const api = fakeApi();
    await reviewed(api);
    expect(api.requestSuggestion.mock.calls[0][1]).toEqual(SOURCES);
    const entities = screen.getByRole("list", { name: "Entities" });
    expect(within(entities).getByText("customers.csv")).toBeDefined();
    expect(within(entities).getByText("99%")).toBeDefined();
    expect(screen.getByRole("button", { name: "Relationship USES" }).textContent).toBe("USES96%");
    expect(screen.getByText("device_type is sometimes empty")).toBeDefined();

    fireEvent.click(screen.getByRole("button", { name: "Relationship USES" }));
    const inspector = screen.getByRole("complementary", { name: "Inspector" });
    expect(within(inspector).getByText("devices name their customer")).toBeDefined();
    const samples = within(inspector).getByRole("table", { name: "Sample matches" });
    expect(within(samples).getByText("CUST-20417")).toBeDefined();
    // The canvas explains how the files join.
    const canvas = screen.getByRole("region", { name: "Ontology preview" });
    expect(within(canvas).getByText("devices.csv.customer_id")).toBeDefined();
    expect(within(canvas).getByText("→ customers.csv.customer_id")).toBeDefined();
    expect(
      (within(inspector).getByRole("combobox", { name: "Source key" }) as HTMLSelectElement).value,
    ).toBe("customer_id");
  });

  it("generates: the ontology first, then each file with creators first", async () => {
    const api = fakeApi();
    await reviewed(api);
    fireEvent.click(screen.getByRole("button", { name: "Generate Graph" }));
    expect(await screen.findByText("Graph generated")).toBeDefined();

    expect(api.createOntology).toHaveBeenCalledTimes(1);
    const [, ontology, expectedVersion] = api.createOntology.mock.calls[0];
    expect(expectedVersion).toBe(2);
    expect(ontology.entity_types.map((t: { name: string }) => t.name)).toEqual([
      "Customer",
      "Device",
    ]);
    const order = api.startIngestion.mock.calls.map((call) => call[1]);
    expect(order).toEqual(["ds-customers", "ds-devices"]);
    expect(api.createOntology.mock.invocationCallOrder[0]).toBeLessThan(
      api.startIngestion.mock.invocationCallOrder[0],
    );
    const devices = api.startIngestion.mock.calls[1][2];
    expect(devices.entities[0]).toMatchObject({ alias: "customer", mode: "match_only" });
    expect(devices.relationships).toEqual([
      { relationship_type: "USES", source: "customer", target: "device", properties: [] },
    ]);
    expect(api.startIngestion.mock.calls[0][3]).not.toBe(api.startIngestion.mock.calls[1][3]);
    expect(screen.getByText("Relationships").nextSibling?.textContent).toBe("2");
    expect(screen.getByRole("link", { name: /Explore Graph/ }).getAttribute("href")).toBe(
      "/graphs/g1/explore",
    );
  });

  it("retries only the file that could not start, with the same key", async () => {
    const api = fakeApi();
    api.startIngestion
      .mockResolvedValueOnce({ job_id: "ingest-ds-customers", status: "queued" })
      .mockRejectedValueOnce(new ApiError(503, "SERVICE_UNAVAILABLE", "core is busy"));
    await reviewed(api);
    fireEvent.click(screen.getByRole("button", { name: "Generate Graph" }));
    expect(await screen.findByText(/Ingesting devices.csv could not start/)).toBeDefined();

    fireEvent.click(screen.getByRole("button", { name: "Retry ingestion" }));
    await screen.findByText("Graph generated");
    expect(api.createOntology).toHaveBeenCalledTimes(1);
    const calls = api.startIngestion.mock.calls.map((call) => [call[1], call[3]]);
    expect(calls.map(([id]) => id)).toEqual(["ds-customers", "ds-devices", "ds-devices"]);
    expect(calls[1][1]).toBe(calls[2][1]);
  });

  it("locks the review while the ontology is being saved", async () => {
    const api = fakeApi();
    let save: (value: Ontology) => void = () => {};
    api.createOntology.mockReturnValueOnce(
      new Promise((resolve) => {
        save = resolve;
      }),
    );
    await reviewed(api);
    fireEvent.click(screen.getByRole("button", { name: "Generate Graph" }));
    const saving = await screen.findByRole("button", { name: "Saving ontology…" });
    expect((saving as HTMLButtonElement).disabled).toBe(true);
    const review = screen.getByRole("group", { name: "Draft review" }) as HTMLFieldSetElement;
    expect(review.disabled).toBe(true);
    expect(
      (screen.getByRole("button", { name: "Add Entity" }) as HTMLButtonElement).matches(
        ":disabled",
      ),
    ).toBe(true);
    save(stored(SUGGESTION.ontology));
    expect(await screen.findByText("Graph generated")).toBeDefined();
  });

  it("explains a concurrent ontology change reported by the server", async () => {
    const api = fakeApi();
    api.createOntology.mockRejectedValueOnce(new ApiError(409, "CONFLICT", "stale"));
    await reviewed(api);
    fireEvent.click(screen.getByRole("button", { name: "Generate Graph" }));
    expect(
      await screen.findByText(/The ontology changed since this model was created/),
    ).toBeDefined();
    expect(api.startIngestion).not.toHaveBeenCalled();
  });

  it("keeps a finished generation finished when refreshing the graph fails", async () => {
    const api = fakeApi();
    await reviewed(api);
    api.getGraph.mockRejectedValue(new ApiError(502, "HTTP_ERROR", "Request failed with HTTP 502"));
    fireEvent.click(screen.getByRole("button", { name: "Generate Graph" }));
    expect(await screen.findByText("Graph generated")).toBeDefined();
    await waitFor(() => expect(api.getGraph).toHaveBeenCalledTimes(2));
    expect(screen.queryByRole("button", { name: "Retry ingestion" })).toBeNull();
    expect(api.startIngestion).toHaveBeenCalledTimes(2);
  });

  it("blocks generation while the model has problems", async () => {
    await reviewed(fakeApi());
    fireEvent.click(screen.getByRole("button", { name: "Add Entity" }));
    await waitFor(() =>
      expect(
        (screen.getByRole("button", { name: "Generate Graph" }) as HTMLButtonElement).disabled,
      ).toBe(true),
    );
    expect(screen.getByText(/problems? to fix/)).toBeDefined();
  });

  it("edits the ontology alone when there are no files", async () => {
    const api = fakeApi();
    api.getOntology.mockResolvedValue({
      ...ONTOLOGY,
      entity_types: [
        {
          id: "t1",
          name: "Customer",
          display_name: "Customer",
          description: null,
          properties: [{ key: "customer_id", type: "string", required: true }],
        },
      ],
    });
    open(api, []);
    const entities = await screen.findByRole("list", { name: "Entities" });
    expect(within(entities).getByText("existing")).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: "Add Entity" }));
    fireEvent.click(await screen.findByRole("button", { name: "Save Ontology" }));
    await screen.findByText("Graph generated");
    expect(api.createOntology).toHaveBeenCalledTimes(1);
    expect(api.startIngestion).not.toHaveBeenCalled();
  });
});
