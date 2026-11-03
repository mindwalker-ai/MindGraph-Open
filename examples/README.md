# Examples

Demo datasets and walkthroughs, delivered in M10:

| Directory | Scenario | Entities |
|---|---|---|
| [`fraud/`](fraud/) | Fraud investigation (launch hero demo) | Customer, Account, Transaction, Merchant, Device, IP Address |
| [`knowledge-graph/`](knowledge-graph/) | Company knowledge graph | Person, Company, Topic, Document, Project |
| [`cyber/`](cyber/) | Cyber investigation | User, Endpoint, IP, Domain, Process, Malware |

`fraud/` already holds a deliberately tiny M4 dataset for verifying
deterministic ingestion by hand (see the README quick start):
`customers.csv`, `accounts.csv`, `ontology.json` and one mapping per file.
All values are synthetic. The full launch demo dataset arrives in M10.

Examples are proposed to be licensed Apache-2.0 (ADR-015, pending legal approval).
