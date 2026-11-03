import { describe, expect, it } from "vitest";

import type { DatasetProfile, GraphBuilderSuggestion, OntologyDraft } from "@/lib/api/types";

import {
  type BuilderDraft,
  type DatasetInfo,
  addRelationshipType,
  addType,
  confidenceLevel,
  fromSuggestion,
  ingestionOrder,
  joinHint,
  manualDraft,
  ontologyUnchanged,
  rebase,
  removeProperty,
  removeType,
  renameRelationshipType,
  reverseRelationshipType,
  sampleMatches,
  setEntityField,
  toSubmission,
  toYaml,
  updateEntityMapping,
  updateProperty,
  updateType,
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

function profile(fields: string[], rows: Record<string, unknown>[] = []): DatasetProfile {
  return {
    record_count: rows.length || 3,
    fields: fields.map((name) => ({
      name,
      inferred_type: name === "balance" ? "float" : "string",
      nullable: false,
      non_null_count: 3,
      sample_values: [],
    })),
    sample_rows: rows,
  };
}

const CUSTOMERS: DatasetInfo = {
  dataSourceId: "ds-customers",
  name: "customers.csv",
  profile: profile(["customer_id", "name"]),
};

const DEVICES: DatasetInfo = {
  dataSourceId: "ds-devices",
  name: "devices.csv",
  profile: profile(
    ["device_id", "customer_id", "device_type"],
    [
      { device_id: "DEVICE-921", customer_id: "CUST-18291", device_type: "phone" },
      { device_id: "DEVICE-921", customer_id: "CUST-20417", device_type: "phone" },
      { device_id: "DEVICE-109", customer_id: null, device_type: "laptop" },
    ],
  ),
};

/** Two files at once: devices.csv only matches the customers customers.csv creates. */
function suggestion(): GraphBuilderSuggestion {
  return {
    ontology: {
      entity_types: [
        {
          name: "Customer",
          display_name: "Customer",
          properties: [
            { name: "customer_id", type: "string", required: true },
            { name: "name", type: "string", required: false },
          ],
        },
        {
          name: "Device",
          display_name: "Device",
          properties: [
            { name: "device_id", type: "string", required: true },
            { name: "device_type", type: "string", required: false },
          ],
        },
      ],
      relationship_types: [
        {
          name: "USES",
          source_entity_type: "Customer",
          target_entity_type: "Device",
          properties: [],
        },
      ],
    },
    mappings: [
      {
        dataset: "customers.csv",
        data_source_id: "ds-customers",
        mapping: {
          entities: [
            {
              alias: "customer",
              entity_type: "Customer",
              mode: "create_or_match",
              identity: [{ source_field: "customer_id", property: "customer_id" }],
              properties: [
                { source_field: "customer_id", property: "customer_id" },
                { source_field: "name", property: "name" },
              ],
            },
          ],
          relationships: [],
        },
      },
      {
        dataset: "devices.csv",
        data_source_id: "ds-devices",
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
              alias: "device",
              entity_type: "Device",
              mode: "create_or_match",
              identity: [{ source_field: "device_id", property: "device_id" }],
              properties: [
                { source_field: "device_id", property: "device_id" },
                { source_field: "device_type", property: "device_type" },
              ],
            },
          ],
          relationships: [
            { relationship_type: "USES", source: "customer", target: "device", properties: [] },
          ],
        },
      },
    ],
    confidence: 0.9,
    annotations: {
      entities: [
        { name: "Customer", confidence: 0.99, reason: "customer_id is unique", existing: true },
        { name: "Device", confidence: 0.98, reason: "device_id is unique", existing: false },
      ],
      relationships: [
        { name: "USES", confidence: 0.96, reason: "devices name their customer", existing: false },
      ],
    },
    warnings: ["device_type is sometimes empty"],
  };
}

/** The datasets listed devices first, as a user might upload them. */
function draft(base: OntologyDraft = EMPTY): BuilderDraft {
  return fromSuggestion(suggestion(), base, [DEVICES, CUSTOMERS]);
}

const type = (d: BuilderDraft, name: string) => d.types.find((t) => t.name === name)!;
const dataset = (d: BuilderDraft, id: string) => d.datasets.find((x) => x.dataSourceId === id)!;

describe("fromSuggestion", () => {
  it("builds one ontology and one mapping per dataset", () => {
    const d = draft();
    expect(d.types.map((t) => t.name)).toEqual(["Customer", "Device"]);
    expect(type(d, "Device").confidence).toBe(0.98);
    expect(d.relationshipTypes[0]).toMatchObject({
      name: "USES",
      sourceTypeKey: type(d, "Customer").key,
      targetTypeKey: type(d, "Device").key,
      confidence: 0.96,
    });
    const devices = dataset(d, "ds-devices");
    expect(devices.entities.map((e) => [e.alias, e.mode])).toEqual([
      ["customer", "match_only"],
      ["device", "create_or_match"],
    ]);
    expect(devices.relationships[0]).toMatchObject({
      sourceKey: devices.entities[0].key,
      targetKey: devices.entities[1].key,
    });
    expect(validateDraft(d)).toEqual([]);
  });

  it("marks base types existing, locks their properties and keeps missing base types", () => {
    const base: OntologyDraft = {
      ...WITH_CUSTOMER,
      entity_types: [
        ...WITH_CUSTOMER.entity_types,
        { name: "Merchant", properties: [{ name: "merchant_id", type: "string", required: true }] },
      ],
    };
    const d = draft(base);
    expect(type(d, "Customer").existing).toBe(true);
    expect(type(d, "Customer").properties.every((p) => p.locked)).toBe(true);
    expect(type(d, "Device").existing).toBe(false);
    expect(type(d, "Merchant").existing).toBe(true);
  });
});

describe("submission", () => {
  it("orders ingestions so creators come first and round-trips the M4 contracts", () => {
    const d = draft();
    expect(ingestionOrder(d)).toEqual(["ds-customers", "ds-devices"]);
    const { ontology, ingestions } = toSubmission(d);
    expect(ontology.entity_types.map((t) => t.name)).toEqual(["Customer", "Device"]);
    expect(ontology.relationship_types).toEqual(suggestion().ontology.relationship_types);
    expect(ingestions.map((i) => i.name)).toEqual(["customers.csv", "devices.csv"]);
    expect(ingestions[0].mapping).toEqual(suggestion().mappings[0].mapping);
    expect(ingestions[1].mapping).toEqual(suggestion().mappings[1].mapping);
  });

  it("skips datasets that map nothing", () => {
    const d = draft();
    const emptied: BuilderDraft = {
      ...d,
      datasets: d.datasets.map((x) =>
        x.dataSourceId === "ds-devices" ? { ...x, entities: [], relationships: [] } : x,
      ),
    };
    expect(toSubmission(emptied).ingestions.map((i) => i.dataSourceId)).toEqual(["ds-customers"]);
  });

  it("renders the draft as YAML", () => {
    const yaml = toYaml(draft());
    expect(yaml).toContain("  relationship_types:\n    - name: USES\n      source: Customer");
    expect(yaml).toContain("  - file: customers.csv");
    expect(yaml).toContain("        mode: match_only");
    expect(yaml.indexOf("file: customers.csv")).toBeLessThan(yaml.indexOf("file: devices.csv"));
  });
});

describe("edits", () => {
  it("renaming a type keeps every mapping pointing at it", () => {
    const d = draft();
    const next = updateType(d, type(d, "Device").key, { name: "Phone", displayName: "Phone" });
    const { ontology, ingestions } = toSubmission(next);
    expect(ontology.relationship_types[0].target_entity_type).toBe("Phone");
    expect(ingestions[1].mapping.entities[1].entity_type).toBe("Phone");
    expect(validateDraft(next)).toEqual([]);
  });

  it("existing types and properties cannot be renamed, retyped or removed", () => {
    const d = draft(WITH_CUSTOMER);
    const customer = type(d, "Customer");
    let next = updateType(d, customer.key, { name: "Client" });
    expect(type(next, "Customer")).toBeDefined();
    next = updateProperty(next, "entity", customer.key, customer.properties[0].key, {
      type: "integer",
      required: false,
    });
    expect(type(next, "Customer").properties[0]).toMatchObject({ type: "string", required: true });
    next = removeProperty(next, "entity", customer.key, customer.properties[0].key);
    expect(type(next, "Customer").properties).toHaveLength(2);
    expect(removeType(next, customer.key)).toBe(next);
  });

  it("removing a type removes its mappings and relationship types", () => {
    const d = draft();
    const next = removeType(d, type(d, "Device").key);
    expect(next.relationshipTypes).toEqual([]);
    expect(dataset(next, "ds-devices").entities.map((e) => e.alias)).toEqual(["customer"]);
    expect(dataset(next, "ds-devices").relationships).toEqual([]);
  });

  it("an alias briefly equal to another one does not take its relationships", () => {
    const d = draft();
    const devices = dataset(d, "ds-devices");
    const [customer, device] = devices.entities;
    let next = updateEntityMapping(d, "ds-devices", device.key, { alias: "customer" });
    expect(validateDraft(next)).toContain('Alias "customer" is used twice in devices.csv.');
    next = updateEntityMapping(next, "ds-devices", device.key, { alias: "customer_x" });
    expect(dataset(next, "ds-devices").relationships[0]).toMatchObject({
      sourceKey: customer.key,
      targetKey: device.key,
    });
    expect(toSubmission(next).ingestions[1].mapping.relationships[0]).toMatchObject({
      source: "customer",
      target: "customer_x",
    });
  });

  it("reverses and renames new relationship types with their mappings", () => {
    const d = draft();
    const key = d.relationshipTypes[0].key;
    const next = renameRelationshipType(reverseRelationshipType(d, key), key, "USED_BY");
    const { ontology, ingestions } = toSubmission(next);
    expect(ontology.relationship_types[0]).toMatchObject({
      name: "USED_BY",
      source_entity_type: "Device",
      target_entity_type: "Customer",
    });
    expect(ingestions[1].mapping.relationships[0]).toMatchObject({
      relationship_type: "USED_BY",
      source: "device",
      target: "customer",
    });
  });

  it("adds a relationship type mapped in a file that holds both types", () => {
    const d = draft();
    const next = addRelationshipType(d, type(d, "Device").key, type(d, "Customer").key);
    expect(next.relationshipTypes[1].name).toBe("RELATED_TO");
    expect(dataset(next, "ds-devices").relationships).toHaveLength(2);
    expect(validateDraft(next)).toEqual([]);
  });

  it("adds an entity type that still needs an identity", () => {
    const next = addType(draft(), "ds-customers");
    expect(next.types.at(-1)?.name).toBe("Entity");
    expect(validateDraft(next)).toContain(
      "Entity Entity in customers.csv needs at least one identity property.",
    );
  });
});

describe("validateDraft", () => {
  it("reports what the server would reject", () => {
    let d = draft();
    const device = type(d, "Device");
    d = updateType(d, device.key, { name: "Bad Name" });
    const devices = dataset(d, "ds-devices");
    d = setEntityField(d, "ds-devices", devices.entities[1].key, device.properties[0].key, {
      sourceField: "missing",
    });
    const errors = validateDraft(d);
    expect(errors.some((e) => e.includes('"Bad Name"'))).toBe(true);
    expect(errors).toContain('Source field "missing" does not exist in devices.csv.');
  });

  it("rejects matching a type that no file creates", () => {
    const d = draft();
    const customers = dataset(d, "ds-customers");
    const next = updateEntityMapping(d, "ds-customers", customers.entities[0].key, {
      mode: "match_only",
    });
    expect(validateDraft(next).some((e) => e.startsWith("Customer is only matched"))).toBe(true);
  });

  it("rejects new required properties on existing types", () => {
    const d = draft(WITH_CUSTOMER);
    const customer = type(d, "Customer");
    const added = {
      ...d,
      types: d.types.map((t) =>
        t.key === customer.key
          ? {
              ...t,
              properties: [
                ...t.properties,
                {
                  key: "x",
                  name: "segment",
                  type: "string" as const,
                  required: true,
                  locked: false,
                },
              ],
            }
          : t,
      ),
    };
    expect(validateDraft(added)).toContain(
      'New property "segment" of existing type Customer must be optional.',
    );
  });
});

describe("manualDraft", () => {
  it("starts with one valid entity per file", () => {
    const d = manualDraft([CUSTOMERS, DEVICES], EMPTY);
    expect(d.types.map((t) => t.name)).toEqual(["Customer", "Device"]);
    expect(dataset(d, "ds-customers").entities[0].alias).toBe("customer");
    expect(validateDraft(d)).toEqual([]);
  });

  it("reuses existing types", () => {
    const d = manualDraft([CUSTOMERS], WITH_CUSTOMER);
    expect(d.types).toHaveLength(1);
    expect(d.types[0].existing).toBe(true);
    expect(validateDraft(d)).toEqual([]);
  });
});

describe("helpers", () => {
  it("explains cross-file joins and shows sample matches", () => {
    const d = draft();
    const devices = dataset(d, "ds-devices");
    expect(joinHint(d, devices, devices.relationships[0])).toEqual({
      from: "devices.csv.customer_id",
      to: "customers.csv.customer_id",
    });
    expect(sampleMatches(devices, devices.relationships[0])).toEqual({
      source: "customer_id",
      target: "device_id",
      rows: [
        ["CUST-18291", "DEVICE-921"],
        ["CUST-20417", "DEVICE-921"],
      ],
    });
  });

  it("detects unchanged ontologies and rebases drafts", () => {
    const d = draft();
    const { ontology } = toSubmission(d);
    expect(ontologyUnchanged(EMPTY, ontology)).toBe(false);
    const rebased = rebase(d, ontology);
    expect(rebased.types.every((t) => t.existing)).toBe(true);
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
