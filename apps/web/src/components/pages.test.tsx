// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { CreateGraph } from "@/components/create/CreateGraph";
import { HomeScreen } from "@/components/home/HomeScreen";
import { UploadData } from "@/components/upload/UploadData";
import type { ApiClient } from "@/lib/api/client";
import type { DataSource, GraphProject, Job } from "@/lib/api/types";

const push = vi.fn();

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
  usePathname: () => "/",
  useRouter: () => ({ push }),
}));

afterEach(() => {
  cleanup();
  push.mockReset();
});

const GRAPH: GraphProject = {
  id: "g1",
  workspace_id: "w",
  name: "Fraud Investigation",
  description: null,
  status: "active",
  created_at: "2026-11-04T00:00:00Z",
  updated_at: "2026-11-04T00:00:00Z",
};

function api(overrides: Partial<Record<keyof ApiClient, unknown>> = {}): ApiClient {
  return {
    listGraphs: vi.fn().mockResolvedValue([GRAPH]),
    getGraph: vi.fn().mockResolvedValue(GRAPH),
    getOntology: vi.fn().mockResolvedValue({
      graph_id: "g1",
      version: 2,
      entity_types: [
        { id: "t", name: "Customer", display_name: "Customer", description: null, properties: [] },
      ],
      relationship_types: [],
    }),
    createGraph: vi.fn().mockResolvedValue(GRAPH),
    ...overrides,
  } as unknown as ApiClient;
}

describe("HomeScreen", () => {
  it("welcomes a new installation", async () => {
    render(<HomeScreen api={api({ listGraphs: vi.fn().mockResolvedValue([]) })} />);
    expect(await screen.findByText(/explorable knowledge graph/)).toBeDefined();
    expect(screen.getByRole("link", { name: /Create Graph/ }).getAttribute("href")).toBe(
      "/graphs/new",
    );
    expect(screen.getByText("Explore Demo").closest("[aria-disabled]")).not.toBeNull();
  });

  it("lists graphs with their ontology and keeps later features disabled", async () => {
    render(<HomeScreen api={api()} />);
    const graphs = await screen.findByRole("region", { name: "Graphs" });
    const card = within(graphs).getByText("Fraud Investigation").closest("a");
    expect(card?.getAttribute("href")).toBe("/graphs/g1/upload");
    expect(await within(graphs).findByRole("img", { name: /1 entity types/ })).toBeDefined();
    expect(within(graphs).getByText(/ontology v2/)).toBeDefined();
    for (const button of screen.getAllByRole("button", { name: /Use template/ })) {
      expect((button as HTMLButtonElement).disabled).toBe(true);
    }
    expect((screen.getByRole("button", { name: /Set up MCP/ }) as HTMLButtonElement).disabled).toBe(
      true,
    );
  });
});

describe("CreateGraph", () => {
  it("creates the graph and continues to the chosen start", async () => {
    const client = api();
    render(<CreateGraph api={client} />);
    const name = screen.getByRole("textbox", { name: "Graph name" });
    expect((screen.getByRole("button", { name: /Continue/ }) as HTMLButtonElement).disabled).toBe(
      true,
    );
    fireEvent.change(name, { target: { value: "  Fraud Investigation " } });
    fireEvent.click(screen.getByRole("radio", { name: /Upload JSON/ }));
    expect(
      (screen.getByRole("radio", { name: /Explore Demo/ }) as HTMLButtonElement).disabled,
    ).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: /Continue/ }));
    await waitFor(() => expect(push).toHaveBeenCalledWith("/graphs/g1/upload?kind=json"));
    expect(client.createGraph).toHaveBeenCalledWith("Fraud Investigation");
  });

  it("opens the ontology editor for an empty start", async () => {
    render(<CreateGraph api={api()} />);
    fireEvent.change(screen.getByRole("textbox", { name: "Graph name" }), {
      target: { value: "Empty" },
    });
    fireEvent.click(screen.getByRole("radio", { name: /Start Empty/ }));
    fireEvent.click(screen.getByRole("button", { name: /Continue/ }));
    await waitFor(() => expect(push).toHaveBeenCalledWith("/graphs/g1/builder"));
  });
});

function dataSource(id: string, filename: string, status: DataSource["status"]): DataSource {
  return {
    id,
    graph_id: "g1",
    kind: "csv",
    filename,
    status,
    size_bytes: 10,
    content_sha256: "0".repeat(64),
    profile:
      status === "ready"
        ? {
            record_count: 2,
            fields: [
              {
                name: "customer_id",
                inferred_type: "string",
                nullable: false,
                non_null_count: 2,
                sample_values: [],
              },
            ],
            sample_rows: [{ customer_id: "C001" }, { customer_id: "C002" }],
          }
        : null,
    created_at: "",
    updated_at: "",
  };
}

describe("UploadData", () => {
  it("uploads and parses several files, then opens the builder on all of them", async () => {
    const parsed = new Set<string>();
    const client = api({
      uploadDataSource: vi.fn(async (_g: string, file: File) =>
        dataSource(`ds-${file.name}`, file.name, "pending"),
      ),
      analyze: vi.fn(async (_g: string, id: string) => {
        parsed.add(id);
        return { job_id: `job-${id}`, status: "queued" };
      }),
      getJob: vi.fn(async (id: string): Promise<Partial<Job>> => ({ id, status: "completed" })),
      getDataSource: vi.fn(async (_g: string, id: string) =>
        dataSource(id, id.slice(3), parsed.has(id) ? "ready" : "pending"),
      ),
    });
    render(<UploadData graphId="g1" api={client} pollMs={1} />);
    fireEvent.change(screen.getByLabelText("Dataset files"), {
      target: { files: [new File(["a"], "customers.csv"), new File(["b"], "devices.csv")] },
    });
    const files = await screen.findByRole("region", { name: "Files" });
    await waitFor(() => expect(within(files).getAllByText("Parsed")).toHaveLength(2));
    expect(within(files).getByText(/2 files · 4 rows · 2 columns/)).toBeDefined();
    expect(screen.getByRole("region", { name: "Preview" })).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: /Analyze Data/ }));
    expect(push).toHaveBeenCalledWith("/graphs/g1/builder?sources=ds-customers.csv,ds-devices.csv");
  });

  it("restores the files of the URL", async () => {
    const client = api({
      getDataSource: vi.fn(async (_g: string, id: string) => dataSource(id, `${id}.csv`, "ready")),
    });
    render(<UploadData graphId="g1" initialSources={["a", "b"]} api={client} pollMs={1} />);
    const files = await screen.findByRole("region", { name: "Files" });
    await waitFor(() => expect(within(files).getAllByText("Parsed")).toHaveLength(2));
    expect(within(files).getByText("a.csv")).toBeDefined();
  });
});
