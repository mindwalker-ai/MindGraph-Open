/**
 * Shared helpers for the browser end-to-end tests. Test data is set up
 * through the same public REST API (via the web app's proxy) that users and
 * the demo loader use; nothing is mocked.
 */

import { readFileSync } from "node:fs";
import path from "node:path";

import { type APIRequestContext, type APIResponse, expect } from "@playwright/test";

/** The repository's `examples/` folder. */
export const EXAMPLES = path.resolve(__dirname, "../../../examples");

export const FRAUD_DEMO = "Demo: Fraud investigation";

export interface Graph {
  id: string;
  name: string;
}

export interface Entity {
  id: string;
  entity_type: string;
  properties: Record<string, unknown>;
}

/** A graph name unique to this run, recognisable as test data. */
export function uniqueName(label: string): string {
  return `E2E ${label} ${new Date().toISOString().replace(/[-:]/g, "").slice(0, 15)}`;
}

async function json<T>(response: APIResponse): Promise<T> {
  expect(response.ok(), `${response.url()} → ${response.status()} ${await response.text()}`).toBe(
    true,
  );
  return (await response.json()) as T;
}

/** The fraud demo graph; fails with the command to run when it is missing. */
export async function fraudDemo(request: APIRequestContext): Promise<Graph> {
  const { data } = await json<{ data: Graph[] }>(await request.get("/api/v1/graphs?limit=100"));
  const graph = data.find((g) => g.name === FRAUD_DEMO);
  if (!graph) throw new Error(`"${FRAUD_DEMO}" is not loaded: run \`make demo-fraud\` first`);
  return graph;
}

/** One entity by an exact property value. */
export async function findEntity(
  request: APIRequestContext,
  graphId: string,
  entityType: string,
  property: string,
  value: string,
): Promise<Entity> {
  const found = await json<{ data: Entity[] }>(
    await request.post(`/api/v1/graphs/${graphId}/entities/search`, {
      data: { entity_type: entityType, property_filters: { [property]: value }, limit: 2 },
    }),
  );
  expect(found.data, `${entityType} ${property}=${value}`).toHaveLength(1);
  return found.data[0];
}

async function waitForJob(request: APIRequestContext, jobId: string): Promise<void> {
  await expect
    .poll(
      async () => {
        const job = await json<{ status: string }>(await request.get(`/api/v1/jobs/${jobId}`));
        return job.status;
      },
      { timeout: 60_000, intervals: [250, 500, 1000] },
    )
    .toBe("completed");
}

/**
 * Creates a graph with the fraud ontology and ingests `files` (CSV names in
 * examples/fraud, each with its `.mapping.json`), in order.
 */
export async function fraudGraph(
  request: APIRequestContext,
  name: string,
  files: string[],
): Promise<Graph> {
  const dir = path.join(EXAMPLES, "fraud");
  const graph = await json<Graph>(await request.post("/api/v1/graphs", { data: { name } }));
  await json(
    await request.post(`/api/v1/graphs/${graph.id}/ontology`, {
      data: JSON.parse(readFileSync(path.join(dir, "ontology.json"), "utf8")),
    }),
  );
  for (const file of files) {
    const source = await json<{ id: string }>(
      await request.post(`/api/v1/graphs/${graph.id}/data-sources`, {
        multipart: {
          file: { name: file, mimeType: "text/csv", buffer: readFileSync(path.join(dir, file)) },
        },
      }),
    );
    const base = `/api/v1/graphs/${graph.id}/data-sources/${source.id}`;
    const analysis = await json<{ job_id: string }>(await request.post(`${base}/analyze`));
    await waitForJob(request, analysis.job_id);
    const mapping = readFileSync(path.join(dir, file.replace(/\.csv$/, ".mapping.json")), "utf8");
    const ingestion = await json<{ job_id: string }>(
      await request.post(`${base}/ingestions`, {
        headers: { "Content-Type": "application/json", "Idempotency-Key": crypto.randomUUID() },
        data: mapping,
      }),
    );
    await waitForJob(request, ingestion.job_id);
  }
  return graph;
}
