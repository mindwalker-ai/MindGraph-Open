// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { McpIntegration } from "@/components/integrations/McpIntegration";
import { type ApiClient, ApiError } from "@/lib/api/client";
import type { GraphProject } from "@/lib/api/types";

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
  usePathname: () => "/graphs/g1/integrations/mcp",
  useRouter: () => ({ push: vi.fn() }),
}));

afterEach(() => {
  cleanup();
  window.localStorage.clear();
});

const GRAPH_ID = "01a1ac2c-48af-7425-b4e6-0dd4cfad2039";
const GRAPH: GraphProject = {
  id: GRAPH_ID,
  workspace_id: "w",
  name: "Fraud Investigation",
  description: null,
  status: "active",
  created_at: "2026-11-05T00:00:00Z",
  updated_at: "2026-11-05T00:00:00Z",
};

function api(overrides: Partial<Record<keyof ApiClient, unknown>> = {}): ApiClient {
  return {
    getGraph: vi.fn().mockResolvedValue(GRAPH),
    graphRagStatus: vi.fn().mockResolvedValue({ status: "not_configured", configured: false }),
    ...overrides,
  } as unknown as ApiClient;
}

function snippet(label: string): string {
  return within(screen.getByRole("region", { name: label })).getByRole("code").textContent ?? "";
}

describe("McpIntegration", () => {
  it("builds the Claude Code configuration for this graph", async () => {
    render(
      <McpIntegration graphId={GRAPH_ID} defaultCoreUrl="http://127.0.0.1:18088" api={api()} />,
    );
    expect(screen.getByRole("heading", { name: "Connect MindGraph to your AI" })).toBeDefined();
    expect(await screen.findByText("Core reachable")).toBeDefined();
    expect(screen.getAllByText("Fraud Investigation").length).toBeGreaterThan(0);
    expect(screen.getByRole("radio", { name: /Claude Code/ }).getAttribute("aria-checked")).toBe(
      "true",
    );
    const file = snippet(".mcp.json (project scope)");
    expect(file).toContain(`"MINDGRAPH_MCP_GRAPH_ID": "${GRAPH_ID}"`);
    expect(file).toContain(`"MINDGRAPH_MCP_CORE_URL": "http://127.0.0.1:18088"`);
    expect(snippet("Terminal (in your project)")).toContain("claude mcp add --scope project");
    // Nothing claims a client connection the browser cannot observe.
    expect(screen.queryByText(/^Connected$/)).toBeNull();
    expect(screen.getByText(/cannot see which clients are connected/)).toBeDefined();
  });

  it("switches clients, the binary path and the ask_graph opt-in", async () => {
    render(
      <McpIntegration graphId={GRAPH_ID} defaultCoreUrl="http://127.0.0.1:8080" api={api()} />,
    );
    fireEvent.click(screen.getByRole("radio", { name: /Codex/ }));
    expect(snippet("~/.codex/config.toml")).toContain("[mcp_servers.mindgraph]");
    fireEvent.change(screen.getByLabelText("Path to mindgraph-mcp"), {
      target: { value: "/opt/mg/mindgraph-mcp" },
    });
    expect(snippet("~/.codex/config.toml")).toContain('command = "/opt/mg/mindgraph-mcp"');
    expect(window.localStorage.getItem("mindgraph.mcp.binary")).toBe("/opt/mg/mindgraph-mcp");

    const tools = screen.getByRole("complementary", { name: "Available tools" });
    expect(within(tools).getByText("7 enabled")).toBeDefined();
    expect(within(tools).getByText("opt-in · off")).toBeDefined();
    expect(within(tools).getAllByText("read-only")).toHaveLength(7);
    expect(within(tools).getByText(/No model configured in Core/)).toBeDefined();
    fireEvent.click(screen.getByRole("checkbox", { name: /Enable ask_graph/ }));
    expect(within(tools).getByText("8 enabled")).toBeDefined();
    expect(snippet("~/.codex/config.toml")).toContain('MINDGRAPH_MCP_ENABLE_ASK = "true"');

    fireEvent.click(screen.getByRole("radio", { name: /Cursor/ }));
    expect(snippet(".cursor/mcp.json")).toContain('"MINDGRAPH_MCP_ENABLE_ASK": "true"');
    expect(screen.getByText(/Customize › MCPs/)).toBeDefined();
  });

  it("copies the configuration", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    render(
      <McpIntegration graphId={GRAPH_ID} defaultCoreUrl="http://127.0.0.1:8080" api={api()} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Copy .mcp.json (project scope)" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledOnce());
    expect(JSON.parse(writeText.mock.calls[0][0] as string).mcpServers.mindgraph.env).toEqual({
      MINDGRAPH_MCP_CORE_URL: "http://127.0.0.1:8080",
      MINDGRAPH_MCP_GRAPH_ID: GRAPH_ID,
    });
    expect(
      await screen.findByRole("button", { name: "Copy .mcp.json (project scope)" }),
    ).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: "Copy example request" }));
    await waitFor(() =>
      expect(writeText).toHaveBeenLastCalledWith("Find Customer C001 and show its relationships."),
    );
  });

  it("says when Core cannot be reached or the graph does not exist", async () => {
    const getGraph = vi
      .fn()
      .mockRejectedValueOnce(
        new ApiError(502, "CORE_UNAVAILABLE", "MindGraph Core cannot be reached"),
      )
      .mockRejectedValueOnce(new ApiError(404, "GRAPH_NOT_FOUND", "Graph was not found"));
    render(
      <McpIntegration
        graphId={GRAPH_ID}
        defaultCoreUrl="http://127.0.0.1:8080"
        api={api({ getGraph })}
      />,
    );
    expect(await screen.findByText("Core unreachable")).toBeDefined();
    fireEvent.click(screen.getByRole("button", { name: /Check again/ }));
    expect(await screen.findByText("Graph not found")).toBeDefined();
    expect(screen.getByRole("alert").textContent).toContain("no graph with this id");
  });

  it("shows the model Core would use for ask_graph", async () => {
    render(
      <McpIntegration
        graphId={GRAPH_ID}
        defaultCoreUrl="http://127.0.0.1:8080"
        api={api({
          graphRagStatus: vi.fn().mockResolvedValue({
            status: "ready",
            configured: true,
            provider: "openai_compatible",
            model: "local-model",
          }),
        })}
      />,
    );
    expect(await screen.findByText("Core model: local-model")).toBeDefined();
  });
});
