import { describe, expect, it } from "vitest";

import type { DatasetProfile, GraphBuilderSuggestion, OntologyDraft } from "@/lib/api/types";

import {
  addEntity,
  confidenceLevel,
  fromSuggestion,
  manualDraft,
  ontologyUnchanged,
  rebase,
  removeEntity,
  removeProperty,
  removeRelationship,
  toSubmission,
  updateEntity,
  updateProperty,
  validateDraft,
} from "./draft";

const EMPTY: OntologyDraft = { entity_types: [], relationship_types: [] };

const WITH_CUSTOMER: OntologyDraft = {
  entity_types: [
    {
      name: "Customer",
      display_name: "Customer",
      properties: [
        { name: "customer_id", type: "string", required: true },
        { name: "name", type: "string", required: false },
      ],
    },
  ],
  relationship_types: [],
};

const ACCOUNT_FIELDS = ["account_id", "customer_id", "balance"];

/** The M5 multi-dataset case: existing Customer, new Account, OWNS. */
function accountsSuggestion(): GraphBuilderSuggestion {
  return {
    ontology: {
      entity_types: [
        ...WITH_CUSTOMER.entity_types,
        {
          name: "Account",
          display_name: "Account",
          properties: [
            { name: "account_id", type: "string", required: true },
            { name: "balance", type: "float", required: false },
          ],
        },
      ],
      relationship_types: [
        {
          name: "OWNS",
          source_entity_type: "Customer",
          target_entity_type: "Account",
          properties: [],
        },
      ],
    },
    mapping: {
      entities: [
        {
          alias: "customer",
          entity_type: "Customer",
          mode: "match_only",
          identity: [{ source_field: "customer_id", property: "customer_id" }],
          properties: [],
        },
        {
          alias: "account",
          entity_type: "Account",
          mode: "create_or_match",
          identity: [{ source_field: "account_id", property: "account_id" }],
          properties: [
            { source_field: "account_id", property: "account_id" },
            { source_field: "balance", property: "balance" },
          ],
        },
      ],
      relationships: [
        { relationship_type: "OWNS", source: "customer", target: "account", properties: [] },
      ],
    },
    confidence: 0.9,
    annotations: {
      entities: [
        {
          name: "Customer",
          confidence: 0.95,
          reason: "references the existing Customer",
          existing: true,
        },
        {
          name: "Account",
          confidence: 0.55,
          reason: "account_id identifies each record",
          existing: false,
        },
      ],
      relationships: [
        { name: "OWNS", confidence: 0.91, reason: "co-occurring ids", existing: false },
      ],
    },
    warnings: ["balance is sometimes empty"],
  };
}

describe("fromSuggestion", () => {
  it("marks existing types and locks their properties", () => {
    const draft = fromSuggestion(accountsSuggestion(), WITH_CUSTOMER);
    const [customer, account] = draft.entities;
    expect(customer.existing).toBe(true);
    expect(customer.mode).toBe("match_only");
    expect(customer.properties.every((p) => p.locked)).toBe(true);
    expect(customer.properties.find((p) => p.name === "customer_id")).toMatchObject({
      identity: true,
      sourceField: "customer_id",
    });
    expect(account.existing).toBe(false);
    expect(account.confidence).toBe(0.55);
    expect(draft.relationships[0]).toMatchObject({
      typeName: "OWNS",
      sourceKey: customer.key,
      targetKey: account.key,
    });
    expect(validateDraft(draft, ACCOUNT_FIELDS)).toEqual([]);
  });

  it("round-trips into the M4 ontology and mapping contracts", () => {
    const { ontology, mapping } = toSubmission(fromSuggestion(accountsSuggestion(), WITH_CUSTOMER));
    expect(ontology.entity_types.map((t) => t.name)).toEqual(["Customer", "Account"]);
    expect(ontology.entity_types[0].properties).toEqual(WITH_CUSTOMER.entity_types[0].properties);
    expect(ontology.relationship_types).toEqual(accountsSuggestion().ontology.relationship_types);
    expect(mapping).toEqual(accountsSuggestion().mapping);
  });
});

describe("edits", () => {
  it("deleting an entity removes its relationships", () => {
    const draft = fromSuggestion(accountsSuggestion(), WITH_CUSTOMER);
    const next = removeEntity(draft, draft.entities[1].key);
    expect(next.entities.map((e) => e.alias)).toEqual(["customer"]);
    expect(next.relationships).toEqual([]);
  });

  it("renaming an alias keeps its relationships", () => {
    const draft = fromSuggestion(accountsSuggestion(), WITH_CUSTOMER);
    const next = updateEntity(draft, draft.entities[1].key, { alias: "acct" });
    expect(next.relationships[0].targetKey).toBe(draft.entities[1].key);
    expect(toSubmission(next).mapping.relationships[0]).toMatchObject({
      source: "customer",
      target: "acct",
    });
    expect(validateDraft(next, ACCOUNT_FIELDS)).toEqual([]);
  });

  it("an alias briefly equal to another one does not take its relationships", () => {
    const draft = fromSuggestion(accountsSuggestion(), WITH_CUSTOMER);
    const [customer, account] = draft.entities;
    // Typing "customer_x" into the account alias, one keystroke at a time.
    let next = updateEntity(draft, account.key, { alias: "customer" });
    expect(validateDraft(next, ACCOUNT_FIELDS)).toContain('Alias "customer" is used twice.');
    next = updateEntity(next, account.key, { alias: "customer_x" });
    expect(next.relationships[0]).toMatchObject({
      sourceKey: customer.key,
      targetKey: account.key,
    });
    expect(toSubmission(next).mapping.relationships[0]).toMatchObject({
      source: "customer",
      target: "customer_x",
    });
  });

  it("existing properties cannot be retyped or removed, only remapped", () => {
    const draft = fromSuggestion(accountsSuggestion(), WITH_CUSTOMER);
    const customer = draft.entities[0];
    const id = customer.properties[0];
    let next = updateProperty(draft, "entity", customer.key, id.key, {
      type: "integer",
      required: false,
      sourceField: "account_id",
    });
    expect(next.entities[0].properties[0]).toMatchObject({
      type: "string",
      required: true,
      sourceField: "account_id",
    });
    next = removeProperty(next, "entity", customer.key, id.key);
    expect(next.entities[0].properties).toHaveLength(2);
  });

  it("deleting a relationship leaves the entities", () => {
    const draft = fromSuggestion(accountsSuggestion(), WITH_CUSTOMER);
    const next = removeRelationship(draft, draft.relationships[0].key);
    expect(next.relationships).toEqual([]);
    expect(next.entities).toHaveLength(2);
  });
});

describe("validateDraft", () => {
  it("reports what the server would reject", () => {
    let draft = fromSuggestion(accountsSuggestion(), WITH_CUSTOMER);
    const account = draft.entities[1];
    draft = updateEntity(draft, account.key, { typeName: "Bank Account" });
    draft = updateProperty(draft, "entity", account.key, account.properties[0].key, {
      sourceField: null,
    });
    draft = addEntity(draft);
    const errors = validateDraft(draft, ACCOUNT_FIELDS);
    expect(errors.some((e) => e.includes('"Bank Account"'))).toBe(true);
    expect(errors.some((e) => e.includes("needs a source field"))).toBe(true);
    expect(errors.some((e) => e.includes("needs at least one identity property"))).toBe(true);
  });

  it("rejects new required properties on existing types", () => {
    const draft = fromSuggestion(accountsSuggestion(), WITH_CUSTOMER);
    const customer = draft.entities[0];
    const added = {
      ...draft,
      entities: draft.entities.map((e) =>
        e.key === customer.key
          ? {
              ...e,
              properties: [
                ...e.properties,
                {
                  key: "x",
                  name: "segment",
                  type: "string" as const,
                  required: true,
                  sourceField: null,
                  identity: false,
                  locked: false,
                },
              ],
            }
          : e,
      ),
    };
    expect(validateDraft(added, ACCOUNT_FIELDS)).toContain(
      'New property "segment" of existing type Customer must be optional.',
    );
  });
});

describe("manualDraft", () => {
  const profile: DatasetProfile = {
    record_count: 3,
    fields: [
      {
        name: "customer_id",
        inferred_type: "integer",
        nullable: false,
        non_null_count: 3,
        sample_values: ["7"],
      },
      {
        name: "full name",
        inferred_type: "string",
        nullable: true,
        non_null_count: 2,
        sample_values: [],
      },
    ],
    sample_rows: [],
  };

  it("starts with one valid entity holding every field", () => {
    const draft = manualDraft(profile, EMPTY);
    expect(draft.entities).toHaveLength(1);
    const [entity] = draft.entities;
    expect(entity.typeName).toBe("Customer");
    expect(entity.properties.map((p) => p.name)).toEqual(["customer_id", "full_name"]);
    expect(entity.properties[0]).toMatchObject({ identity: true, type: "string", required: true });
    expect(
      validateDraft(
        draft,
        profile.fields.map((f) => f.name),
      ),
    ).toEqual([]);
  });
});

describe("helpers", () => {
  it("detects unchanged ontologies and rebases drafts", () => {
    const draft = fromSuggestion(accountsSuggestion(), WITH_CUSTOMER);
    const { ontology } = toSubmission(draft);
    expect(ontologyUnchanged(WITH_CUSTOMER, ontology)).toBe(false);
    const rebased = rebase(draft, ontology);
    expect(rebased.entities.every((e) => e.existing)).toBe(true);
    expect(ontologyUnchanged(ontology, toSubmission(rebased).ontology)).toBe(true);
  });

  it("bands confidence", () => {
    expect([0.97, 0.85, 0.7, 0.6, 0.3].map(confidenceLevel)).toEqual([
      "high",
      "high",
      "medium",
      "medium",
      "low",
    ]);
  });
});
