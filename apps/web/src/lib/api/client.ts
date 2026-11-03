/**
 * A small typed client for the MindGraph API. Components never call
 * `fetch` themselves; they receive an {@link ApiClient} (injectable in tests).
 *
 * Requests go to the same origin (`/api/v1/...`). The Next.js route handler in
 * `app/api/v1/[...path]/route.ts` forwards them to MindGraph Core, which is
 * only reachable inside the Docker network.
 */

import type {
  DataSource,
  GraphBuilderStatus,
  GraphProject,
  IngestionMapping,
  Job,
  JobAccepted,
  Ontology,
  OntologyDraft,
} from "./types";

/** A failed API call, carrying the stable error envelope fields. */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
    readonly requestId?: string,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

export interface ApiClient {
  listGraphs(): Promise<GraphProject[]>;
  createGraph(name: string, description?: string): Promise<GraphProject>;
  getGraph(graphId: string): Promise<GraphProject>;
  getOntology(graphId: string): Promise<Ontology>;
  /** Creates the next version. With `expectedVersion`, the server rejects
   * the write (409 CONFLICT) if another version became active meanwhile. */
  createOntology(
    graphId: string,
    draft: OntologyDraft,
    expectedVersion?: number,
  ): Promise<Ontology>;
  uploadDataSource(graphId: string, file: File): Promise<DataSource>;
  getDataSource(graphId: string, dataSourceId: string): Promise<DataSource>;
  analyze(graphId: string, dataSourceId: string): Promise<JobAccepted>;
  graphBuilderStatus(): Promise<GraphBuilderStatus>;
  requestSuggestion(
    graphId: string,
    dataSourceId: string,
    idempotencyKey: string,
  ): Promise<JobAccepted>;
  startIngestion(
    graphId: string,
    dataSourceId: string,
    mapping: IngestionMapping,
    idempotencyKey: string,
  ): Promise<JobAccepted>;
  getJob(jobId: string): Promise<Job>;
}

type Fetch = typeof fetch;

function parse(text: string): unknown {
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    return undefined;
  }
}

async function decode<T>(response: Response): Promise<T> {
  const text = await response.text();
  // A proxy or an HTML error page may answer with something that is not JSON.
  const body = parse(text);
  if (!response.ok) {
    const error = (body as { error?: { code?: string; message?: string; request_id?: string } })
      ?.error;
    throw new ApiError(
      response.status,
      error?.code ?? "HTTP_ERROR",
      error?.message ?? `Request failed with HTTP ${response.status}`,
      error?.request_id,
    );
  }
  if (body === undefined) {
    throw new ApiError(
      response.status,
      "INVALID_RESPONSE",
      "The server sent an unreadable response",
    );
  }
  return body as T;
}

/** The client used by the app, talking to `baseUrl` (same origin by default). */
export function createApiClient(
  baseUrl = "",
  fetcher: Fetch = (...args) => fetch(...args),
): ApiClient {
  const url = (path: string) => `${baseUrl}/api/v1${path}`;
  const json = (method: string, body: unknown, headers: Record<string, string> = {}) => ({
    method,
    headers: { "Content-Type": "application/json", ...headers },
    body: JSON.stringify(body),
  });
  const call = async <T>(path: string, init?: RequestInit) =>
    decode<T>(await fetcher(url(path), init));

  return {
    listGraphs: async () => (await call<{ data: GraphProject[] }>("/graphs")).data,
    createGraph: (name, description) =>
      call("/graphs", json("POST", { name, description: description || undefined })),
    getGraph: (graphId) => call(`/graphs/${graphId}`),
    getOntology: (graphId) => call(`/graphs/${graphId}/ontology`),
    createOntology: (graphId, draft, expectedVersion) =>
      call(
        `/graphs/${graphId}/ontology`,
        json(
          "POST",
          expectedVersion === undefined ? draft : { ...draft, expected_version: expectedVersion },
        ),
      ),
    uploadDataSource: (graphId, file) => {
      const form = new FormData();
      form.append("file", file, file.name);
      return call(`/graphs/${graphId}/data-sources`, { method: "POST", body: form });
    },
    getDataSource: (graphId, dataSourceId) =>
      call(`/graphs/${graphId}/data-sources/${dataSourceId}`),
    analyze: (graphId, dataSourceId) =>
      call(`/graphs/${graphId}/data-sources/${dataSourceId}/analyze`, { method: "POST" }),
    graphBuilderStatus: () => call("/graph-builder/status"),
    requestSuggestion: (graphId, dataSourceId, idempotencyKey) =>
      call(`/graphs/${graphId}/data-sources/${dataSourceId}/graph-builder/suggestions`, {
        method: "POST",
        headers: { "Idempotency-Key": idempotencyKey },
      }),
    startIngestion: (graphId, dataSourceId, mapping, idempotencyKey) =>
      call(
        `/graphs/${graphId}/data-sources/${dataSourceId}/ingestions`,
        json("POST", { mapping }, { "Idempotency-Key": idempotencyKey }),
      ),
    getJob: (jobId) => call(`/jobs/${jobId}`),
  };
}

/** A fresh idempotency key for one user action. */
export function newIdempotencyKey(): string {
  return crypto.randomUUID();
}
