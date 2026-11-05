/**
 * Copyable `mindgraph-mcp` configurations for MCP clients (M8).
 *
 * Every client launches its own `mindgraph-mcp` process over stdio, so the
 * snippets only name the binary and its environment: the graph id of the
 * current page, the Core URL on loopback, and the opt-in `ask_graph` flag.
 * No credentials exist to fill in. Formats follow each client's documented
 * stdio configuration (Claude Code `.mcp.json` and `claude mcp add`, Codex
 * `~/.codex/config.toml` and `codex mcp add`, Cursor `.cursor/mcp.json`).
 */

export type McpClientId = "claude" | "codex" | "cursor" | "custom";

export interface McpSettings {
  /** Absolute path of the `mindgraph-mcp` binary. */
  binary: string;
  /** MindGraph Core on a loopback address, e.g. `http://127.0.0.1:8080`. */
  coreUrl: string;
  graphId: string;
  /** Registers `ask_graph`, which sends graph evidence to Core's model provider. */
  enableAsk: boolean;
}

export interface Snippet {
  /** Where the snippet goes: a file path, or "Terminal". */
  label: string;
  language: "json" | "toml" | "shell";
  text: string;
}

export interface McpClient {
  id: McpClientId;
  name: string;
  /** Short monospace subtitle of the client card. */
  kind: string;
  initials: string;
  snippets: (settings: McpSettings) => Snippet[];
  /** How to reload the client and find the tools. */
  reload: string;
  discover: string;
}

export const DEFAULT_CORE_URL = "http://127.0.0.1:8080";
export const DEFAULT_BINARY = "/path/to/MindGraph-Open/target/release/mindgraph-mcp";
export const SERVER_NAME = "mindgraph";
export const EXAMPLE_PROMPT = "Find Customer C001 and show its relationships.";

/** The server's environment: only what differs from its defaults, plus the graph. */
export function mcpEnv(settings: McpSettings): Record<string, string> {
  const env: Record<string, string> = {
    MINDGRAPH_MCP_CORE_URL: settings.coreUrl.trim() || DEFAULT_CORE_URL,
    MINDGRAPH_MCP_GRAPH_ID: settings.graphId,
  };
  if (settings.enableAsk) env.MINDGRAPH_MCP_ENABLE_ASK = "true";
  return env;
}

function binary(settings: McpSettings): string {
  return settings.binary.trim() || DEFAULT_BINARY;
}

/** Quotes a shell word only when it needs it. */
export function shellWord(word: string): string {
  return /^[\w@%+=:,./-]+$/.test(word) ? word : `'${word.replaceAll("'", `'\\''`)}'`;
}

function jsonConfig(settings: McpSettings): string {
  return JSON.stringify(
    {
      mcpServers: {
        [SERVER_NAME]: {
          type: "stdio",
          command: binary(settings),
          args: [],
          env: mcpEnv(settings),
        },
      },
    },
    null,
    2,
  );
}

function addCommand(program: string, flags: string[], settings: McpSettings): string {
  const env = Object.entries(mcpEnv(settings)).map(
    ([key, value]) => `  --env ${shellWord(`${key}=${value}`)} \\`,
  );
  return [
    `${program} ${[...flags, SERVER_NAME].join(" ")} \\`,
    ...env,
    `  -- ${shellWord(binary(settings))}`,
  ].join("\n");
}

/** TOML basic strings use JSON-compatible escapes. */
function tomlString(value: string): string {
  return JSON.stringify(value);
}

function codexToml(settings: McpSettings): string {
  const env = Object.entries(mcpEnv(settings)).map(
    ([key, value]) => `${key} = ${tomlString(value)}`,
  );
  return [
    `[mcp_servers.${SERVER_NAME}]`,
    `command = ${tomlString(binary(settings))}`,
    "",
    `[mcp_servers.${SERVER_NAME}.env]`,
    ...env,
  ].join("\n");
}

function customShell(settings: McpSettings): string {
  const env = Object.entries(mcpEnv(settings)).map(
    ([key, value]) => `export ${key}=${shellWord(value)}`,
  );
  return [
    "# Your MCP client starts this command and speaks MCP over its stdin/stdout.",
    ...env,
    shellWord(binary(settings)),
    "",
    "# Check the configuration without starting MCP:",
    `${shellWord(binary(settings))} --check`,
  ].join("\n");
}

export const MCP_CLIENTS: McpClient[] = [
  {
    id: "claude",
    name: "Claude Code",
    kind: "CLI · IDE",
    initials: "Cl",
    snippets: (settings) => [
      {
        label: "Terminal (in your project)",
        language: "shell",
        text: addCommand("claude mcp add", ["--scope", "project"], settings),
      },
      { label: ".mcp.json (project scope)", language: "json", text: jsonConfig(settings) },
    ],
    reload: "Start a new claude session in the project and approve the mindgraph server.",
    discover: "Run /mcp to see mindgraph and its tools.",
  },
  {
    id: "codex",
    name: "Codex",
    kind: "CLI · IDE",
    initials: "Cx",
    snippets: (settings) => [
      { label: "Terminal", language: "shell", text: addCommand("codex mcp add", [], settings) },
      { label: "~/.codex/config.toml", language: "toml", text: codexToml(settings) },
    ],
    reload: "Start a new Codex session.",
    discover: "Run /mcp in the Codex TUI, or codex mcp list in a terminal.",
  },
  {
    id: "cursor",
    name: "Cursor",
    kind: "Editor",
    initials: "Cu",
    snippets: (settings) => [
      { label: ".cursor/mcp.json", language: "json", text: jsonConfig(settings) },
    ],
    reload: "Save the file and restart Cursor.",
    discover: "Open Customize › MCPs in the sidebar and enable mindgraph.",
  },
  {
    id: "custom",
    name: "Custom MCP Client",
    kind: "stdio",
    initials: "{ }",
    snippets: (settings) => [
      { label: "Command and environment", language: "shell", text: customShell(settings) },
      { label: "mcpServers JSON (most clients)", language: "json", text: jsonConfig(settings) },
    ],
    reload: "Restart or reload your client.",
    discover: "List the tools with your client's tools/list.",
  },
];

export interface McpTool {
  name: string;
  description: string;
  /** Only registered with MINDGRAPH_MCP_ENABLE_ASK=true. */
  optIn?: boolean;
}

/** The tools `mindgraph-mcp` registers (crates/mindgraph-mcp/src/tools.rs). */
export const MCP_TOOLS: McpTool[] = [
  { name: "get_graph_info", description: "Name, description and status of this graph" },
  {
    name: "get_graph_schema",
    description: "Entity types, relationship types and their properties",
  },
  {
    name: "search_entities",
    description: "Exact, typed property matches (not full-text or semantic)",
  },
  { name: "get_entity", description: "One entity by its MindGraph id" },
  { name: "get_neighbors", description: "Relationships one hop around an entity" },
  { name: "find_path", description: "Bounded shortest path between two entities (≤ 6 hops)" },
  { name: "get_subgraph", description: "Up to 100 entities and the relationships among them" },
  {
    name: "ask_graph",
    description: "GraphRAG answer with cited evidence; sends graph data to Core's model provider",
    optIn: true,
  },
];

/** The URL of a loopback Core, or the default when `raw` is not one. */
export function loopbackCoreUrl(raw: string | undefined): string {
  if (!raw) return DEFAULT_CORE_URL;
  try {
    const url = new URL(raw);
    const host = url.hostname.replace(/^\[|\]$/g, "");
    const loopback = host === "localhost" || host === "::1" || /^127\./.test(host);
    return url.protocol === "http:" && loopback ? url.origin : DEFAULT_CORE_URL;
  } catch {
    return DEFAULT_CORE_URL;
  }
}
