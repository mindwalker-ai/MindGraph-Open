# Examples

Demo datasets and walkthroughs, delivered in M10:

| Directory | Scenario | Entities |
|---|---|---|
| [`fraud/`](fraud/) | Fraud investigation (launch hero demo) | Customer, Account, Transaction, Merchant, Device, IP Address |
| [`knowledge-graph/`](knowledge-graph/) | Company knowledge graph | Person, Company, Topic, Document, Project |
| [`cyber/`](cyber/) | Cyber investigation | User, Endpoint, IP, Domain, Process, Malware |

`fraud/` already holds a deliberately tiny dataset for verifying
deterministic ingestion by hand (see the README quick start):

- `customers.csv`, `accounts.csv`, `devices.csv`, `ontology.json` and one
  mapping per file (M4).
- `customers_kyc.csv` with `customers_kyc.mapping.json` (M9). It is a second
  customer source with deliberate near-duplicates for
  [entity resolution](../docs/entity-resolution/README.md):
  - name variants and letter-case differences;
  - local and international phone formats;
  - a common name that belongs to a different person;
  - an invalid phone number.

All values are synthetic. The full launch demo dataset arrives in M10.

Examples are proposed to be licensed Apache-2.0 (ADR-015, pending legal approval).
