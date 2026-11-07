import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import type { GraphProject } from "@/lib/api/types";

import { DEMOS, demoGraph } from "./demos";

interface Manifest {
  graph: { name: string };
  ontology: string;
  explore: { entity_type: string; property: string; value: unknown };
  resolution?: unknown;
}

function readExample<T>(slug: string, file: string): T {
  const url = new URL(`../../../../examples/${slug}/${file}`, import.meta.url);
  return JSON.parse(readFileSync(url, "utf8")) as T;
}

function graph(name: string, id = name): GraphProject {
  return {
    id,
    workspace_id: "w",
    name,
    description: null,
    status: "active",
    created_at: "2026-11-06T00:00:00Z",
    updated_at: "2026-11-06T00:00:00Z",
  };
}

describe("demo scenarios", () => {
  it.each(DEMOS.map((demo) => [demo.slug, demo] as const))(
    "%s matches its examples/<slug>/demo.json",
    (slug, demo) => {
      const manifest = readExample<Manifest>(slug, "demo.json");
      expect(demo.graphName).toBe(manifest.graph.name);
      expect(demo.focus).toEqual(manifest.explore);
      expect(demo.resolution).toBe(manifest.resolution !== undefined);
      const ontology = readExample<{
        entity_types: { name: string }[];
        relationship_types: { name: string }[];
      }>(slug, manifest.ontology);
      expect([...demo.entityTypes].sort()).toEqual(ontology.entity_types.map((t) => t.name).sort());
      expect([...demo.relationshipTypes].sort()).toEqual(
        ontology.relationship_types.map((t) => t.name).sort(),
      );
    },
  );

  it("finds a loaded demo by its exact name, else the newest --new copy", () => {
    const fraud = DEMOS[0];
    expect(demoGraph(fraud, [graph("Fraud Investigation")])).toBeUndefined();
    expect(
      demoGraph(fraud, [
        graph(`${fraud.graphName} · 2026-11-06 10:00:00 UTC`, "copy"),
        graph(fraud.graphName, "exact"),
      ])?.id,
    ).toBe("exact");
    expect(
      demoGraph(fraud, [
        graph(`${fraud.graphName} · 2026-11-06 11:00:00 UTC`, "newest"),
        graph(`${fraud.graphName} · 2026-11-06 10:00:00 UTC`, "older"),
      ])?.id,
    ).toBe("newest");
    expect(demoGraph(fraud, [graph(`${fraud.graphName} (mine)`)])).toBeUndefined();
  });
});
