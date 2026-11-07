# Company knowledge graph demo

People, the companies they work for, the projects they contribute to, the
documents they write and the topics those documents cover, across three
fictional organisations: Arunika Labs (applied AI research), Kalana Systems
(industrial software) and Tirta Data (water utility analytics).

```bash
make demo-knowledge
```

Re-running reuses and verifies the graph `Demo: Company knowledge graph`.
Without make: `docker compose --profile demo run --rm demo knowledge-graph`.

## The data

| File | Rows | Becomes |
|---|---|---|
| `people.csv` | 6 | `Person` and `Company`, `Person -WORKS_AT-> Company` (since) |
| `contributions.csv` | 9 | `Project`, `Person -CONTRIBUTES_TO-> Project` (role) |
| `documents.csv` | 7 | `Document`, `Person -AUTHORED-> Document` (one row per author) |
| `document_topics.csv` | 7 | `Topic`, `Document -ABOUT-> Topic` |
| `project_references.csv` | 5 | `Document -REFERENCES-> Project` (section) |

## What the loader checks

| Check | Expected |
|---|---|
| Entities | 6 Person, 3 Company, 3 Project, 5 Document, 4 Topic |
| Relationships created | 34 |
| Flood Early Warning (PRJ-FLOOD) | 4 contributors |
| D-101 | 2 authors |
| Time-series forecasting (T-TS) | 2 documents about it |
| Path Fajar Nugroho → Sari Anggraini | 2 hops: `CONTRIBUTES_TO · CONTRIBUTES_TO` |
| Path D-105 → Arunika Labs | 3 hops: `REFERENCES · CONTRIBUTES_TO · WORKS_AT` |

## What to look at

1. Start on **Flood Early Warning**: four people from two companies (Arunika
   Labs and Tirta Data) contribute to it.
2. Expand **D-101** to see its two authors from different companies and its
   two topics.
3. Find Path from the *Gauge data quality checklist* (D-105) to **Arunika
   Labs**: the checklist references the project that an Arunika researcher
   contributes to.
4. With an LLM provider configured, ask: *"Who works on topics related to
   time-series forecasting?"*
