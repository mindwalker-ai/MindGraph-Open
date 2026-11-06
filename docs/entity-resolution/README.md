# Entity resolution

MindGraph can find **potential duplicate** entities: two records of the same
entity type that may describe the same real-world person, company or object.
For example, a customer imported once from core banking and again from a KYC
system. A person reviews every candidate.

Approving a candidate creates a **logical identity mapping**. The original
entities and their relationships stay exactly as they are in the graph.

This is a deterministic, explainable foundation (M9, [ADR-009](../adr/MindGraph%20Open%20Architecture%20Decision%20Records.md#adr-009-entity-resolution-v01)).
It is not an automated identity-resolution engine. It does not use machine
learning, LLMs, embeddings or automatic merging.

```text
GraphStore ──bounded search──► normalize ─► block ─► compare ─► score ─► candidates (PostgreSQL)
(one entity type, ≤ 999)                                                       │
                                                                     human review │
                                         ┌────────────────────────────────────────┤
                                         ▼                                        ▼
                              Same entity: logical identity group     Different entities: rejected
                              (metadata + provenance; graph unchanged)  (kept as a decision)
```

## Try it with the fraud demo

1. Load the fraud demo (README, "Load a dataset without AI"). Include
   `examples/fraud/customers_kyc.csv` with `customers_kyc.mapping.json`.
   That file is a second source of customers with deliberate near-duplicates:
   - name variants (`M. Ikhwan Nasution` / `Muhammad Ikhwan Nasution`,
     `ALICE  TAN`);
   - emails that differ in letter case;
   - local and international phone formats;
   - a common name (`Dewi Kusuma`) that belongs to a different person;
   - an invalid phone number.
2. Open the graph and choose **Resolution** in the sidebar
   (`/graphs/{graphId}/resolution`).
3. Keep **Customer** and the suggested rules: `email`, `phone` with default
   country `ID`, and `name` as a fuzzy name. Then click **Run Scan**.
4. Pick a candidate. Both original records appear side by side, with the
   evidence for every compared property.
5. Choose **Same Entity**, pick which record stays canonical and confirm. Or
   choose **Different Entities**.
6. In the Graph Explorer, an entity's details now show **Resolved identity**
   with its canonical record and the linked records. Pending candidates show
   as **Potential duplicate**.

The same through the API:

```bash
GRAPH=<graph id>
curl -s -X POST localhost:8080/api/v1/graphs/$GRAPH/resolution/scans \
  -H 'Content-Type: application/json' -d '{
    "entity_type": "Customer",
    "max_entities": 400,
    "rules": [
      {"property": "email", "kind": "normalized_email"},
      {"property": "phone", "kind": "normalized_phone", "country": "ID"},
      {"property": "name",  "kind": "fuzzy_name", "threshold": 0.9}
    ]}'
curl -s "localhost:8080/api/v1/graphs/$GRAPH/resolution/candidates?status=pending&limit=20"
curl -s localhost:8080/api/v1/graphs/$GRAPH/resolution/candidates/<candidate id>
curl -s -X POST localhost:8080/api/v1/graphs/$GRAPH/resolution/candidates/<candidate id>/decisions \
  -H 'Content-Type: application/json' \
  -d '{"decision": "same_entity", "canonical_entity_id": "<entity id>"}'
curl -s localhost:8080/api/v1/graphs/$GRAPH/resolution/entities/<entity id>
```

## Matching methods

Every rule names **one explicit property** of the scanned entity type, and
the property must be in the active ontology. No property is treated as an
identifier because of its name. Entities are compared only with entities of
the same type in the same graph, so a `Customer` is never compared with a
`Merchant`.

| Rule | Property types | Matches when | Strength |
|---|---|---|---|
| `exact_identifier` | string, integer | the trimmed values are identical (case-sensitive) | strong |
| `normalized_email` | string | the addresses are equal after normalization | strong |
| `normalized_phone` | string (integer: reported, never compared) | the E.164 numbers are equal | strong |
| `normalized_string` | string | the normalized texts are equal | moderate |
| `fuzzy_name` | string | the name similarity is at or above the threshold (default 0.90, range 0.5–1.0) | weak |

A scan accepts at most 10 rules, and each `(property, kind)` pair at most
once. Missing values never match: absent, blank or whitespace on either side
is **Missing data**. A present value that cannot be compared is
**Unavailable**, with a reason, for example an invalid email or phone
number.

## Normalization

Normalized values are comparison keys only. They are never written back to
an entity and never stored.

- **Text**: Unicode NFKC, default case folding, then NFKC again; trimmed,
  with whitespace runs collapsed. Stable and idempotent, so `ＡＬＩＣＥ  ` and
  `alice` compare equal, and so do `STRASSE` and `straße`.
- **Names**: normalized text in which punctuation (`.`, `,`, `-`) separates
  tokens and apostrophes are removed, so `M. Ikhwan` gives `m`, `ikhwan`.
  Letters and combining marks of every script are kept.
- **Email**:
  - trimmed, and the address must have a basic `local@domain.tld` shape;
  - only the domain is lower-cased, because the local part may be
    case-sensitive (RFC 5321);
  - no provider alias rules: Gmail dots and `+tags` stay significant.

  Addresses that differ only in the case of their local part
  (`IKHWAN@example.com` and `ikhwan@example.com`) count as a weaker match.
  The evidence says so. Turn this off per rule with
  `"case_insensitive_local_part": false`.
- **Phone**: parsed with libphonenumber metadata (the
  [`phonenumber`](https://crates.io/crates/phonenumber) crate,
  Apache-2.0) and compared in E.164 form.
  - With `"country": "ID"`, `0812-1234-5678` and `+62 812 1234 5678` are
    the same number.
  - Without a country, a national number is **unavailable** and never
    guessed.
  - Numbers the metadata considers invalid never match.
  - Phones stored as integers are reported and not compared, because
    leading zeros may already be lost and MindGraph does not reconstruct
    digits.

## Fuzzy names

The similarity is a **token-aware Jaro-Winkler** score:

- Tokens are paired one-to-one by similarity, and the sum is divided by the
  token count of the longer name. A missing middle name therefore lowers
  the score.
- Typos within a token are tolerated (`Santoso` / `Santosa`), and token
  order does not matter.
- An initial pairs with a token that starts with the same letter at a fixed
  0.85. Such matches are flagged **relies on an initial**, because initials
  are ambiguous.
- Whole-name Jaro-Winkler is also considered when the token counts differ,
  so `AliceTan` and `Alice Tan` still compare. It is deliberately ignored
  for names with the same token count: otherwise a long shared surname
  would make `Dewi Santoso` and `Dian Santoso` (often relatives) look like
  one person.

Similar names are **weak evidence**. Common names, initials and family
members produce similar names for different people. A name match alone can
create a candidate, never an identity.

## Candidate score

Each agreeing signal has a fixed weight:

| Signal | Weight |
|---|---|
| `exact_identifier` | 0.95 |
| `normalized_email` | 0.90 (0.75 when only the local part's case differs) |
| `normalized_phone` | 0.85 |
| `normalized_string` | 0.60 |
| `fuzzy_name` | 0.60 × similarity |

```text
score = 1 − Π (1 − weight)   over agreeing signals, rounded to four decimals
```

For example:

- A case-only email match, a phone match and a name match through an
  initial (similarity 0.95) score `1 − 0.25 × 0.15 × 0.43 ≈ 0.98`.
- A shared common name alone scores 0.60.

Disagreeing, missing and unavailable signals do not lower the score. They
are shown as evidence for the reviewer instead.

**The score is not a probability.** It is a deterministic ranking heuristic
that puts the most promising candidates first. `0.94` does not mean "94 %
likely to be the same person", and the UI never shows a percentage. The
threshold of a fuzzy rule is a heuristic cut-off, not a calibrated
probability either.

## Scans and their limits

A scan is synchronous and bounded. It reports every limit it hits, so a
partial scan is never presented as complete.

| Bound | Value | Reported as |
|---|---|---|
| Entities examined | the first `max_entities` (2–999, default 400) of the type, in id (creation) order | `truncated: true` when more exist |
| Comparisons | at most 20,000 distinct pairs | `comparison_budget_reached` |
| Block size | blocks of more than 50 entities are skipped | `oversized_blocks` and a warning |
| Candidates kept | the 2,000 highest scores | `candidate_limit_reached` |
| Deadline | 30 s, waiting for a slot included; 2 scans at a time | `504 RESOLUTION_SCAN_TIMEOUT` |

`exhaustive` is `true` only when none of these happened. Data-quality
findings are counted per rule, for example "3 national phone numbers need a
default country". They never include values.

**Blocking.** Only pairs that share a block are compared. A block is an
identifier value, a lower-cased email, an E.164 phone, a normalized text, or
the first three letters of a name token. Pairs are discovered strongest rule
first, so when the budget runs out, weak fuzzy-only pairs are dropped first.
Every discovered pair is compared with all rules, so its evidence is
complete. Two names that share no token prefix (a typo in the first letters
of every token) are never compared.

**Rescans.** A new scan skips pairs that are already pending, were rejected
earlier, or are already linked, and counts them under `skipped`. There is
at most one pending candidate per pair.

**Why bounded.** The GraphStore search returns at most 1,000 entities and
has no cursor-based enumeration. M9 therefore covers a selected subset and
says so. Graph-wide resolution needs an indexed enumeration API and an
asynchronous resolution-run model. Both are future work.

### Measured performance

`cargo test -p mindgraph-entity-resolution --release --test scan_perf -- --nocapture`
uses synthetic customers with mixed duplicates: case-only emails, national
phones, initials and name typos.

| Entities | Pairs compared | Candidates | Injected duplicates found | Matching time |
|---|---|---|---|---|
| 400 | 4,667 of 79,800 possible | 144 | 100 of 100 | 58 ms |
| 999 | 20,000 (budget reached) | 316 | 193 of 199 | 66 ms |

These were measured on an Apple Silicon laptop in a release build. They
cover matching only, not the GraphStore read or the database write. The
candidates kept used about 50 KiB and 110 KiB. They are measurements, not a
service-level objective.

## Human review

| Decision | Request | Effect |
|---|---|---|
| Same entity | `{"decision": "same_entity", "canonical_entity_id": "<id>"}` | the pair joins one identity group, with the canonical entity the reviewer chose |
| Different entities | `{"decision": "different_entities"}` | the candidate is rejected; no mapping |
| Reconsider | `{"decision": "same_entity", "canonical_entity_id": "<id>", "reconsider": true}` | a rejected candidate is accepted after all; both decisions stay in the history |

- No candidate is ever approved automatically, not even an exact identifier
  match.
- The canonical entity is the reviewer's choice: any member of the
  resulting group. There is no "oldest id wins" policy.
- Repeating the same decision returns the earlier one (`200`,
  `"replayed": true`).
- Changing a decision is `409 DECISION_CONFLICT`. Accepted mappings cannot
  be undone in v0.1.
- Linking two entities that an earlier review marked as different is
  `409 IDENTITY_CONFLICT`. Reconsider that candidate first.
- Rejecting two entities that are already linked is also
  `409 IDENTITY_CONFLICT`, because groups cannot be split in v0.1.
- Decisions of one graph are serialised, so concurrent reviewers cannot
  produce conflicting canonical mappings. The loser of a race gets a `409`,
  and the UI reloads the candidate.

## Logical versus physical merge

M9 never merges graph entities. The GraphStore cannot update an entity,
rewire its relationships and delete a duplicate as one atomic operation.
HugeGraph writes are not coordinated with PostgreSQL either. A "merge"
assembled from separate writes could leave the graph half-rewritten.
Instead:

- Both original entities stay, with all their properties and relationships.
- No `SAME_AS` relationship is created and no property is overwritten.
- The mapping lives in PostgreSQL.
- The Graph Explorer, Ask MindGraph (GraphRAG) and MCP keep returning the
  real entities and edges. They do not collapse identity groups, and
  shortest paths do not treat linked records as one node. The resolution
  API is the only place that reports canonical identity.
- Identity responses always say `"physical_graph_merged": false`.

## Canonical identity

Groups are **flat**: every member maps directly to the canonical entity,
which maps to itself. Resolving any member is one lookup, and chains or
cycles cannot exist.

```text
E2 → E1,  E4 → E3,  approve (E2, E4) choosing E3
   ⇒  E1 → E3, E2 → E3, E4 → E3, E3 → E3        (transitive consolidation)
```

A group holds one entity type and one graph, with at most 100 members.
PostgreSQL enforces flatness and single typing with a deferred constraint
trigger, even against writes that bypass MindGraph.

`GET /api/v1/graphs/{graph}/resolution/entities/{entity}` returns:

```json
{
  "entity_id": "019a6f3e-…",
  "canonical_entity_id": "019a6f3d-…",
  "member_entity_ids": ["019a6f3d-…", "019a6f3e-…"],
  "is_canonical": false,
  "resolved": true,
  "resolution_type": "manual",
  "physical_graph_merged": false
}
```

An entity in no group is its own canonical entity (`resolved: false`).

## Provenance

Every decision is immutable. It records:

- the candidate's original entity ids;
- the canonical entity, the canonical entities of the groups it
  consolidated, and the members whose mapping it wrote;
- the decision, score and agreeing rules (`normalized_email:email`, …);
- the scan id, which holds the full rule snapshot and the ontology version
  used;
- the timestamp and the actor (`local-user` in the local version);
- whether it reconsidered an earlier rejection.

Each group member also records the decision that last wrote its mapping.

**Limitation:** M4 ingestion does not keep source-record references on
entities, so provenance does not name source files or rows. MindGraph does
not invent them.

## Data privacy

Matching data is often personal data.

- PostgreSQL stores entity ids, scores, rule snapshots and evidence that
  names properties, rules and outcomes. No names, emails, phone numbers or
  other values are stored.
- The review UI reads the original properties from the GraphStore when it
  shows a candidate.
- Logs carry graph, scan and candidate ids, the entity type, rule and
  candidate counts, durations and truncation. They never carry values.
- Matching runs inside MindGraph Core. No data goes to an AI service or a
  model.

## Relation to ingestion, GraphRAG and MCP

- **Ingestion (M4)** still matches records by the exact identity fields of
  the mapping (`create_or_match`, `match_only`). That decides which rows
  become one entity at write time. M9 works on entities that already exist,
  for duplicates the mapping could not see, such as records with different
  source ids. It never changes ingestion, and ingestion never reads M9
  mappings. Using canonical identity during ingestion needs its own design
  review.
- **GraphRAG (M7) and MCP (M8)** are unchanged: their answers and tools see
  separate records. A resolution-aware traversal, which would treat a group
  as one node with clear provenance, is a future design.

## Future physical consolidation

A later version may offer physical consolidation: one entity with merged
properties and rewired relationships. It needs:

- an atomic or compensating GraphStore operation;
- property reconciliation rules;
- undo or split support;
- graph-wide indexed scans.

The decisions and provenance recorded by M9 are designed to feed that work.
None of it is part of v0.1.

## Tests

- `mindgraph-entity-resolution`:
  - normalization, similarity, rules, scoring, blocking and identity
    planning;
  - scan-level matching tests;
  - the `scan_perf` sanity check.
- `mindgraph-core`: `EntityResolutionService` over memory stores. A write
  spy and before/after snapshots prove that approval leaves the GraphStore
  untouched.
- `mindgraph-metadata`:
  - the shared `EntityResolutionStore` contract, against memory and
    PostgreSQL (`make test-postgres`);
  - database-level immutability and flatness.
- `mindgraph-api`:
  - every endpoint and error code (`resolution_api`);
  - the full flow against HugeGraph 1.7 (`hugegraph_resolution`, part of
    `make test-hugegraph`).
- `apps/web`: rule suggestions, the review workspace and the Graph
  Explorer identity card.
