# AI Graph Builder

AI suggests, Rust validates, you review, and the deterministic M4 engine
executes. Configure one OpenAI-compatible endpoint in `.env`, then
`docker compose up -d`:

```bash
# Hosted provider
MINDGRAPH_LLM_PROVIDER=openai_compatible
MINDGRAPH_LLM_BASE_URL=https://api.openai.com/v1
MINDGRAPH_LLM_MODEL=<model name>
MINDGRAPH_LLM_API_KEY=<your key>

# ...or a local model with Ollama on the host (no key, no data leaves the machine)
MINDGRAPH_LLM_PROVIDER=openai_compatible
MINDGRAPH_LLM_BASE_URL=http://host.docker.internal:11434/v1   # Podman: host.containers.internal
MINDGRAPH_LLM_MODEL=qwen3:8b
```

Ollama listens on `127.0.0.1` by default. Docker Desktop and Podman on macOS
and Windows forward `host.docker.internal` / `host.containers.internal` to
it, but on Linux the name resolves to the bridge address, which a loopback
listener does not accept. On Linux, bind Ollama to the bridge interface
(for example `OLLAMA_HOST=172.17.0.1:11434 ollama serve`, the default Docker
bridge; check yours with `ip addr show docker0`), not to `0.0.0.0`, and keep
port 11434 closed in your firewall so only local containers can reach it.

Leave `MINDGRAPH_LLM_PROVIDER` empty to disable AI; every manual flow keeps
working. With the API (continuing the [REST API walkthrough](rest-api.md): `$CUSTOMERS`
and `$ACCOUNTS` are two analysed data sources):

```bash
curl -s $API/graph-builder/status | jq .                   # {"status": "ready", ...}
# One suggestion for several files (1-10); the shorthand
# POST .../data-sources/$SOURCE/graph-builder/suggestions models a single file.
JOB=$(curl -s -X POST $API/graphs/$GRAPH/graph-builder/suggestions \
  -H "Content-Type: application/json" -H "Idempotency-Key: $(uuidgen)" \
  -d "{\"data_source_ids\": [\"$CUSTOMERS\", \"$ACCOUNTS\"]}" | jq -r .job_id)
curl -N $API/jobs/$JOB/events                              # ends with "event: completed"
curl -s $API/jobs/$JOB | jq '.result.suggestion | {entities: [.ontology.entity_types[].name],
  files: [.mappings[] | {dataset, modes: [.mapping.entities[] | {alias, mode}]}],
  confidence, warnings}'

# Review it, then apply it with the M4 endpoints. expected_version makes the
# ontology write fail with 409 if another version became active meanwhile
# (the body is a complete ontology, so a stale one would drop newer types):
curl -s $API/jobs/$JOB | jq '.result | .suggestion.ontology + {expected_version: .ontology_version_base}' > ontology.json
# --fail-with-body stops here on 409 (or any error), so no mapping is ever
# ingested against an ontology this example did not save:
curl -s --fail-with-body -X POST $API/graphs/$GRAPH/ontology \
  -H "Content-Type: application/json" -d @ontology.json \
&& curl -s $API/jobs/$JOB | jq -c '.result.suggestion.mappings[]' | while read -r m; do
  # The mappings come in a safe order: files that create a type first.
  SOURCE=$(jq -r .data_source_id <<<"$m")
  ING=$(curl -s -X POST $API/graphs/$GRAPH/data-sources/$SOURCE/ingestions \
    -H "Content-Type: application/json" -H "Idempotency-Key: $(uuidgen)" \
    -d "$(jq '{mapping: .mapping}' <<<"$m")" | jq -r .job_id)
  curl -sN $API/jobs/$ING/events | grep -m1 -E 'event: (completed|failed)'
done
```

For `customers.csv` and `accounts.csv` together, a good suggestion has one
`Customer` type created from `customers.csv`, an `Account` type, and
`Customer ─OWNS→ Account` in the `accounts.csv` mapping, whose customer
column uses `mode: match_only`, so accounts link to the customers of the
other file. The same works one file at a time: a later suggestion keeps
existing types (marked `existing`) and matches them.

**What the model receives** when you request a suggestion, per file: its
filename as a label, field names, inferred types, null counts, up to 5
sample values per field and up to 10 sample rows; plus the graph's current
ontology. Never the files, other records, graph contents, paths or
MindGraph's own credentials. Sample values
are copied from your data unfiltered, so they can contain sensitive values
(passwords, tokens, personal data) if your dataset does: use a local model
for such data. Sample values are treated as untrusted data in the prompt; the model has no tools, and every answer is
validated by Pydantic in the AI service and again, authoritatively, by Rust
(existing types are never removed or changed). At most one repair attempt is
made when Rust rejects a suggestion. Details:
[`docs/architecture/`](../architecture/README.md#ai-graph-builder-adr-007-adr-008-m5)
and [`services/ai/README.md`](../../services/ai/README.md).
