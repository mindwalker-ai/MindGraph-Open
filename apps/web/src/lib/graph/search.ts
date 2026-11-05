/**
 * Exact entity search: turns the form (entity type, property, value) into a
 * typed `EntitySearchQuery` using the active ontology. MindGraph Core
 * matches property values exactly; there is no full-text or fuzzy search.
 */

import type {
  Entity,
  EntitySearchQuery,
  Ontology,
  PropertyJson,
  PropertyType,
} from "@/lib/api/types";
import { EXPLORER_LIMITS } from "@/lib/api/types";

export interface SearchForm {
  /** Empty for any entity type. */
  entityType: string;
  /** Empty to list entities of the type without a property filter. */
  property: string;
  value: string;
  limit: number;
}

export const DEFAULT_SEARCH_LIMIT = 25;

export interface PropertyChoice {
  key: string;
  type: PropertyType;
  /** The key has different types in different entity types. */
  ambiguous: boolean;
}

/** The properties that can be searched for an entity type (or any type). */
export function propertyChoices(ontology: Ontology | null, entityType: string): PropertyChoice[] {
  const types = (ontology?.entity_types ?? []).filter(
    (type) => !entityType || type.name === entityType,
  );
  const choices = new Map<string, PropertyChoice>();
  for (const type of types) {
    for (const property of type.properties) {
      const known = choices.get(property.key);
      if (!known) choices.set(property.key, { ...property, ambiguous: false });
      else if (known.type !== property.type) known.ambiguous = true;
    }
  }
  return [...choices.values()].sort((a, b) => a.key.localeCompare(b.key));
}

export type SearchBuild = { ok: true; query: EntitySearchQuery } | { ok: false; error: string };

type Typed = { value: PropertyJson } | { error: string };

function typed(raw: string, type: PropertyType): Typed {
  const value = raw.trim();
  switch (type) {
    case "integer":
      // Sent as a decimal string, so large identifiers keep every digit.
      return /^-?\d+$/.test(value) ? { value } : { error: "Enter a whole number" };
    case "float": {
      const number = Number(value);
      return value && Number.isFinite(number) ? { value: number } : { error: "Enter a number" };
    }
    case "boolean":
      return /^(true|false)$/i.test(value)
        ? { value: value.toLowerCase() === "true" }
        : { error: "Enter true or false" };
    case "datetime":
      return Number.isNaN(Date.parse(value))
        ? { error: "Enter an RFC 3339 date-time, e.g. 2026-01-31T10:00:00Z" }
        : { value };
    case "string_array":
      return {
        value: value
          .split(",")
          .map((item) => item.trim())
          .filter(Boolean),
      };
    case "string":
      return { value };
  }
}

/** Validates the form and types the value; nothing is sent when it fails. */
export function buildSearchQuery(form: SearchForm, ontology: Ontology | null): SearchBuild {
  const { limit } = form;
  if (!Number.isInteger(limit) || limit < 1 || limit > EXPLORER_LIMITS.searchMax) {
    return { ok: false, error: `The limit must be between 1 and ${EXPLORER_LIMITS.searchMax}` };
  }
  const entityType = form.entityType.trim();
  if (entityType && !ontology?.entity_types.some((type) => type.name === entityType)) {
    return { ok: false, error: `${entityType} is not an entity type of the active ontology` };
  }
  const query: EntitySearchQuery = { limit, ...(entityType ? { entity_type: entityType } : {}) };
  const key = form.property.trim();
  if (!key) {
    return form.value.trim()
      ? { ok: false, error: "Choose the property to match the value against" }
      : { ok: true, query };
  }
  const choice = propertyChoices(ontology, entityType).find((c) => c.key === key);
  if (!choice) return { ok: false, error: `${key} is not a property of this entity type` };
  if (choice.ambiguous) {
    return { ok: false, error: `${key} has different types; choose an entity type first` };
  }
  if (!form.value.trim()) return { ok: false, error: "Enter the exact value to match" };
  const result = typed(form.value, choice.type);
  if ("error" in result) return { ok: false, error: result.error };
  return { ok: true, query: { ...query, property_filters: { [key]: result.value } } };
}

export type SearchState =
  | { status: "idle" }
  | { status: "loading"; query: EntitySearchQuery }
  | { status: "done"; query: EntitySearchQuery; results: Entity[]; limit: number }
  | { status: "error"; query: EntitySearchQuery | null; message: string };

/** A one-line, honest description of a search outcome. */
export function searchSummary(state: SearchState): string {
  switch (state.status) {
    case "idle":
      return "";
    case "loading":
      return "Searching…";
    case "error":
      return state.message;
    case "done":
      if (state.results.length === 0) return "No entities match this exact search";
      if (state.results.length >= state.limit) {
        return `First ${state.results.length} matches; narrow the search to see others`;
      }
      return `${state.results.length} ${state.results.length === 1 ? "match" : "matches"}`;
  }
}
