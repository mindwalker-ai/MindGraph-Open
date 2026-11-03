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
} from "@/lib/api/types";

import { GraphBuilder } from "./GraphBuilder";

vi.mock("next/link", () => ({
  default: ({ href, children, ...props }: { href: string; children: ReactNode }) => (
    <a href={href} {...props}>
      {children}
    </a>
  ),
}));

afterEach(cleanup);

const ONTOLOGY: Ontology = {
  graph_id: "g1",
  version: 2,
  entity_types: [
    {
      id: "t1",
      name: "Customer",
      display_name: "Customer",
      description: null,
      properties: [
        { key: "customer_id", type: "string", required: true },
        { key: "name", type: "string", required: false },
      ],
    },
  ],
  relationship_types: [],
};

const PENDING: DataSource = {
  id: "d1",
  graph_id: "g1",
  kind: "csv",
  filename: "accounts.csv",
  status: "pending",
  size_bytes: 2048,
  content_sha256: "0".repeat(64),
  profile: null,
  created_at: "",
  updated_at: "",
};

const READY: DataSource = {
  ...PENDING,
  status: "ready",
  profile: {
    record_count: 7,
    fields: [
      {
        name: "account_id",
        inferred_type: "string",
        nullable: false,
        non_null_count: 7,
        sample_values: ["A1001"],
      },
      {
        name: "customer_id",
        inferred_type: "string",
        nullable: false,
        non_null_count: 7,
        sample_values: ["C001"],
      },
      {
        name: "balance",
        inferred_type: "float",
        nullable: true,
        non_null_count: 6,
        sample_values: ["1520.75"],
      },
    ],
    sample_rows: [],
  },
};

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
        name: "Account",
        display_name: "Account",
        properties: [
          { name: "account_id", type: "string", required: true },
          { name: "balance", type: "float", required: false },
        ],
      },
    ],
    relationship_types: [
      {
        name: "OWNS",
        source_entity_type: "Customer",
        target_entity_type: "Account",
        properties: [],
      },
    ],
  },
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
        alias: "account",
        entity_type: "Account",
        mode: "create_or_match",
        identity: [{ source_field: "account_id", property: "account_id" }],
        properties: [
          { source_field: "account_id", property: "account_id" },
          { source_field: "balance", property: "balance" },
        ],
      },
    ],
    relationships: [
      { relationship_type: "OWNS", source: "customer", target: "account", properties: [] },
    ],
  },
  confidence: 0.91,
  annotations: {
    entities: [
      {
        name: "Customer",
        confidence: 0.97,
        reason: "customer_id references Customer",
        existing: true,
      },
      {
        name: "Account",
        confidence: 0.94,
        reason: "account_id identifies each record",
        existing: false,
      },
    ],
    relationships: [
      { name: "OWNS", confidence: 0.91, reason: "co-occurring ids", existing: false },
    ],
  },
  warnings: ["balance is empty for one record"],
};

function job(kind: Job["kind"], status: Job["status"], extra: Partial<Job> = {}): Job {
  return {
    id: `${kind}-job`,
    graph_id: "g1",
    data_source_id: "d1",
    kind,
    status,
    progress: status === "completed" ? 1 : 0.5,
    ontology_version: kind === "analyze" ? null : 2,
    error: null,
    result: null,
    created_at: "",
    updated_at: "",
    ...extra,
  };
}

type FakeApi = { [K in keyof ApiClient]: ReturnType<typeof vi.fn> };

function fakeApi(status: GraphBuilderStatus["status"] = "ready"): FakeApi {
  const suggestionPolls = [
    job("ai_suggest", "running"),
    job("ai_suggest", "completed", {
      result: {
        prompt_version: "graph_builder_v1",
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
    getGraph: vi.fn().mockResolvedValue({
      id: "g1",
      workspace_id: "w",
      name: "Fraud",
      description: null,
      status: "active",
      created_at: "",
      updated_at: "",
    }),
    getOntology: vi.fn().mockResolvedValue(ONTOLOGY),
    createOntology: vi.fn().mockResolvedValue({ ...ONTOLOGY, version: 3 }),
    uploadDataSource: vi.fn().mockResolvedValue(PENDING),
    getDataSource: vi.fn().mockResolvedValue(READY),
    analyze: vi.fn().mockResolvedValue({ job_id: "analyze-job", status: "queued" }),
    graphBuilderStatus: vi.fn().mockResolvedValue({
      status,
      configured: status !== "not_configured",
      provider: "openai_compatible",
      model: "test-model",
    }),
    requestSuggestion: vi.fn().mockResolvedValue({ job_id: "ai_suggest-job", status: "queued" }),
    startIngestion: vi.fn().mockResolvedValue({ job_id: "ingest-job", status: "queued" }),
    getJob: vi.fn(async (id: string) => {
      if (id === "analyze-job") return job("analyze", "completed");
      if (id === "ai_suggest-job") {
        return suggestionPolls.length > 1 ? suggestionPolls.shift() : suggestionPolls[0];
      }
      return job("ingest", "completed", {
        result: {
          records_processed: 7,
          entities_created: 7,
          entities_matched: 5,
          relationships_created: 7,
        },
      });
    }),
  };
}

async function analysed(api: FakeApi) {
  render(<GraphBuilder graphId="g1" api={api as unknown as ApiClient} pollMs={1} />);
  await screen.findByText(/Fraud · Graph Builder/);
  fireEvent.change(screen.getByLabelText("Dataset file"), {
    target: { files: [new File(["account_id\nA1\n"], "accounts.csv", { type: "text/csv" })] },
  });
  fireEvent.click(screen.getByRole("button", { name: "Upload" }));
  fireEvent.click(await screen.findByRole("button", { name: "Analyze dataset" }));
  await screen.findByText("1520.75");
}

async function reviewed(api: FakeApi) {
  await analysed(api);
  fireEvent.click(await screen.findByRole("button", { name: "Generate Graph Model" }));
  await screen.findByRole("list", { name: "Entities" });
}

describe("GraphBuilder", () => {
  it("renders the analysed dataset profile", async () => {
    await analysed(fakeApi());
    expect(screen.getByText("accounts.csv")).toBeDefined();
    expect(screen.getByText("account_id")).toBeDefined();
    expect(screen.getByText("float")).toBeDefined();
    expect(screen.getByText(/records ·/)).toBeDefined();
  });

  it("offers the manual path when AI is not configured", async () => {
    const api = fakeApi("not_configured");
    await analysed(api);
    expect(screen.getByText(/AI Graph Builder is not configured\./)).toBeDefined();
    const suggest = screen.getByRole("button", { name: "Generate Graph Model" });
    expect((suggest as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Build manually" }));
    const entities = await screen.findByRole("list", { name: "Entities" });
    expect(within(entities).getByText("Account")).toBeDefined();
    expect(api.requestSuggestion).not.toHaveBeenCalled();
  });

  it("shows suggestion progress, then entities and relationships", async () => {
    const api = fakeApi();
    await analysed(api);
    fireEvent.click(screen.getByRole("button", { name: "Generate Graph Model" }));
    expect(await screen.findByLabelText("AI suggestion progress")).toBeDefined();
    const entities = await screen.findByRole("list", { name: "Entities" });
    const customer = within(entities).getByText("Customer").closest("button") as HTMLElement;
    expect(within(customer).getByText("Existing")).toBeDefined();
    expect(within(customer).getByText("97%")).toBeDefined();
    const account = within(entities).getByText("Account").closest("button") as HTMLElement;
    expect(within(account).getByText("New")).toBeDefined();
    const relationships = screen.getByRole("list", { name: "Relationships" });
    expect(within(relationships).getByText("Customer ─OWNS→ Account")).toBeDefined();
    expect(screen.getByText(/balance is empty for one record/)).toBeDefined();
    expect(typeof api.requestSuggestion.mock.calls[0][2]).toBe("string");
  });

  it("deletes a relationship from the draft", async () => {
    await reviewed(fakeApi());
    fireEvent.click(screen.getByRole("button", { name: "Delete relationship OWNS" }));
    expect(screen.getByText("No relationships.")).toBeDefined();
  });

  it("generates the graph: ontology first, then the edited mapping as an ingestion", async () => {
    const api = fakeApi();
    await reviewed(api);
    // Edit a mapping: Account.balance is no longer mapped.
    const entities = screen.getByRole("list", { name: "Entities" });
    fireEvent.click(within(entities).getByText("Account"));
    fireEvent.change(screen.getByLabelText("Source field of balance"), { target: { value: "" } });

    fireEvent.click(screen.getByRole("button", { name: "Generate Graph" }));
    expect(await screen.findByText("Graph generated")).toBeDefined();
    expect(screen.getByText("Entities matched")).toBeDefined();
    expect(screen.getByRole("link", { name: "Explore Graph" }).getAttribute("href")).toBe(
      "/graphs/g1/explore",
    );

    expect(api.createOntology).toHaveBeenCalledTimes(1);
    const ontology = api.createOntology.mock.calls[0][1];
    expect(ontology.entity_types.map((t: { name: string }) => t.name)).toEqual([
      "Customer",
      "Account",
    ]);
    expect(api.createOntology.mock.invocationCallOrder[0]).toBeLessThan(
      api.startIngestion.mock.invocationCallOrder[0],
    );
    const [, , mapping, key] = api.startIngestion.mock.calls[0];
    expect(mapping.entities[0]).toMatchObject({ alias: "customer", mode: "match_only" });
    expect(mapping.entities[1].properties).toEqual([
      { source_field: "account_id", property: "account_id" },
    ]);
    expect(typeof key).toBe("string");
  });

  it("keeps the saved ontology and retries only the ingestion", async () => {
    const api = fakeApi();
    api.startIngestion
      .mockRejectedValueOnce(new ApiError(503, "SERVICE_UNAVAILABLE", "core is busy"))
      .mockResolvedValueOnce({ job_id: "ingest-job", status: "queued" });
    await reviewed(api);
    fireEvent.click(screen.getByRole("button", { name: "Generate Graph" }));
    expect(
      await screen.findByText(/Ontology saved successfully\. Graph generation could not start/),
    ).toBeDefined();

    fireEvent.click(screen.getByRole("button", { name: "Retry ingestion" }));
    await screen.findByText("Graph generated");
    expect(api.createOntology).toHaveBeenCalledTimes(1);
    expect(api.startIngestion).toHaveBeenCalledTimes(2);
    expect(api.startIngestion.mock.calls[0][3]).toBe(api.startIngestion.mock.calls[1][3]);
  });

  it("blocks generation while the draft is invalid", async () => {
    await reviewed(fakeApi());
    fireEvent.click(screen.getByRole("button", { name: "+ Add entity" }));
    await waitFor(() =>
      expect(
        (screen.getByRole("button", { name: "Generate Graph" }) as HTMLButtonElement).disabled,
      ).toBe(true),
    );
    expect(screen.getByRole("list", { name: "Draft problems" })).toBeDefined();
  });
});
