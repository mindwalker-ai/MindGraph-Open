/**
 * Ask MindGraph answers (M7) in the browser: shape checks, reference
 * lookups and contextual question suggestions. Everything here is pure.
 *
 * MindGraph Core retrieves the evidence and checks every citation; the
 * browser still verifies the response shape before anything reaches the
 * graph (every relationship's endpoints and every path item must be part of
 * the evidence), so a malformed answer can never add dangling edges.
 */

import {
  ASK_LIMITS,
  type AnswerSource,
  type AnswerStatus,
  type AskHistoryTurn,
  type AskResponse,
  type Citation,
  type EvidenceEntity,
  type EvidenceKind,
  type EvidencePath,
  type EvidenceRelationship,
  type Ontology,
  type Subgraph,
} from "@/lib/api/types";
import { MalformedGraphError, parseEntity, parseRelationship } from "@/lib/graph/transform";

const STATUSES = new Set<AnswerStatus>(["answered", "partial", "insufficient_evidence"]);
const SOURCES = new Set<AnswerSource>(["model", "no_evidence", "unsupported"]);
const KINDS: Record<string, EvidenceKind> = { E: "entity", R: "relationship", P: "path" };
const REF = /^[ERP][1-9]\d{0,5}$/;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function list(value: unknown, what: string): unknown[] {
  if (!Array.isArray(value)) throw new MalformedGraphError(`${what} must be an array`);
  return value;
}

function strings(value: unknown, what: string): string[] {
  return list(value, what).map((item) => {
    if (typeof item !== "string") throw new MalformedGraphError(`${what} must hold strings`);
    return item;
  });
}

function reference(value: unknown, prefix: "E" | "R" | "P", seen: Set<string>): string {
  if (typeof value !== "string" || !REF.test(value) || value[0] !== prefix) {
    throw new MalformedGraphError(`an evidence item has an invalid reference`);
  }
  if (seen.has(value)) throw new MalformedGraphError(`evidence reference ${value} repeats`);
  seen.add(value);
  return value;
}

/**
 * Checks an Ask MindGraph response for `graphId`.
 *
 * @throws MalformedGraphError
 */
export function parseAskResponse(payload: unknown, graphId: string): AskResponse {
  if (!isRecord(payload)) throw new MalformedGraphError("an answer must be an object");
  const { status, answer, answer_source: source, evidence, retrieval, metadata } = payload;
  if (!STATUSES.has(status as AnswerStatus)) throw new MalformedGraphError("unknown status");
  if (typeof answer !== "string") throw new MalformedGraphError("answer must be a string");
  if (!SOURCES.has(source as AnswerSource)) throw new MalformedGraphError("unknown answer_source");
  if (!isRecord(evidence)) throw new MalformedGraphError("evidence must be an object");
  if (!isRecord(retrieval) || !isRecord(retrieval.plan)) {
    throw new MalformedGraphError("retrieval must describe its plan");
  }
  if (!isRecord(metadata)) throw new MalformedGraphError("metadata must be an object");

  const refs = new Set<string>();
  const entities: EvidenceEntity[] = list(evidence.entities, "evidence entities").map((item) => ({
    ...parseEntity(item, graphId),
    ref: reference(isRecord(item) ? item.ref : undefined, "E", refs),
  }));
  const entityIds = new Set(entities.map((entity) => entity.id));
  const relationships: EvidenceRelationship[] = list(
    evidence.relationships,
    "evidence relationships",
  ).map((item) => {
    const relationship = parseRelationship(item, graphId);
    if (!entityIds.has(relationship.source_id) || !entityIds.has(relationship.target_id)) {
      throw new MalformedGraphError(
        `relationship ${relationship.id} has an endpoint outside the evidence`,
      );
    }
    return { ...relationship, ref: reference(isRecord(item) ? item.ref : undefined, "R", refs) };
  });
  const relationshipIds = new Set(relationships.map((relationship) => relationship.id));
  const paths: EvidencePath[] = list(evidence.paths, "evidence paths").map((item) => {
    if (!isRecord(item)) throw new MalformedGraphError("a path must be an object");
    const path = {
      ref: reference(item.ref, "P", refs),
      entity_ids: strings(item.entity_ids, "path entity_ids"),
      relationship_ids: strings(item.relationship_ids, "path relationship_ids"),
    };
    if (
      path.entity_ids.length !== path.relationship_ids.length + 1 ||
      !path.entity_ids.every((id) => entityIds.has(id)) ||
      !path.relationship_ids.every((id) => relationshipIds.has(id))
    ) {
      throw new MalformedGraphError(`path ${path.ref} refers to items outside the evidence`);
    }
    return path;
  });
  const citations: Citation[] = list(payload.citations, "citations").map((item) => {
    if (!isRecord(item) || typeof item.ref !== "string" || !refs.has(item.ref)) {
      throw new MalformedGraphError("a citation refers to missing evidence");
    }
    const kind = KINDS[item.ref[0]];
    if (item.kind !== kind)
      throw new MalformedGraphError(`citation ${item.ref} has the wrong kind`);
    return typeof item.id === "string"
      ? { ref: item.ref, kind, id: item.id }
      : { ref: item.ref, kind };
  });
  const number = (value: unknown) => (typeof value === "number" ? value : 0);
  return {
    status: status as AnswerStatus,
    answer,
    answer_source: source as AnswerSource,
    evidence: { entities, relationships, paths },
    citations,
    warnings: strings(payload.warnings ?? [], "warnings"),
    retrieval: {
      scope: retrieval.scope === "selected_entities" ? "selected_entities" : "bounded_graph",
      plan: retrieval.plan as unknown as AskResponse["retrieval"]["plan"],
      operations: number(retrieval.operations),
      candidates_inspected: number(retrieval.candidates_inspected),
      truncated: retrieval.truncated === true,
      context_bytes: number(retrieval.context_bytes),
      limitations: strings(retrieval.limitations ?? [], "limitations"),
    },
    metadata: {
      interpret_prompt_version: String(metadata.interpret_prompt_version ?? ""),
      synthesize_prompt_version:
        typeof metadata.synthesize_prompt_version === "string"
          ? metadata.synthesize_prompt_version
          : null,
      provider: String(metadata.provider ?? ""),
      model: String(metadata.model ?? ""),
      conversation_turns: number(metadata.conversation_turns),
      duration_ms: number(metadata.duration_ms),
    },
  };
}

/**
 * What a follow-up remembers of an answered question: its text and the
 * entities it was about, the question's scope first and then the cited
 * entities (and the endpoints of cited relationships and paths) in citation
 * order, at most `ASK_LIMITS.historyTurnEntities`.
 */
export function historyTurn(
  question: string,
  scopeIds: readonly string[],
  answer: AskResponse,
): AskHistoryTurn {
  const ids: string[] = [];
  const add = (id: string) => {
    if (!ids.includes(id)) ids.push(id);
  };
  scopeIds.forEach(add);
  const relationships = new Map(answer.evidence.relationships.map((r) => [r.ref, r]));
  const paths = new Map(answer.evidence.paths.map((p) => [p.ref, p]));
  for (const citation of answer.citations) {
    if (citation.kind === "entity" && citation.id) add(citation.id);
    const relationship = relationships.get(citation.ref);
    if (relationship) [relationship.source_id, relationship.target_id].forEach(add);
    paths.get(citation.ref)?.entity_ids.forEach(add);
  }
  return { question, entity_ids: ids.slice(0, ASK_LIMITS.historyTurnEntities) };
}

/** What a reference highlights on the graph. */
export interface EvidenceTargets {
  entityIds: string[];
  relationshipIds: string[];
}

/** Every entity and relationship of the evidence. */
export function evidenceTargets(answer: AskResponse): EvidenceTargets {
  return {
    entityIds: answer.evidence.entities.map((entity) => entity.id),
    relationshipIds: answer.evidence.relationships.map((relationship) => relationship.id),
  };
}

/**
 * The graph objects of one reference: an entity, a relationship with its
 * endpoints, or a whole path. `null` for a reference that is not in the
 * evidence.
 */
export function refTargets(answer: AskResponse, ref: string): EvidenceTargets | null {
  const { entities, relationships, paths } = answer.evidence;
  const entity = entities.find((item) => item.ref === ref);
  if (entity) return { entityIds: [entity.id], relationshipIds: [] };
  const relationship = relationships.find((item) => item.ref === ref);
  if (relationship) {
    return {
      entityIds: [...new Set([relationship.source_id, relationship.target_id])],
      relationshipIds: [relationship.id],
    };
  }
  const path = paths.find((item) => item.ref === ref);
  if (path) return { entityIds: [...path.entity_ids], relationshipIds: [...path.relationship_ids] };
  return null;
}

/** The evidence as a plain subgraph, for merging into the loaded graph. */
export function evidenceSubgraph(answer: AskResponse): Subgraph {
  return {
    entities: answer.evidence.entities.map((entity) => ({
      id: entity.id,
      graph_id: entity.graph_id,
      entity_type: entity.entity_type,
      properties: entity.properties,
      property_types: entity.property_types,
    })),
    relationships: answer.evidence.relationships.map((relationship) => ({
      id: relationship.id,
      graph_id: relationship.graph_id,
      relationship_type: relationship.relationship_type,
      source_id: relationship.source_id,
      target_id: relationship.target_id,
      properties: relationship.properties,
      property_types: relationship.property_types,
    })),
  };
}

/** A piece of answer text, or a group of citations such as `[E1, R2]`. */
export type AnswerToken = { text: string } | { refs: string[] };

const CITATION = /\[((?:[ERP][1-9]\d{0,5})(?:\s*,\s*[ERP][1-9]\d{0,5})*)\]/g;

/**
 * Splits answer text into plain text and citation groups. Only references
 * in `known` become citations; MindGraph Core has already removed unknown
 * ones, so anything else is left as text.
 */
export function citationTokens(text: string, known: ReadonlySet<string>): AnswerToken[] {
  const tokens: AnswerToken[] = [];
  let last = 0;
  for (const match of text.matchAll(CITATION)) {
    const refs = match[1].split(",").map((ref) => ref.trim());
    if (!refs.every((ref) => known.has(ref))) continue;
    if (match.index > last) tokens.push({ text: text.slice(last, match.index) });
    tokens.push({ refs });
    last = match.index + match[0].length;
  }
  if (last < text.length) tokens.push({ text: text.slice(last) });
  return tokens;
}

/** Relationship types of the evidence with their counts (`USES ×2`). */
export function relationshipTypeCounts(answer: AskResponse): { type: string; count: number }[] {
  const counts = new Map<string, number>();
  for (const relationship of answer.evidence.relationships) {
    counts.set(
      relationship.relationship_type,
      (counts.get(relationship.relationship_type) ?? 0) + 1,
    );
  }
  return [...counts]
    .map(([type, count]) => ({ type, count }))
    .sort((a, b) => b.count - a.count || a.type.localeCompare(b.type));
}

/** A suggested question; `submit: false` fills the input for the user to finish. */
export interface Suggestion {
  question: string;
  submit: boolean;
}

function article(name: string): string {
  return /^[aeiou]/i.test(name) ? "an" : "a";
}

/**
 * Questions that fit the selected entity's type and the ontology (or the
 * whole graph when nothing is selected). Nothing is hardcoded to a
 * particular domain: the relationship types decide.
 */
export function suggestedQuestions(
  ontology: Ontology,
  selected: { entityType: string; label: string } | null,
  max = 4,
): Suggestion[] {
  const out: Suggestion[] = [];
  const add = (question: string, submit = true) => {
    if (!out.some((s) => s.question === question)) out.push({ question, submit });
  };
  const relationships = ontology.relationship_types;
  if (selected) {
    const { entityType: type, label } = selected;
    add(`Which entities are directly connected to ${label}?`);
    for (const relationship of relationships) {
      const { source_entity_type: source, target_entity_type: target, name } = relationship;
      if (source === type && target !== type) {
        add(`Which other ${type} entities share ${article(target)} ${target} with ${label}?`);
      } else if (target === type && source !== type) {
        add(`Which ${source} entities are connected to ${label} through ${name}?`);
      }
    }
    const pathQuestion = `How is ${label} connected to `;
    return [...out.slice(0, max - 1), { question: pathQuestion, submit: false }];
  }
  for (const relationship of relationships) {
    const { source_entity_type: source, target_entity_type: target } = relationship;
    if (source !== target) add(`Which ${source} entities share ${article(target)} ${target}?`);
  }
  for (const type of ontology.entity_types.slice(0, 2)) add(`List some ${type.name} entities.`);
  return out.slice(0, max);
}
