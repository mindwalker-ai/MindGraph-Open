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

export interface GraphBuilderSuggestion {
  ontology: OntologyDraft;
  mapping: IngestionMapping;
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
