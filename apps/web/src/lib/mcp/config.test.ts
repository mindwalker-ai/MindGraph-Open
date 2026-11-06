import { describe, expect, it } from "vitest";

import {
  DEFAULT_BINARY,
  DEFAULT_CORE_URL,
  MCP_CLIENTS,
  MCP_TOOLS,
  type McpSettings,
  loopbackCoreUrl,
  mcpEnv,
  shellWord,
} from "./config";

const GRAPH = "01a1ac2c-48af-7425-b4e6-0dd4cfad2039";
const SETTINGS: McpSettings = {
  binary: "/opt/mindgraph/target/release/mindgraph-mcp",
  coreUrl: "http://127.0.0.1:18088",
  graphId: GRAPH,
  enableAsk: false,
};

function snippets(id: string, settings: McpSettings = SETTINGS) {
  const client = MCP_CLIENTS.find((candidate) => candidate.id === id);
  if (!client) throw new Error(id);
  return client.snippets(settings);
}

describe("mcp configuration snippets", () => {
  it("names only the graph, the loopback Core and the opt-in flag", () => {
    expect(mcpEnv(SETTINGS)).toEqual({
      MINDGRAPH_MCP_CORE_URL: "http://127.0.0.1:18088",
      MINDGRAPH_MCP_GRAPH_ID: GRAPH,
    });
    expect(mcpEnv({ ...SETTINGS, enableAsk: true }).MINDGRAPH_MCP_ENABLE_ASK).toBe("true");
    expect(mcpEnv({ ...SETTINGS, coreUrl: "  " }).MINDGRAPH_MCP_CORE_URL).toBe(DEFAULT_CORE_URL);
  });

  it("writes a project .mcp.json and a claude mcp add command for Claude Code", () => {
    const [command, file] = snippets("claude");
    expect(command.text).toBe(
      [
        "claude mcp add --scope project mindgraph \\",
        "  --env MINDGRAPH_MCP_CORE_URL=http://127.0.0.1:18088 \\",
        `  --env MINDGRAPH_MCP_GRAPH_ID=${GRAPH} \\`,
        "  -- /opt/mindgraph/target/release/mindgraph-mcp",
      ].join("\n"),
    );
    expect(JSON.parse(file.text)).toEqual({
      mcpServers: {
        mindgraph: {
          type: "stdio",
          command: "/opt/mindgraph/target/release/mindgraph-mcp",
          args: [],
          env: { MINDGRAPH_MCP_CORE_URL: "http://127.0.0.1:18088", MINDGRAPH_MCP_GRAPH_ID: GRAPH },
        },
      },
    });
  });

  it("writes an mcp_servers table for Codex", () => {
    const [command, toml] = snippets("codex", { ...SETTINGS, enableAsk: true });
    expect(command.text.startsWith("codex mcp add mindgraph \\")).toBe(true);
    expect(command.text).toContain("--env MINDGRAPH_MCP_ENABLE_ASK=true");
    expect(toml.label).toBe("~/.codex/config.toml");
    expect(toml.text).toBe(
      [
        "[mcp_servers.mindgraph]",
        'command = "/opt/mindgraph/target/release/mindgraph-mcp"',
        "",
        "[mcp_servers.mindgraph.env]",
        'MINDGRAPH_MCP_CORE_URL = "http://127.0.0.1:18088"',
        `MINDGRAPH_MCP_GRAPH_ID = "${GRAPH}"`,
        'MINDGRAPH_MCP_ENABLE_ASK = "true"',
      ].join("\n"),
    );
  });

  it("writes .cursor/mcp.json for Cursor and a plain command for other clients", () => {
    const [cursor] = snippets("cursor");
    expect(cursor.label).toBe(".cursor/mcp.json");
    expect(JSON.parse(cursor.text).mcpServers.mindgraph.type).toBe("stdio");
    const [custom] = snippets("custom");
    expect(custom.text).toContain(`export MINDGRAPH_MCP_GRAPH_ID=${GRAPH}`);
    expect(custom.text).toContain("/opt/mindgraph/target/release/mindgraph-mcp --check");
  });

  it("quotes paths with spaces and escapes them in JSON and TOML", () => {
    const spaced = { ...SETTINGS, binary: "/Users/me/Mind Graph/it's/mindgraph-mcp" };
    expect(shellWord(spaced.binary)).toBe(`'/Users/me/Mind Graph/it'\\''s/mindgraph-mcp'`);
    expect(snippets("claude", spaced)[0].text).toContain(
      `-- '/Users/me/Mind Graph/it'\\''s/mindgraph-mcp'`,
    );
    expect(JSON.parse(snippets("cursor", spaced)[0].text).mcpServers.mindgraph.command).toBe(
      spaced.binary,
    );
    expect(snippets("codex", { ...SETTINGS, binary: 'C:\\mg\\"x"' })[1].text).toContain(
      'command = "C:\\\\mg\\\\\\"x\\""',
    );
    expect(snippets("cursor", { ...SETTINGS, binary: "" })[0].text).toContain(DEFAULT_BINARY);
  });

  it("never contains credentials, an HTTP endpoint or npx", () => {
    for (const client of MCP_CLIENTS) {
      for (const snippet of client.snippets({ ...SETTINGS, enableAsk: true })) {
        expect(snippet.text).not.toMatch(/api[_-]?key|token|password|secret|npx|\/mcp\b/i);
      }
    }
  });

  it("lists the seven read-only tools and the opt-in ask_graph", () => {
    expect(MCP_TOOLS.map((tool) => tool.name)).toEqual([
      "get_graph_info",
      "get_graph_schema",
      "search_entities",
      "get_entity",
      "get_neighbors",
      "find_path",
      "get_subgraph",
      "ask_graph",
    ]);
    expect(MCP_TOOLS.filter((tool) => tool.optIn).map((tool) => tool.name)).toEqual(["ask_graph"]);
  });

  it("suggests the web app's Core URL only when it is on loopback", () => {
    expect(loopbackCoreUrl("http://127.0.0.1:18088")).toBe("http://127.0.0.1:18088");
    expect(loopbackCoreUrl("http://localhost:8080/")).toBe("http://localhost:8080");
    expect(loopbackCoreUrl("http://core:8080")).toBe(DEFAULT_CORE_URL);
    expect(loopbackCoreUrl("https://127.0.0.1:8080")).toBe(DEFAULT_CORE_URL);
    expect(loopbackCoreUrl("not a url")).toBe(DEFAULT_CORE_URL);
    expect(loopbackCoreUrl(undefined)).toBe(DEFAULT_CORE_URL);
  });
});
