# Building a graph through the REST API

This walkthrough builds a graph with `curl` only, the same way the web app
and the demo loader do. The files in [`examples/fraud/`](../../examples/fraud/) build
`Customer ──OWNS──> Account` and `Customer ──USES──> Device` deterministically
(synthetic data). The commands use
[`jq`](https://jqlang.org) to read ids; without it, copy the ids by hand.
Nothing here needs Python or the AI service.

```bash
API=http://localhost:8080/api/v1

# 1. Create a graph and define its ontology (Customer, Account, OWNS).
GRAPH=$(curl -s -X POST $API/graphs -H "Content-Type: application/json" \
  -d '{"name": "Fraud demo"}' | jq -r .id)
curl -s -X POST $API/graphs/$GRAPH/ontology -H "Content-Type: application/json" \
  -d @examples/fraud/ontology.json | jq '{version, types: [.entity_types[].name]}'

# 2. Upload customers.csv and analyse it (an asynchronous job).
SOURCE=$(curl -s -F file=@examples/fraud/customers.csv $API/graphs/$GRAPH/data-sources | jq -r .id)
JOB=$(curl -s -X POST $API/graphs/$GRAPH/data-sources/$SOURCE/analyze | jq -r .job_id)
curl -s $API/jobs/$JOB | jq '{status, progress}'          # repeat until "completed"

# 3. Inspect the inferred schema.
curl -s $API/graphs/$GRAPH/data-sources/$SOURCE \
  | jq '.profile.fields[] | {name, inferred_type, nullable}'

# 4. Submit the mapping and watch the ingestion job (Server-Sent Events).
JOB=$(curl -s -X POST $API/graphs/$GRAPH/data-sources/$SOURCE/ingestions \
  -H "Content-Type: application/json" -H "Idempotency-Key: $(uuidgen)" \
  -d @examples/fraud/customers.mapping.json | jq -r .id)
curl -N $API/jobs/$JOB/events                              # ends with "event: completed"

# 5. Accounts: create Account entities, look up existing customers, link OWNS.
SOURCE=$(curl -s -F file=@examples/fraud/accounts.csv $API/graphs/$GRAPH/data-sources | jq -r .id)
JOB=$(curl -s -X POST $API/graphs/$GRAPH/data-sources/$SOURCE/analyze | jq -r .job_id)
sleep 1 && curl -s $API/jobs/$JOB | jq .status
JOB=$(curl -s -X POST $API/graphs/$GRAPH/data-sources/$SOURCE/ingestions \
  -H "Content-Type: application/json" -H "Idempotency-Key: $(uuidgen)" \
  -d @examples/fraud/accounts.mapping.json | jq -r .id)
sleep 1 && curl -s $API/jobs/$JOB | jq '{status, result}'
# → {"records_processed": 7, "entities_created": 7, "entities_matched": 5, "relationships_created": 7}

# 6. Devices: two customers share DEVICE-921 (Customer ─USES→ Device).
SOURCE=$(curl -s -F file=@examples/fraud/devices.csv $API/graphs/$GRAPH/data-sources | jq -r .id)
JOB=$(curl -s -X POST $API/graphs/$GRAPH/data-sources/$SOURCE/analyze | jq -r .job_id)
sleep 1 && curl -s $API/jobs/$JOB | jq .status
JOB=$(curl -s -X POST $API/graphs/$GRAPH/data-sources/$SOURCE/ingestions \
  -H "Content-Type: application/json" -H "Idempotency-Key: $(uuidgen)" \
  -d @examples/fraud/devices.mapping.json | jq -r .id)
sleep 1 && curl -s $API/jobs/$JOB | jq '{status, result}'
# → {"records_processed": 4, "entities_created": 3, "entities_matched": 3, "relationships_created": 4}

# 7. Optional (M9): a second customer source, KYC, with near-duplicates of the
#    first. Its own ids (K-1001, …) create new Customer entities.
SOURCE=$(curl -s -F file=@examples/fraud/customers_kyc.csv $API/graphs/$GRAPH/data-sources | jq -r .id)
JOB=$(curl -s -X POST $API/graphs/$GRAPH/data-sources/$SOURCE/analyze | jq -r .job_id)
sleep 1 && curl -s $API/jobs/$JOB | jq .status
JOB=$(curl -s -X POST $API/graphs/$GRAPH/data-sources/$SOURCE/ingestions \
  -H "Content-Type: application/json" -H "Idempotency-Key: $(uuidgen)" \
  -d @examples/fraud/customers_kyc.mapping.json | jq -r .id)
sleep 1 && curl -s $API/jobs/$JOB | jq '{status, result}'
# → {"records_processed": 8, "entities_created": 8, "entities_matched": 0, "relationships_created": 0}
```

Then open `http://localhost:3000/graphs/$GRAPH/explore`: select Alice Tan,
expand her neighbourhood (her accounts through `OWNS`, her devices through
`USES`), and find the path to Budi Santoso, two hops through the shared
`DEVICE-921`. The same reads are available over the API:

```bash
ALICE=$(curl -s -X POST $API/graphs/$GRAPH/entities/search -H "Content-Type: application/json" \
  -d '{"entity_type": "Customer", "property_filters": {"customer_id": "C001"}}' | jq -r '.data[0].id')
BUDI=$(curl -s -X POST $API/graphs/$GRAPH/entities/search -H "Content-Type: application/json" \
  -d '{"entity_type": "Customer", "property_filters": {"customer_id": "C002"}}' | jq -r '.data[0].id')
curl -s "$API/graphs/$GRAPH/entities/$ALICE/neighbors?direction=outgoing&limit=100" \
  | jq '[.relationships[].relationship_type]'                   # → ["OWNS", "OWNS", "USES", "USES"]
curl -s -X POST $API/graphs/$GRAPH/paths -H "Content-Type: application/json" \
  -d "{\"source_id\": \"$ALICE\", \"target_id\": \"$BUDI\", \"max_depth\": 3}" \
  | jq '{found, hops, via: [.relationships[].relationship_type]}' # → 2 hops via USES, USES
# 8. Ask MindGraph (needs a configured model): evidence first, then a cited answer.
curl -s -X POST $API/graphs/$GRAPH/ask -H "Content-Type: application/json" \
  -d "{\"question\": \"Who else shares this device?\",
       \"scope\": {\"mode\": \"selected_entities\", \"entity_ids\": [\"$ALICE\"]}}" \
  | jq '{status, answer, evidence: [.evidence.entities[] | .properties.customer_id // .properties.device_id]}'
# → "status": "answered", evidence ["DEVICE-921", "C001", "C002"]
# 9. A follow-up: the earlier question and the entities it cited (kept by the client).
curl -s -X POST $API/graphs/$GRAPH/ask -H "Content-Type: application/json" \
  -d "{\"question\": \"Is there a path between them?\",
       \"history\": [{\"question\": \"Who else shares this device?\",
                     \"entity_ids\": [\"$ALICE\", \"$BUDI\"]}]}" \
  | jq '{status, plan: .retrieval.plan.operation, turns: .metadata.conversation_turns}'
# → "status": "answered", plan "find_path", turns 1
```

Submitting the same `Idempotency-Key` again returns the same job instead of
ingesting twice. After the first successful ingestion the graph's status
changes from `draft` to `active`.

The full API is described in [`docs/api/openapi.yaml`](../api/openapi.yaml)
and [`docs/api/README.md`](../api/README.md).
