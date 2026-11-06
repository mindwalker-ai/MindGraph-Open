/**
 * Pure helpers of the Entity Resolution workspace (M9): rule suggestions
 * from the ontology, the request a scan sends, and the wording of evidence.
 *
 * The wording is deliberate. A candidate's score orders review work; it is
 * never shown as a percentage or called a probability, and an approval is a
 * logical identity mapping, never a merge of graph entities.
 */

import type {
  Entity,
  MatchSignal,
  PropertyJson,
  PropertyType,
  ResolutionRule,
  RuleKind,
  SignalNote,
  SignalOutcome,
} from "@/lib/api/types";
import { RESOLUTION_LIMITS } from "@/lib/api/types";

export interface RuleKindInfo {
  kind: RuleKind;
  label: string;
  /** Short name used in summaries ("matched on email, phone"). */
  short: string;
  description: string;
  accepts: readonly PropertyType[];
}

export const RULE_KINDS: readonly RuleKindInfo[] = [
  {
    kind: "exact_identifier",
    label: "Exact identifier",
    short: "identifier",
    description:
      "Equal values (trimmed, case-sensitive). Use only for a property that identifies one real-world record, such as a national ID.",
    accepts: ["string", "integer"],
  },
  {
    kind: "normalized_email",
    label: "Normalized email",
    short: "email",
    description:
      "Equal addresses after trimming and lower-casing the domain. No provider alias rules (dots, +tags) are assumed.",
    accepts: ["string"],
  },
  {
    kind: "normalized_phone",
    label: "Normalized phone",
    short: "phone",
    description:
      "Equal numbers in E.164 form. National numbers (0812…) need a default country; invalid numbers never match.",
    accepts: ["string", "integer"],
  },
  {
    kind: "normalized_string",
    label: "Normalized text",
    short: "text",
    description: "Equal text after Unicode normalization, case folding and whitespace collapsing.",
    accepts: ["string"],
  },
  {
    kind: "fuzzy_name",
    label: "Fuzzy name",
    short: "fuzzy name",
    description:
      "Similar names (token-aware Jaro-Winkler) at or above the threshold. Weak evidence: common names and initials are ambiguous.",
    accepts: ["string"],
  },
];

export function kindInfo(kind: RuleKind): RuleKindInfo {
  return RULE_KINDS.find((info) => info.kind === kind) ?? RULE_KINDS[0];
}

/** A rule row of the rules panel. */
export interface RuleDraft extends ResolutionRule {
  /** Stable row key. */
  key: string;
  enabled: boolean;
}

export const DEFAULT_COUNTRY = "ID";

let rowCounter = 0;
function rowKey(): string {
  rowCounter += 1;
  return `rule-${rowCounter}`;
}

/** A new rule row for `property` with the kind's default options. */
export function ruleDraft(property: string, kind: RuleKind, enabled = true): RuleDraft {
  const draft: RuleDraft = { key: rowKey(), property, kind, enabled };
  if (kind === "fuzzy_name") draft.threshold = RESOLUTION_LIMITS.thresholdDefault;
  if (kind === "normalized_phone") draft.country = DEFAULT_COUNTRY;
  if (kind === "normalized_email") draft.case_insensitive_local_part = true;
  return draft;
}

const EMAIL = /(^|_)(e_?mail|mail)(_|$)/i;
const PHONE = /(^|_)(phone|mobile|msisdn|tel|telephone|whatsapp|hp)(_|$)/i;
const NAME = /(^|_)(full_?name|name|nama)$/i;
const NATIONAL_ID = /(^|_)(nik|ktp|npwp|ssn|passport|national_id|tax_id)$/i;

/**
 * Suggested rules for an entity type, by property name and type. Nothing is
 * treated as an identifier just because it ends in `_id`: such properties
 * are offered, unchecked, since ingestion identities are already unique.
 */
export function suggestRules(properties: { key: string; type: PropertyType }[]): RuleDraft[] {
  const rules: RuleDraft[] = [];
  for (const { key, type } of properties) {
    if (type === "string" && EMAIL.test(key)) rules.push(ruleDraft(key, "normalized_email"));
    else if ((type === "string" || type === "integer") && PHONE.test(key))
      rules.push(ruleDraft(key, "normalized_phone"));
    else if (type === "string" && NAME.test(key)) rules.push(ruleDraft(key, "fuzzy_name"));
    else if ((type === "string" || type === "integer") && NATIONAL_ID.test(key))
      rules.push(ruleDraft(key, "exact_identifier"));
    else if ((type === "string" || type === "integer") && /_id$/i.test(key))
      rules.push(ruleDraft(key, "exact_identifier", false));
  }
  const order: RuleKind[] = RULE_KINDS.map((info) => info.kind);
  return rules.sort((a, b) => order.indexOf(a.kind) - order.indexOf(b.kind));
}

/** The kinds that can compare a property of `type`. */
export function kindsFor(type: PropertyType | undefined): RuleKind[] {
  return RULE_KINDS.filter((info) => type && info.accepts.includes(type)).map((info) => info.kind);
}

/** The enabled rules as the scan endpoint expects them. */
export function rulesToRequest(drafts: RuleDraft[]): ResolutionRule[] {
  return drafts
    .filter((draft) => draft.enabled && draft.property)
    .map(({ property, kind, threshold, country, case_insensitive_local_part }) => {
      const rule: ResolutionRule = { property, kind };
      if (kind === "fuzzy_name" && threshold !== undefined) rule.threshold = threshold;
      if (kind === "normalized_phone" && country?.trim())
        rule.country = country.trim().toUpperCase();
      if (kind === "normalized_email" && case_insensitive_local_part !== undefined)
        rule.case_insensitive_local_part = case_insensitive_local_part;
      return rule;
    });
}

export const OUTCOME_LABEL: Record<SignalOutcome, string> = {
  exact: "Exact match",
  normalized: "Normalized match",
  fuzzy: "Fuzzy similarity",
  no_match: "No match",
  missing: "Missing data",
  unavailable: "Unavailable",
};

export const NOTE_LABEL: Record<SignalNote, string> = {
  initials_matched: "relies on an initial",
  case_insensitive_local_part: "local part differs in letter case",
  invalid_value: "not a valid value",
  missing_country: "national number without a default country",
  integer_phone: "stored as an integer; leading zeros may be lost",
  value_too_long: "too long to compare",
  type_mismatch: "stored with another type",
};

/** A score as text: two decimals, never a percentage. */
export function formatScore(score: number): string {
  return score.toFixed(2);
}

export type EvidenceStrength = "strong" | "moderate" | "weak";

/** Strong: an identifier, email or phone agrees. Weak: only a name does. */
export function evidenceStrength(signals: MatchSignal[]): EvidenceStrength {
  const matched = signals.filter((signal) => signal.matched).map((signal) => signal.rule);
  if (matched.some((rule) => rule !== "fuzzy_name" && rule !== "normalized_string")) {
    return "strong";
  }
  return matched.includes("normalized_string") ? "moderate" : "weak";
}

/** "matched on email, phone, fuzzy name" (properties named when unusual). */
export function matchedSummary(signals: MatchSignal[]): string {
  const parts = signals
    .filter((signal) => signal.matched)
    .map((signal) => {
      const short = kindInfo(signal.rule).short;
      return signal.property === short || short === "fuzzy name"
        ? short
        : `${short} (${signal.property})`;
    });
  return parts.length ? `matched on ${parts.join(", ")}` : "no agreeing evidence";
}

/** One row of the side-by-side comparison. */
export interface CompareRow {
  key: string;
  a: PropertyJson | undefined;
  b: PropertyJson | undefined;
  /** The evidence for this property, when a rule compared it. */
  signals: MatchSignal[];
}

/** Compared properties first (in rule order), then the others by name. */
export function compareRows(
  signals: MatchSignal[],
  a: Entity | null,
  b: Entity | null,
): CompareRow[] {
  const compared: string[] = [];
  for (const signal of signals) {
    if (!compared.includes(signal.property)) compared.push(signal.property);
  }
  const others = [
    ...new Set([...Object.keys(a?.properties ?? {}), ...Object.keys(b?.properties ?? {})]),
  ]
    .filter((key) => !compared.includes(key))
    .sort();
  return [...compared, ...others].map((key) => ({
    key,
    a: a?.properties[key],
    b: b?.properties[key],
    signals: signals.filter((signal) => signal.property === key),
  }));
}
