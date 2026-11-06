/**
 * A small typed client for the MindGraph API. Components never call
 * `fetch` themselves; they receive an {@link ApiClient} (injectable in tests).
 *
 * Requests go to the same origin (`/api/v1/...`). The Next.js route handler in
 * `app/api/v1/[...path]/route.ts` forwards them to MindGraph Core, which is
 * only reachable inside the Docker network.
 */

import type {
  AskRequest,
  AskResponse,
  CandidateDetail,
  CandidatePage,
  CandidateQuery,
  DataSource,
  DecisionRequest,
  DecisionResponse,
  Entity,
  EntitySearchQuery,
  EntityIdentity,
  EntitySearchResult,
  GraphBuilderStatus,
  GraphPath,
  GraphProject,
  IngestionMapping,
  Job,
  JobAccepted,
  NeighborOptions,
  Ontology,
  OntologyDraft,
  PathOptions,
  ResolutionScan,
  ScanRequest,
  Subgraph,
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
  /** One AI suggestion that models the analysed data sources together. */
  requestSuggestion(
    graphId: string,
    dataSourceIds: string[],
    idempotencyKey: string,
  ): Promise<JobAccepted>;
  startIngestion(
    graphId: string,
    dataSourceId: string,
    mapping: IngestionMapping,
    idempotencyKey: string,
  ): Promise<JobAccepted>;
  getJob(jobId: string): Promise<Job>;
  /** Exact search by entity type and property values (no full-text search). */
  searchEntities(graphId: string, query: EntitySearchQuery): Promise<EntitySearchResult>;
  getEntity(graphId: string, entityId: string): Promise<Entity>;
  /** The entity, its matching relationships (bounded) and their other ends. */
  getNeighbors(graphId: string, entityId: string, options?: NeighborOptions): Promise<Subgraph>;
  /** A shortest path computed by MindGraph Core, or `found: false`. */
  findPath(
    graphId: string,
    sourceId: string,
    targetId: string,
    options?: PathOptions,
  ): Promise<GraphPath>;
  /** The given entities plus every relationship among them. */
  getSubgraph(graphId: string, entityIds: string[]): Promise<Subgraph>;
  /** Ask MindGraph: a question answered from bounded graph evidence (M7). */
  ask(graphId: string, request: AskRequest): Promise<AskResponse>;
  /** Whether a model is configured for Ask MindGraph. */
  graphRagStatus(): Promise<GraphBuilderStatus>;
  /** Runs a bounded duplicate scan over one entity type (M9). */
  runResolutionScan(graphId: string, request: ScanRequest): Promise<ResolutionScan>;
  /** The graph's most recent scans, newest first. */
  listResolutionScans(graphId: string, limit?: number): Promise<ResolutionScan[]>;
  /** One page of the review queue, highest score first. */
  listResolutionCandidates(graphId: string, query?: CandidateQuery): Promise<CandidatePage>;
  /** A candidate with both original entities and its decision history. */
  getResolutionCandidate(graphId: string, candidateId: string): Promise<CandidateDetail>;
  /** Records a review decision (a logical identity mapping, never a graph merge). */
  decideResolutionCandidate(
    graphId: string,
    candidateId: string,
    request: DecisionRequest,
  ): Promise<DecisionResponse>;
  /** The logical identity group of an entity. */
  getEntityIdentity(graphId: string, entityId: string): Promise<EntityIdentity>;
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
    requestSuggestion: (graphId, dataSourceIds, idempotencyKey) =>
      call(
        `/graphs/${graphId}/graph-builder/suggestions`,
        json("POST", { data_source_ids: dataSourceIds }, { "Idempotency-Key": idempotencyKey }),
      ),
    startIngestion: (graphId, dataSourceId, mapping, idempotencyKey) =>
      call(
        `/graphs/${graphId}/data-sources/${dataSourceId}/ingestions`,
        json("POST", { mapping }, { "Idempotency-Key": idempotencyKey }),
      ),
    getJob: (jobId) => call(`/jobs/${jobId}`),
    searchEntities: (graphId, query) =>
      call(`/graphs/${graphId}/entities/search`, json("POST", query)),
    getEntity: (graphId, entityId) =>
      call(`/graphs/${graphId}/entities/${encodeURIComponent(entityId)}`),
    getNeighbors: (graphId, entityId, options = {}) => {
      const params = new URLSearchParams();
      if (options.direction) params.set("direction", options.direction);
      if (options.relationship_type) params.set("relationship_type", options.relationship_type);
      if (options.limit !== undefined) params.set("limit", String(options.limit));
      const query = params.size ? `?${params}` : "";
      return call(`/graphs/${graphId}/entities/${encodeURIComponent(entityId)}/neighbors${query}`);
    },
    findPath: (graphId, sourceId, targetId, options = {}) =>
      call(
        `/graphs/${graphId}/paths`,
        json("POST", { source_id: sourceId, target_id: targetId, ...options }),
      ),
    getSubgraph: (graphId, entityIds) =>
      call(`/graphs/${graphId}/subgraph`, json("POST", { entity_ids: entityIds })),
    ask: (graphId, request) => call(`/graphs/${graphId}/ask`, json("POST", request)),
    graphRagStatus: () => call("/graphrag/status"),
    runResolutionScan: (graphId, request) =>
      call(`/graphs/${graphId}/resolution/scans`, json("POST", request)),
    listResolutionScans: async (graphId, limit) =>
      (
        await call<{ data: ResolutionScan[] }>(
          `/graphs/${graphId}/resolution/scans${limit === undefined ? "" : `?limit=${limit}`}`,
        )
      ).data,
    listResolutionCandidates: (graphId, query = {}) => {
      const params = new URLSearchParams();
      for (const [key, value] of Object.entries(query)) {
        if (value !== undefined && value !== "") params.set(key, String(value));
      }
      const suffix = params.size ? `?${params}` : "";
      return call(`/graphs/${graphId}/resolution/candidates${suffix}`);
    },
    getResolutionCandidate: (graphId, candidateId) =>
      call(`/graphs/${graphId}/resolution/candidates/${encodeURIComponent(candidateId)}`),
    decideResolutionCandidate: (graphId, candidateId, request) =>
      call(
        `/graphs/${graphId}/resolution/candidates/${encodeURIComponent(candidateId)}/decisions`,
        json("POST", request),
      ),
    getEntityIdentity: (graphId, entityId) =>
      call(`/graphs/${graphId}/resolution/entities/${encodeURIComponent(entityId)}`),
  };
}

/** A fresh idempotency key for one user action. */
export function newIdempotencyKey(): string {
  return crypto.randomUUID();
}
