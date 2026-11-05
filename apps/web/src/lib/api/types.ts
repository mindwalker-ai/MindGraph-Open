/**
 * Typed shapes of the public MindGraph API (`/api/v1`), as documented in
 * `docs/api/openapi.yaml`. Transport only: no UI state lives here.
 */

export type PropertyType = "string" | "integer" | "float" | "boolean" | "datetime" | "string_array";

export const PROPERTY_TYPES: readonly PropertyType[] = [
  "string",
  "integer",
  "float",
  "boolean",
  "datetime",
  "string_array",
];

export interface GraphProject {
  id: string;
  workspace_id: string;
  name: string;
  description: string | null;
  status: "draft" | "active" | "archived";
  created_at: string;
  updated_at: string;
}

export interface FieldProfile {
  name: string;
  inferred_type: PropertyType;
  nullable: boolean;
  non_null_count: number;
  sample_values: string[];
}

export interface DatasetProfile {
  record_count: number;
  fields: FieldProfile[];
  sample_rows: Record<string, unknown>[];
}

export interface DataSource {
  id: string;
  graph_id: string;
  kind: "csv" | "json";
  filename: string;
  status: "pending" | "ready" | "failed";
  size_bytes: number;
  content_sha256: string;
  profile: DatasetProfile | null;
  created_at: string;
  updated_at: string;
}

/** A property as sent to `POST /graphs/{id}/ontology`. */
export interface PropertyDraft {
  name: string;
  type: PropertyType;
  required: boolean;
}

export interface EntityTypeDraft {
  name: string;
  display_name?: string | null;
  description?: string | null;
  properties: PropertyDraft[];
}

export interface RelationshipTypeDraft {
  name: string;
  source_entity_type: string;
  target_entity_type: string;
  properties: PropertyDraft[];
}

/** The complete next ontology version (`POST /graphs/{id}/ontology`). */
export interface OntologyDraft {
  entity_types: EntityTypeDraft[];
  relationship_types: RelationshipTypeDraft[];
}

/** A stored ontology version (`GET /graphs/{id}/ontology`). */
export interface Ontology {
  graph_id: string;
  version: number;
  entity_types: {
    id: string;
    name: string;
    display_name: string;
    description: string | null;
    properties: { key: string; type: PropertyType; required: boolean }[];
  }[];
  relationship_types: {
    id: string;
    name: string;
    source_entity_type: string;
    target_entity_type: string;
    properties: { key: string; type: PropertyType; required: boolean }[];
  }[];
}

export type EntityMode = "create_or_match" | "match_only";

export interface FieldMapping {
  source_field: string;
  property: string;
}

export interface EntityMapping {
  alias: string;
  entity_type: string;
  mode: EntityMode;
  identity: FieldMapping[];
  properties: FieldMapping[];
}

export interface RelationshipMapping {
  relationship_type: string;
  source: string;
  target: string;
  properties: FieldMapping[];
}

/** The M4 ingestion mapping contract. */
export interface IngestionMapping {
  entities: EntityMapping[];
  relationships: RelationshipMapping[];
}

export interface TypeAnnotation {
  name: string;
  confidence: number;
  reason: string;
  existing: boolean;
}

/** The mapping of one data source in a suggestion. */
export interface DatasetMapping {
  /** The dataset's label in the AI request (its filename, made unique). */
  dataset: string;
  data_source_id: string;
  mapping: IngestionMapping;
}

export interface GraphBuilderSuggestion {
  ontology: OntologyDraft;
  /** One mapping per data source, in a safe ingestion order. */
  mappings: DatasetMapping[];
  confidence: number;
  annotations: { entities: TypeAnnotation[]; relationships: TypeAnnotation[] };
  warnings: string[];
}

export interface GraphBuilderResult {
  prompt_version: string;
  provider: string;
  model: string;
  ontology_version_base: number;
  repair_attempts: number;
  suggestion: GraphBuilderSuggestion;
}

export interface IngestionSummary {
  records_processed: number;
  entities_created: number;
  entities_matched: number;
  relationships_created: number;
}

export type JobStatus = "queued" | "running" | "completed" | "failed" | "cancelled";

export interface Job {
  id: string;
  graph_id: string;
  data_source_id: string;
  /** Every data source the job reads, in order. */
  data_source_ids: string[];
  kind: "analyze" | "ingest" | "ai_suggest";
  status: JobStatus;
  progress: number;
  ontology_version: number | null;
  error: { code: string; message: string } | null;
  result: IngestionSummary | GraphBuilderResult | null;
  created_at: string;
  updated_at: string;
}

export interface JobAccepted {
  job_id: string;
  status: JobStatus;
}

export interface GraphBuilderStatus {
  status: "ready" | "not_configured" | "unavailable";
  configured: boolean;
  provider?: string;
  model?: string;
}

export function isTerminal(status: JobStatus): boolean {
  return status === "completed" || status === "failed" || status === "cancelled";
}

// --- Graph Explorer (M6) ------------------------------------------------------

/**
 * A property value as the explorer endpoints render it: plain JSON, with
 * datetimes as RFC 3339 strings and integers beyond 2^53 - 1 as decimal
 * strings (`property_types` tells them apart).
 */
export type PropertyJson = string | number | boolean | string[];

/** An entity of one graph (`GET /graphs/{id}/entities/{entity_id}`). */
export interface Entity {
  id: string;
  graph_id: string;
  entity_type: string;
  properties: Record<string, PropertyJson>;
  property_types: Record<string, PropertyType>;
}

/** A directed relationship between two entities of the same graph. */
export interface Relationship {
  id: string;
  graph_id: string;
  relationship_type: string;
  source_id: string;
  target_id: string;
  properties: Record<string, PropertyJson>;
  property_types: Record<string, PropertyType>;
}

/** Entities and the relationships among them, ordered by id. */
export interface Subgraph {
  entities: Entity[];
  relationships: Relationship[];
}

/** A shortest path from source to target, or `found: false` within the depth. */
export interface GraphPath {
  found: boolean;
  hops: number;
  /** In order, from the source to the target. */
  entities: Entity[];
  /** `relationships[i]` connects `entities[i]` and `entities[i + 1]`. */
  relationships: Relationship[];
}

export type Direction = "outgoing" | "incoming" | "both";

export const DIRECTIONS: readonly Direction[] = ["both", "outgoing", "incoming"];

/** Exact matching (`POST /graphs/{id}/entities/search`). */
export interface EntitySearchQuery {
  entity_type?: string;
  /** Property key to exact value; values are typed by the active ontology. */
  property_filters?: Record<string, PropertyJson>;
  limit?: number;
}

export interface EntitySearchResult {
  data: Entity[];
  count: number;
  limit: number;
}

export interface NeighborOptions {
  direction?: Direction;
  relationship_type?: string;
  limit?: number;
}

export interface PathOptions {
  direction?: Direction;
  max_depth?: number;
}

/** Bounds of the explorer endpoints (`mindgraph-graph`). */
export const EXPLORER_LIMITS = {
  searchDefault: 100,
  searchMax: 1000,
  neighborDefault: 100,
  neighborMax: 1000,
  pathDefaultDepth: 3,
  pathMaxDepth: 6,
  subgraphMax: 1000,
} as const;

// --- Ask MindGraph (M7) -------------------------------------------------------

/** What a question is about: the selected entities, or bounded discovery. */
export type AskScope =
  { mode: "selected_entities"; entity_ids: string[] } | { mode: "bounded_graph" };

/**
 * An earlier question of the conversation, sent with a follow-up. The
 * browser keeps the conversation; MindGraph Core checks the ids again.
 */
export interface AskHistoryTurn {
  question: string;
  /** Entities that question was about or cited, most relevant first. */
  entity_ids: string[];
}

export interface AskRequest {
  question: string;
  scope: AskScope;
  /** Earlier questions, oldest first (at most `ASK_LIMITS.historyTurns`). */
  history?: AskHistoryTurn[];
}

/** How well the retrieved evidence supports the answer. */
export type AnswerStatus = "answered" | "partial" | "insufficient_evidence";

/** Who wrote the answer: the model, or MindGraph when there was nothing to phrase. */
export type AnswerSource = "model" | "no_evidence" | "unsupported";

export type EvidenceKind = "entity" | "relationship" | "path";

export interface EvidenceEntity extends Entity {
  /** `E1`, `E2`, … */
  ref: string;
}

export interface EvidenceRelationship extends Relationship {
  /** `R1`, `R2`, … */
  ref: string;
}

export interface EvidencePath {
  /** `P1`, `P2`, … */
  ref: string;
  /** In order; `relationship_ids[i]` connects `entity_ids[i]` and `entity_ids[i + 1]`. */
  entity_ids: string[];
  relationship_ids: string[];
}

/** A citation MindGraph Core checked against the evidence. */
export interface Citation {
  ref: string;
  kind: EvidenceKind;
  /** The cited entity or relationship id; absent for paths. */
  id?: string;
}

/** The validated retrieval plan (descriptive, never executable). */
export interface PlanSummary {
  operation: "entity_lookup" | "neighbors" | "find_path" | "shared_neighbor" | "unsupported";
  source?: string;
  target?: string;
  entity_type?: string;
  shared_entity_type?: string;
  relationship_type?: string;
  direction?: Direction;
  max_depth?: number;
  reason?: string;
  notes?: string[];
}

export interface RetrievalReport {
  scope: AskScope["mode"];
  plan: PlanSummary;
  operations: number;
  candidates_inspected: number;
  /** A limit stopped retrieval: the answer may miss results. */
  truncated: boolean;
  context_bytes: number;
  limitations: string[];
}

export interface AnswerMetadata {
  interpret_prompt_version: string;
  synthesize_prompt_version: string | null;
  provider: string;
  model: string;
  /** Earlier questions used as conversation context. */
  conversation_turns: number;
  duration_ms: number;
}

/** `POST /graphs/{id}/ask`: the checked answer and the evidence it rests on. */
export interface AskResponse {
  status: AnswerStatus;
  answer: string;
  answer_source: AnswerSource;
  evidence: {
    entities: EvidenceEntity[];
    relationships: EvidenceRelationship[];
    paths: EvidencePath[];
  };
  citations: Citation[];
  warnings: string[];
  retrieval: RetrievalReport;
  metadata: AnswerMetadata;
}

/** Bounds of `POST /graphs/{id}/ask` (`mindgraph-graphrag`). */
export const ASK_LIMITS = {
  questionMax: 2000,
  seedMax: 5,
  /** Earlier questions a follow-up carries. */
  historyTurns: 3,
  /** Entity ids one earlier question carries. */
  historyTurnEntities: 10,
} as const;
