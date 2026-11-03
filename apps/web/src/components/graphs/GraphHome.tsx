"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { type FormEvent, useEffect, useMemo, useState } from "react";

import { Badge, Button, ErrorNote, Panel, inputClass } from "@/components/ui/primitives";
import { type ApiClient, ApiError, createApiClient } from "@/lib/api/client";
import type { GraphProject } from "@/lib/api/types";

/** Lists graph projects and creates new ones (then opens the Graph Builder). */
export function GraphHome({ api: injected }: { api?: ApiClient }) {
  const api = useMemo(() => injected ?? createApiClient(), [injected]);
  const router = useRouter();
  const [graphs, setGraphs] = useState<GraphProject[] | null>(null);
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);

  useEffect(() => {
    api
      .listGraphs()
      .then(setGraphs, (err: unknown) =>
        setError(err instanceof ApiError ? err.message : "MindGraph Core cannot be reached."),
      );
  }, [api]);

  async function create(event: FormEvent) {
    event.preventDefault();
    if (!name.trim()) return;
    setCreating(true);
    setError(null);
    try {
      const graph = await api.createGraph(name.trim());
      router.push(`/graphs/${graph.id}/builder`);
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
      setCreating(false);
    }
  }

  return (
    <div className="flex flex-col gap-5">
      <Panel title="New graph">
        <form onSubmit={create} className="flex flex-wrap gap-2">
          <input
            aria-label="Graph name"
            placeholder="Fraud Investigation"
            className={`${inputClass} max-w-sm`}
            value={name}
            onChange={(event) => setName(event.target.value)}
          />
          <Button type="submit" variant="primary" disabled={!name.trim() || creating}>
            {creating ? "Creating…" : "Create graph"}
          </Button>
        </form>
      </Panel>
      {error && <ErrorNote>{error}</ErrorNote>}
      <Panel title="Graphs">
        {graphs === null && !error && <p className="text-sm text-muted">Loading…</p>}
        {graphs?.length === 0 && <p className="text-sm text-muted">No graphs yet.</p>}
        <ul className="flex flex-col divide-y divide-border">
          {graphs?.map((graph) => (
            <li key={graph.id} className="flex items-center justify-between gap-3 py-2">
              <div className="min-w-0">
                <p className="truncate font-medium">{graph.name}</p>
                <p className="font-mono text-xs text-muted">{graph.id}</p>
              </div>
              <div className="flex shrink-0 items-center gap-2">
                <Badge tone={graph.status === "active" ? "success" : "neutral"}>
                  {graph.status}
                </Badge>
                <Link href={`/graphs/${graph.id}/builder`} className="text-sm text-accent">
                  Graph Builder →
                </Link>
              </div>
            </li>
          ))}
        </ul>
      </Panel>
    </div>
  );
}
