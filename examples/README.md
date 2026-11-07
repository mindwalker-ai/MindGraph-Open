# Examples

Three synthetic demo scenarios. Each folder holds CSV files, an
`ontology.json`, one `*.mapping.json` per file (the M4 mapping format) and a
`demo.json` manifest that the demo loader reads.

| Scenario | Load it | Entity types | Walkthrough |
|---|---|---|---|
| Fraud investigation | `make demo-fraud` | Customer, Account, Device, Transaction, Merchant | [fraud/README.md](fraud/README.md) |
| Cyber investigation | `make demo-cyber` | User, Endpoint, Process, IPAddress, Domain | [cyber/README.md](cyber/README.md) |
| Company knowledge graph | `make demo-knowledge` | Person, Company, Project, Document, Topic | [knowledge-graph/README.md](knowledge-graph/README.md) |

`make demo-*` starts the Docker Compose stack if needed and runs the
one-shot `demo` service (`mindgraph demo <scenario>`). The loader:

- uses only Core's public REST API (create graph, ontology, upload, analyse,
  ingest), exactly like a user;
- verifies entity counts, relationships, exact searches, neighbours and
  shortest paths, and prints the Graph ID and an Explorer link;
- waits with bounded timeouts and exits non-zero with the API error when a
  step fails;
- reuses the graph with the demo's exact name when it exists (verifying it
  instead of loading again), and creates a timestamped copy only with
  `--new`. It never changes other graphs or deletes data.

```bash
docker compose --profile demo run --rm demo --list          # scenarios
docker compose --profile demo run --rm demo fraud --new     # a fresh copy
cargo run -p mindgraph-cli -- demo cyber --api http://127.0.0.1:8080   # without Docker
```

`fraud/` also keeps the original M4 walkthrough files (customers, accounts,
devices) used by the README's "load a dataset without AI" steps and the test
suites, and `customers_kyc.csv` for entity resolution.

All values are synthetic. Domains and IP addresses in `cyber/` come from the
reserved documentation ranges (RFC 2606, RFC 5737). The examples are proposed
to be licensed Apache-2.0 (ADR-015); that split is pending legal approval
(see [docs/legal/LICENSE_AUDIT.md](../docs/legal/LICENSE_AUDIT.md)).
