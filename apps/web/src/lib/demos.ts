/**
 * The demo scenarios bundled in `examples/` (M10).
 *
 * The web app does not load demos itself: `make demo-<name>` (the `mindgraph
 * demo` loader) loads them through Core's public REST API on the machine
 * running MindGraph. These entries describe each scenario and how to find
 * its graph once loaded. `demos.test.ts` keeps them in sync with each
 * scenario's `examples/<slug>/demo.json`.
 */

import type { GraphProject } from "@/lib/api/types";

export interface DemoFocus {
  entity_type: string;
  property: string;
  value: string;
}

export interface DemoScenario {
  /** The folder under `examples/`, and the loader's scenario name. */
  slug: "fraud" | "cyber" | "knowledge-graph";
  title: string;
  description: string;
  /** Exact graph name the loader creates (`graph.name` in demo.json). */
  graphName: string;
  /** `make` target that loads it. */
  command: string;
  entityTypes: string[];
  relationshipTypes: string[];
  /** What to look at once it is loaded; true of the bundled data. */
  highlights: string[];
  /** The entity the Explorer opens on (`explore` in demo.json). */
  focus: DemoFocus;
  /** Whether the scenario includes entity resolution candidates. */
  resolution: boolean;
  /** A colour token of the type palette. */
  color: string;
}

export const DEMOS: DemoScenario[] = [
  {
    slug: "fraud",
    title: "Fraud Investigation",
    description: "Trace shared devices and merchants across customers and accounts.",
    graphName: "Demo: Fraud investigation",
    command: "make demo-fraud",
    entityTypes: ["Customer", "Account", "Device", "Transaction", "Merchant"],
    relationshipTypes: ["OWNS", "USES", "SENT", "TO"],
    highlights: [
      "Alice Tan and Budi Santoso use the same device, DEVICE-921.",
      "Their accounts paid CryptoSwap Exchange six minutes apart.",
      "Seven possible duplicate customer records wait for review.",
    ],
    focus: { entity_type: "Customer", property: "customer_id", value: "C001" },
    resolution: true,
    color: "var(--risk)",
  },
  {
    slug: "cyber",
    title: "Cyber Investigation",
    description: "Follow users, endpoints and processes to the hosts they contacted.",
    graphName: "Demo: Cyber investigation",
    command: "make demo-cyber",
    entityTypes: ["User", "Endpoint", "Process", "IPAddress", "Domain"],
    relationshipTypes: ["LOGGED_INTO", "EXECUTED", "CONNECTED_TO", "RESOLVED_TO"],
    highlights: [
      "Two Finance workstations ran updater.exe from a temporary folder.",
      "Both processes connected to 203.0.113.45, which two domains resolve to.",
      "Synthetic activity for practice: nothing in it is a confirmed incident.",
    ],
    focus: { entity_type: "Endpoint", property: "endpoint_id", value: "WS-FIN-07" },
    resolution: false,
    color: "var(--t-ip)",
  },
  {
    slug: "knowledge-graph",
    title: "Company Knowledge Graph",
    description: "Map people, companies, projects, documents and topics.",
    graphName: "Demo: Company knowledge graph",
    command: "make demo-knowledge",
    entityTypes: ["Person", "Company", "Project", "Document", "Topic"],
    relationshipTypes: ["WORKS_AT", "CONTRIBUTES_TO", "AUTHORED", "REFERENCES", "ABOUT"],
    highlights: [
      "Four people from two companies contribute to Flood Early Warning.",
      "Documents link projects to topics such as time-series forecasting.",
      "A Tirta Data checklist leads to Arunika Labs in three hops.",
    ],
    focus: { entity_type: "Project", property: "project_id", value: "PRJ-FLOOD" },
    resolution: false,
    color: "var(--t-customer)",
  },
];

/**
 * The loaded graph of a scenario: the one with its exact name, else the
 * newest copy made with `--new` (`<name> · <timestamp>`). `graphs` is
 * newest first, as `GET /graphs` returns it.
 */
export function demoGraph(demo: DemoScenario, graphs: GraphProject[]): GraphProject | undefined {
  return (
    graphs.find((graph) => graph.name === demo.graphName) ??
    graphs.find((graph) => graph.name.startsWith(`${demo.graphName} · `))
  );
}
