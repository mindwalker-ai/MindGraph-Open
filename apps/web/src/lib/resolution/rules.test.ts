import { describe, expect, it } from "vitest";

import type { Entity, MatchSignal } from "@/lib/api/types";

import {
  compareRows,
  evidenceStrength,
  formatScore,
  kindsFor,
  matchedSummary,
  ruleDraft,
  rulesToRequest,
  suggestRules,
} from "./rules";

const signal = (property: string, rule: MatchSignal["rule"], matched: boolean): MatchSignal => ({
  property,
  rule,
  outcome: matched ? "normalized" : "no_match",
  matched,
  weight: matched ? 0.9 : 0,
});

describe("rule suggestions", () => {
  it("suggests rules by property name and type, never ids by default", () => {
    const rules = suggestRules([
      { key: "customer_id", type: "string" },
      { key: "name", type: "string" },
      { key: "email", type: "string" },
      { key: "phone", type: "string" },
      { key: "age", type: "integer" },
      { key: "nik", type: "string" },
    ]);
    expect(rules.map((r) => [r.property, r.kind, r.enabled])).toEqual([
      ["customer_id", "exact_identifier", false],
      ["nik", "exact_identifier", true],
      ["email", "normalized_email", true],
      ["phone", "normalized_phone", true],
      ["name", "fuzzy_name", true],
    ]);
    expect(rules.find((r) => r.kind === "fuzzy_name")?.threshold).toBe(0.9);
    expect(rules.find((r) => r.kind === "normalized_phone")?.country).toBe("ID");
  });

  it("offers only kinds that can compare the property type", () => {
    expect(kindsFor("string")).toHaveLength(5);
    expect(kindsFor("integer")).toEqual(["exact_identifier", "normalized_phone"]);
    expect(kindsFor("boolean")).toEqual([]);
  });

  it("sends only enabled rules with the options of their kind", () => {
    const email = ruleDraft("email", "normalized_email");
    const phone = { ...ruleDraft("phone", "normalized_phone"), country: " id " };
    const off = ruleDraft("customer_id", "exact_identifier", false);
    const name = { ...ruleDraft("name", "fuzzy_name"), threshold: 0.85, country: "XX" };
    expect(rulesToRequest([email, phone, off, name])).toEqual([
      { property: "email", kind: "normalized_email", case_insensitive_local_part: true },
      { property: "phone", kind: "normalized_phone", country: "ID" },
      { property: "name", kind: "fuzzy_name", threshold: 0.85 },
    ]);
  });
});

describe("evidence wording", () => {
  it("shows scores as decimals, never percentages", () => {
    expect(formatScore(0.9853)).toBe("0.99");
    expect(formatScore(0.6)).toBe("0.60");
  });

  it("grades evidence by the strongest agreeing rule", () => {
    expect(evidenceStrength([signal("name", "fuzzy_name", true)])).toBe("weak");
    expect(evidenceStrength([signal("city", "normalized_string", true)])).toBe("moderate");
    expect(
      evidenceStrength([
        signal("name", "fuzzy_name", true),
        signal("phone", "normalized_phone", true),
      ]),
    ).toBe("strong");
  });

  it("summarises what matched", () => {
    expect(
      matchedSummary([
        signal("email", "normalized_email", true),
        signal("mobile", "normalized_phone", true),
        signal("name", "fuzzy_name", true),
        signal("customer_id", "exact_identifier", false),
      ]),
    ).toBe("matched on email, phone (mobile), fuzzy name");
    expect(matchedSummary([])).toBe("no agreeing evidence");
  });

  it("puts compared properties first, then the rest by name", () => {
    const entity = (properties: Entity["properties"]): Entity => ({
      id: "e",
      graph_id: "g",
      entity_type: "Customer",
      properties,
      property_types: {},
    });
    const rows = compareRows(
      [signal("phone", "normalized_phone", true), signal("email", "normalized_email", false)],
      entity({ name: "A", phone: "+62", city: "Jakarta" }),
      entity({ name: "B", email: "b@example.com" }),
    );
    expect(rows.map((row) => row.key)).toEqual(["phone", "email", "city", "name"]);
    expect(rows[1]).toMatchObject({ a: undefined, b: "b@example.com" });
    expect(rows[0].signals).toHaveLength(1);
  });
});
