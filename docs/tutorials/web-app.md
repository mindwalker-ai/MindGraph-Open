# Using the web app

A tour of the browser workflow: build a graph from your own files, explore
it, ask questions, connect an AI agent and review possible duplicates. To try
it on ready-made data first, load a demo (`make demo-fraud`, see
[examples](../../examples/README.md)).

## Build a graph in the browser

Open **http://localhost:3000**:

```text
Create a Graph    name it; start from CSV, JSON or an empty ontology
Upload Data       drop several files; each is parsed (rows, columns, headers)
AI Graph Builder  "Generate Graph Model": one model for all files, linked across
                  files by shared identifiers (or "Build manually")
                  review on a canvas: entity types, relationships, YAML view;
                  edit names, properties, source fields, modes and endpoints
Generate          saves the ontology, then ingests each file, creators first
```

The AI only *suggests*. The uploaded files, their analysis profiles and the
suggestion are stored as you go, but no ontology version and no graph data
are written until you click **Generate Graph**, which uses the same
endpoints as the curl examples below. When it finishes, **Explore Graph**
opens the result in the Graph Explorer. Without an
AI provider the builder says so and the manual path works the same way. See
[the AI Graph Builder guide](ai-graph-builder.md) to configure a model.

## Explore a graph

Open a graph with **Explore Graph** on its card on the home page (or on the
builder's success screen), or go to `/graphs/<graph_id>/explore`. The Graph
Explorer is a read-only investigation workspace:

```text
Find entities     exact search by entity type, property and value, or open
                  an entity by its ID (no full-text or fuzzy search)
Inspect           click an entity or relationship: typed properties and the
                  relationships loaded around it
Expand            double-click (or E) loads an entity's neighbours: both
                  directions or one, optionally one relationship type, at
                  most 100 relationships per expansion (up to 1,000)
Find Path         the shortest path between two entities, computed by
                  MindGraph Core within a maximum depth (1-6 hops), then
                  highlighted; "no path" only means none within that depth
Layouts and view  force-directed or circular layout, fit, zoom, hide entities
                  and types, reset; nothing here changes the stored graph
```

The canvas shows the subgraph loaded so far, never the whole graph: it starts
from one entity's neighbourhood and grows as you expand. The browser holds at
most 500 entities and 1,500 relationships; past that, the explorer says what
was left out and offers to unload hidden entities or reset the view. The
counts in the status bar are what is loaded, not totals of the graph.

## Ask MindGraph

In the Graph Explorer, click **Ask MindGraph** (or press A, or choose *Ask
about this entity* in an entity's context menu). The panel opens next to the
graph, which stays visible:

```text
Scope        "Selected · <entity>" (the most reliable) or "Whole graph
             (bounded)"; suggested questions follow the entity's type and
             the ontology
Answer       MindGraph Core retrieves evidence from the graph, then the
             model phrases an answer that cites it: [E1] entity, [R1]
             relationship, [P1] path
Show on      the evidence is merged into the canvas and highlighted in
Graph        orange; click a reference to focus that entity or path;
             "Clear Evidence Highlight" restores normal styling
Status       Answered · Partial (a retrieval limit was hit, results may be
             missing) · Insufficient evidence (the retrieved graph context
             does not support an answer)
Follow-up    ask again in the same panel: "Is there a path between them?",
             "What about C003?". The last 3 questions and the entities they
             cited go with it; evidence is retrieved afresh every time.
             "New conversation" (the clock icon) forgets them
```

For the fraud example, select Alice Tan and ask *"Who else shares this
device?"*: the evidence is Alice (C001), Budi Santoso (C002), `DEVICE-921`
and their two `USES` relationships. Ask MindGraph needs a configured model
(see [how Ask MindGraph works](ask-mindgraph.md)); without one the panel says so
and exploring keeps working.

## Connect an AI agent over MCP

`mindgraph-mcp` gives Claude Code, Codex, Cursor or any MCP client
read-only tools over one graph:

```text
get_graph_info   get_graph_schema   search_entities   get_entity
get_neighbors    find_path          get_subgraph      ask_graph (opt-in, GraphRAG)
```

The client starts the binary over stdio, and the binary calls MindGraph
Core's REST API on `127.0.0.1`. It never touches PostgreSQL, HugeGraph or
the AI service. Build it, then copy your client's configuration from
**MCP** in the sidebar, which fills in the graph id:

```bash
cargo build -p mindgraph-mcp --release
claude mcp add --scope project mindgraph \
  --env MINDGRAPH_MCP_CORE_URL=http://127.0.0.1:8080 \
  --env MINDGRAPH_MCP_GRAPH_ID=<graph id> \
  -- "$PWD/target/release/mindgraph-mcp"
```

Then ask your agent *"Find Customer C001 and show its relationships."*.
The tools are deterministic and bounded, and search is exact, not
full-text. `ask_graph` stays off unless you enable it, because it sends
graph evidence to Core's model provider. See [`docs/mcp/`](../mcp/README.md)
for the tools, security and the [Claude Code](../mcp/claude-code.md),
[Codex](../mcp/codex.md) and [Cursor](../mcp/cursor.md) guides.

## Review potential duplicates

**Resolution** in the sidebar finds records of one entity type that may
describe the same real-world entity:

1. Pick the matching rules: exact identifier, normalized email, phone or
   text, and fuzzy name.
2. Run a bounded scan. It reports what it covered.
3. Compare each candidate's two original records side by side, with the
   evidence for every property.

Approving a candidate (**Same Entity**, then pick the canonical record)
creates a **logical identity mapping** with provenance. Nothing in the graph
is merged, deleted or rewired, so the Graph Explorer, Ask MindGraph and MCP
keep showing the original records. An entity's details in the explorer show
its resolved identity and pending duplicates. The score ranks review work
and is not a probability. No AI is involved.

`make demo-fraud` loads `examples/fraud/customers_kyc.csv`, a second customer
source with deliberate near-duplicates (or see step 7 of the
[REST API walkthrough](rest-api.md)). See
[`docs/entity-resolution/`](../entity-resolution/README.md) for the
methods, scoring, limits and API.

