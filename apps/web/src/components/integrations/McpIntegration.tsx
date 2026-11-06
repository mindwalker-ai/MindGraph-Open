"use client";

/**
 * Screen 10 of the mockup, "Connect MindGraph to your AI" (M8).
 *
 * A configuration screen: it builds copyable `mindgraph-mcp` settings for
 * Claude Code, Codex, Cursor or any stdio client from this graph's id. It
 * never claims a client is connected: each client starts its own
 * `mindgraph-mcp` process over stdio, which the browser cannot observe. The
 * only live status is whether MindGraph Core answers (through the web app)
 * and whether Core has a model for the opt-in `ask_graph` tool.
 */

import { Check, Copy, FileCode2, Lock, RefreshCw, Sparkles, Terminal } from "lucide-react";
import {
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useState,
  useSyncExternalStore,
} from "react";

import { AppShell } from "@/components/shell/AppShell";
import { Badge, Button, Field, Kicker, inputClass } from "@/components/ui/primitives";
import { type ApiClient, ApiError, createApiClient } from "@/lib/api/client";
import type { GraphBuilderStatus, GraphProject } from "@/lib/api/types";
import {
  DEFAULT_BINARY,
  EXAMPLE_PROMPT,
  MCP_CLIENTS,
  MCP_TOOLS,
  type McpClientId,
  type McpSettings,
  type Snippet,
} from "@/lib/mcp/config";
import { cn } from "@/lib/utils";

const BINARY_KEY = "mindgraph.mcp.binary";
const CORE_URL_KEY = "mindgraph.mcp.coreUrl";

type CoreState =
  | { state: "checking" }
  | { state: "reachable" }
  | { state: "unreachable"; message: string }
  | { state: "graph_missing" };

function readSetting(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function writeSetting(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // Storage may be unavailable (private window); the setting just is not kept.
  }
}

/** A value this browser saved; `null` on the server and when nothing is saved. */
function useSavedSetting(key: string): string | null {
  return useSyncExternalStore(
    () => () => {},
    () => readSetting(key) || null,
    () => null,
  );
}

function CopyButton({ text, label }: { text: string; label: string }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1500);
    return () => window.clearTimeout(timer);
  }, [copied]);
  return (
    <Button
      size="sm"
      aria-label={label}
      icon={copied ? <Check size={12} /> : <Copy size={12} />}
      onClick={() => void navigator.clipboard?.writeText(text).then(() => setCopied(true))}
    >
      {copied ? "Copied" : "Copy"}
    </Button>
  );
}

function CodeBlock({ snippet }: { snippet: Snippet }) {
  const lines = snippet.text.split("\n");
  const Icon = snippet.language === "shell" ? Terminal : FileCode2;
  return (
    <section
      aria-label={snippet.label}
      className="overflow-hidden rounded-[4px] border border-border bg-panel"
    >
      <header className="flex items-center gap-2 border-b border-border px-3 py-2">
        <Icon size={13} className="text-text-3" />
        <span className="flex-1 truncate font-mono text-[11px] text-text-2">{snippet.label}</span>
        <CopyButton text={snippet.text} label={`Copy ${snippet.label}`} />
      </header>
      <pre className="overflow-x-auto py-3 font-mono text-[11.5px] leading-[21px]">
        <code>
          {lines.map((line, index) => (
            <span key={index} className="flex">
              <span aria-hidden className="w-9 shrink-0 pr-3 text-right text-text-3 select-none">
                {index + 1}
              </span>
              <span className="pr-4 whitespace-pre text-text">{line}</span>
            </span>
          ))}
        </code>
      </pre>
    </section>
  );
}

function Step({
  number,
  title,
  children,
}: {
  number: number;
  title: string;
  children?: ReactNode;
}) {
  return (
    <li className="flex gap-3">
      <span className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full border border-border-strong font-mono text-[10px] text-text-2">
        {number}
      </span>
      <div className="flex min-w-0 flex-col gap-1">
        <span className="text-[13px] text-text">{title}</span>
        {children && <div className="text-[12px] text-text-3">{children}</div>}
      </div>
    </li>
  );
}

function InlineCommand({ command }: { command: string }) {
  return (
    <span className="flex items-center gap-2">
      <code className="rounded-[3px] border border-border bg-bg px-[6px] py-px font-mono text-[11px] text-text-2">
        {command}
      </code>
      <CopyButton text={command} label={`Copy ${command}`} />
    </span>
  );
}

function CoreStatus({ core, onRetry }: { core: CoreState; onRetry: () => void }) {
  const [tone, text] =
    core.state === "reachable"
      ? (["ok", "Core reachable"] as const)
      : core.state === "checking"
        ? (["neutral", "Checking Core…"] as const)
        : core.state === "graph_missing"
          ? (["danger", "Graph not found"] as const)
          : (["danger", "Core unreachable"] as const);
  return (
    <div className="flex items-center gap-2">
      <span
        role="status"
        title={core.state === "unreachable" ? core.message : "Checked through the web app"}
        className={cn(
          "flex h-8 items-center gap-2 rounded-[4px] border px-3 text-[12.5px] font-semibold",
          tone === "ok" && "border-ok/40 bg-ok/10 text-ok",
          tone === "danger" && "border-danger/40 bg-danger/10 text-danger",
          tone === "neutral" && "border-border-strong text-text-3",
        )}
      >
        <span
          className={cn(
            "h-1.5 w-1.5 rounded-full",
            tone === "ok" ? "bg-ok" : tone === "danger" ? "bg-danger" : "bg-text-3",
          )}
        />
        {text}
      </span>
      <Button icon={<RefreshCw size={13} />} onClick={onRetry} disabled={core.state === "checking"}>
        Check again
      </Button>
    </div>
  );
}

export function McpIntegration({
  graphId,
  defaultCoreUrl,
  api: injected,
}: {
  graphId: string;
  /** The Core URL to suggest: Core's loopback URL when the web app knows it. */
  defaultCoreUrl: string;
  api?: ApiClient;
}) {
  const api = useMemo(() => injected ?? createApiClient(), [injected]);
  const [graph, setGraph] = useState<GraphProject | null>(null);
  const [core, setCore] = useState<CoreState>({ state: "checking" });
  const [model, setModel] = useState<GraphBuilderStatus | null>(null);
  const [clientId, setClientId] = useState<McpClientId>("claude");
  // The paths this browser saved earlier (none during server rendering),
  // until the user edits them on this page.
  const savedBinary = useSavedSetting(BINARY_KEY);
  const savedCoreUrl = useSavedSetting(CORE_URL_KEY);
  const [editedBinary, setEditedBinary] = useState<string | null>(null);
  const [editedCoreUrl, setEditedCoreUrl] = useState<string | null>(null);
  const binary = editedBinary ?? savedBinary ?? DEFAULT_BINARY;
  const coreUrl = editedCoreUrl ?? savedCoreUrl ?? defaultCoreUrl;
  const [enableAsk, setEnableAsk] = useState(false);

  /** Asks Core for the graph and the model status; state changes only in callbacks. */
  const load = useCallback(() => {
    let cancelled = false;
    api.getGraph(graphId).then(
      (found) => {
        if (cancelled) return;
        setGraph(found);
        setCore({ state: "reachable" });
      },
      (error: unknown) => {
        if (cancelled) return;
        if (error instanceof ApiError && error.code === "GRAPH_NOT_FOUND") {
          setCore({ state: "graph_missing" });
        } else {
          setCore({
            state: "unreachable",
            message: error instanceof Error ? error.message : "MindGraph Core cannot be reached",
          });
        }
      },
    );
    api.graphRagStatus().then(
      (status) => !cancelled && setModel(status),
      () => !cancelled && setModel(null),
    );
    return () => {
      cancelled = true;
    };
  }, [api, graphId]);

  useEffect(() => load(), [load]);

  const retry = () => {
    setCore({ state: "checking" });
    load();
  };

  const client = MCP_CLIENTS.find((candidate) => candidate.id === clientId) ?? MCP_CLIENTS[0];
  const settings: McpSettings = { binary, coreUrl, graphId, enableAsk };
  const snippets = client.snippets(settings);

  return (
    <AppShell graphId={graphId} graphName={graph?.name}>
      <main className="mx-auto flex max-w-[1180px] flex-col gap-7 px-12 py-10">
        <header className="flex flex-wrap items-end justify-between gap-4">
          <div className="flex flex-col gap-2">
            <Kicker className="text-accent">Model Context Protocol</Kicker>
            <h1 className="text-[28px] leading-tight font-bold text-text">
              Connect MindGraph to your AI
            </h1>
            <p className="text-[14px] text-text-2">
              Give your AI agents read-only access to this graph through MCP.
            </p>
          </div>
          <CoreStatus core={core} onRetry={retry} />
        </header>

        {core.state === "graph_missing" && (
          <p role="alert" className="text-[13px] text-danger">
            MindGraph Core has no graph with this id. Pick a graph on the home page.
          </p>
        )}

        <div role="radiogroup" aria-label="MCP client" className="grid grid-cols-4 gap-3">
          {MCP_CLIENTS.map((candidate) => {
            const selected = candidate.id === clientId;
            return (
              <button
                key={candidate.id}
                type="button"
                role="radio"
                aria-checked={selected}
                onClick={() => setClientId(candidate.id)}
                className={cn(
                  "flex items-center gap-3 rounded-[6px] border px-4 py-3 text-left transition-colors",
                  selected
                    ? "border-accent bg-accent/10"
                    : "border-border bg-panel hover:border-border-strong",
                )}
              >
                <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[4px] border border-border-strong bg-raised font-mono text-[11px] text-text-2">
                  {candidate.initials}
                </span>
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="truncate text-[14px] font-semibold text-text">
                    {candidate.name}
                  </span>
                  <span className="font-mono text-[10.5px] text-text-3">{candidate.kind}</span>
                </span>
              </button>
            );
          })}
        </div>

        <div className="grid grid-cols-[minmax(0,1fr)_360px] gap-6">
          <div className="flex min-w-0 flex-col gap-5">
            <div className="flex items-center justify-between">
              <h2 className="text-[16px] font-semibold text-text">Configuration</h2>
              <Badge tone="accent">stdio</Badge>
            </div>

            <div className="grid grid-cols-2 gap-3 rounded-[6px] border border-border bg-panel p-4">
              <div className="flex flex-col gap-1">
                <span className="text-[12px] text-text-3">Graph</span>
                <span className="truncate text-[13px] text-text">{graph?.name ?? "…"}</span>
              </div>
              <div className="flex flex-col gap-1">
                <span className="text-[12px] text-text-3">Graph ID</span>
                <span className="flex items-center gap-2">
                  <code className="truncate font-mono text-[11.5px] text-text-2">{graphId}</code>
                  <CopyButton text={graphId} label="Copy graph ID" />
                </span>
              </div>
              <Field label="Path to mindgraph-mcp">
                <input
                  className={inputClass}
                  value={binary}
                  spellCheck={false}
                  onChange={(event) => {
                    setEditedBinary(event.target.value);
                    writeSetting(BINARY_KEY, event.target.value);
                  }}
                />
              </Field>
              <Field label="MindGraph Core URL (loopback only)">
                <input
                  className={inputClass}
                  value={coreUrl}
                  spellCheck={false}
                  onChange={(event) => {
                    setEditedCoreUrl(event.target.value);
                    writeSetting(CORE_URL_KEY, event.target.value);
                  }}
                />
              </Field>
              <label className="col-span-2 flex items-start gap-2 text-[12.5px] text-text-2">
                <input
                  type="checkbox"
                  className="mt-[3px] accent-[var(--color-accent)]"
                  checked={enableAsk}
                  onChange={(event) => setEnableAsk(event.target.checked)}
                />
                <span>
                  Enable <code className="font-mono text-[11.5px]">ask_graph</code> (GraphRAG). It
                  sends the question and retrieved graph data to the model provider configured in
                  MindGraph Core.
                </span>
              </label>
            </div>

            {snippets.map((snippet) => (
              <CodeBlock key={`${client.id}-${snippet.label}`} snippet={snippet} />
            ))}

            <ol aria-label="Connection steps" className="flex flex-col gap-3">
              <Step number={1} title="Start MindGraph">
                <InlineCommand command="docker compose up" />
              </Step>
              <Step number={2} title="Build mindgraph-mcp (no prebuilt binary is published yet)">
                <InlineCommand command="cargo build -p mindgraph-mcp --release" />
              </Step>
              <Step number={3} title={`Select your MCP client: ${client.name}`} />
              <Step number={4} title="Copy the configuration above" />
              <Step number={5} title="Restart or reload your MCP client">
                {client.reload}
              </Step>
              <Step number={6} title="Discover tools">
                {client.discover}
              </Step>
            </ol>

            <section
              aria-label="Example request"
              className="flex items-center gap-3 rounded-[6px] border border-border bg-panel px-4 py-3"
            >
              <Kicker>Try it</Kicker>
              <span className="flex-1 text-[13px] text-text">“{EXAMPLE_PROMPT}”</span>
              <CopyButton text={EXAMPLE_PROMPT} label="Copy example request" />
            </section>

            <p className="text-[12px] text-text-3">
              MindGraph cannot see which clients are connected: each client starts its own{" "}
              <code className="font-mono">mindgraph-mcp</code> process. Run{" "}
              <code className="font-mono">mindgraph-mcp --check</code> with the same environment to
              test a configuration.
            </p>
          </div>

          <aside
            aria-label="Available tools"
            className="flex flex-col self-start rounded-[6px] border border-border bg-panel"
          >
            <header className="flex items-center justify-between border-b border-border px-4 py-3">
              <h2 className="text-[15px] font-semibold text-text">Available Tools</h2>
              <Kicker>{enableAsk ? "8" : "7"} enabled</Kicker>
            </header>
            <ul className="flex flex-col">
              {MCP_TOOLS.map((tool) => {
                const enabled = !tool.optIn || enableAsk;
                return (
                  <li
                    key={tool.name}
                    className="flex flex-col gap-1 border-b border-border px-4 py-3 last:border-b-0"
                  >
                    <span className="flex items-center gap-2">
                      <code
                        className={cn(
                          "flex-1 font-mono text-[12.5px]",
                          enabled ? "text-text" : "text-text-3",
                        )}
                      >
                        {tool.name}
                      </code>
                      {tool.optIn ? (
                        <Badge tone={enabled ? "warn" : "neutral"}>
                          {enabled ? "opt-in · on" : "opt-in · off"}
                        </Badge>
                      ) : (
                        <Badge tone="ok">read-only</Badge>
                      )}
                    </span>
                    <span className="text-[12px] text-text-3">{tool.description}</span>
                    {tool.optIn && (
                      <span className="flex items-center gap-1 text-[11.5px] text-text-3">
                        <Sparkles size={11} />
                        {model?.status === "ready"
                          ? `Core model: ${model.model ?? model.provider ?? "configured"}`
                          : "No model configured in Core: ask_graph would answer AI_NOT_CONFIGURED"}
                      </span>
                    )}
                  </li>
                );
              })}
            </ul>
            <footer className="flex gap-2 border-t border-border px-4 py-3 text-[11.5px] text-text-3">
              <Lock size={12} className="mt-[2px] shrink-0" />
              <span>
                No tool can change the graph, and every tool reads only this graph. mindgraph-mcp
                connects only to a Core on 127.0.0.1, ::1 or localhost.
              </span>
            </footer>
          </aside>
        </div>
      </main>
    </AppShell>
  );
}
