# Release checklist: v0.1.0-alpha.1

Status per item: **PASS** (verified, evidence given), **FAIL**, or
**NOT VERIFIED** (not run, or not run in the stated environment). Verified on
2026-11-07 on branch `feat/m10-launch-hardening` (macOS 15 on Apple M4,
Podman 6.1 with `podman compose` 1.6). Nothing has been tagged or published.

## Blockers (must be resolved before release)

| Item | Status | Notes |
|---|---|---|
| Legal approval of the licensing split (ADR-015) | **NOT VERIFIED** | Open questions in [LICENSE_AUDIT.md](../legal/LICENSE_AUDIT.md). Not approved. |
| Per-package license files, SPDX headers, third-party notices (fonts, libvips) | **NOT VERIFIED** | Waits for the legal decision. |
| Private contact for security and conduct reports | **NOT VERIFIED** | SECURITY.md relies on GitHub private vulnerability reporting; a maintainer must enable it on the repository. |
| CI green on GitHub (new jobs: full-stack, audit, openapi, mcp-release, release gate) | **NOT VERIFIED** | The workflow is updated but has not run on GitHub; nothing was pushed. |
| Clean install with Docker Engine / Docker Desktop | **NOT VERIFIED** | Verified with Podman only; the CI full-stack job runs it with Docker on Linux. |

## Build and code quality

| Item | Status | Evidence |
|---|---|---|
| `make check` (fmt, clippy `-D warnings`, tests, boundaries, web, AI) | PASS | Rust 566 passed / 0 failed / 113 ignored (live-only); web 132/132; AI 70/70; ESLint, Prettier, Ruff clean; 4 boundary checks OK |
| `cargo build --workspace --locked` | PASS | |
| `cargo build -p mindgraph-mcp --release --locked` | PASS | 6.8 MB binary; `mindgraph-mcp --help`, `mindgraph --version` → `0.1.0-alpha.1` |
| MCP protocol tests against the release binary | PASS | 7/7 (`--release --test mcp_protocol`) |
| `make test-postgres` (PostgreSQL 17) | PASS | 60/60 |
| `make test-hugegraph` (HugeGraph 1.7, 1 GiB heap) | PASS | contract 47/47, ingestion 1/1, resolution 1/1, MCP on HugeGraph 1/1 |
| GraphStore contract on HugeGraph with a 512 MiB heap | PASS | 47/47 (path search no longer uses HugeGraph's traverser) |
| `make check-openapi` | PASS | valid OpenAPI 3.1.0; all 30 router operations documented, nothing extra; negative check catches both directions |
| `make audit` | PASS | cargo-deny: advisories, bans, licenses, sources ok (1 documented ignore); npm production 0 vulnerabilities; pip-audit none. See [dependency-audit.md](dependency-audit.md) |

## Docker and full stack

| Item | Status | Evidence |
|---|---|---|
| `docker compose config` / build / up (Podman) | PASS | `scripts/smoke-stack.sh`: images build, 5 services start, Core healthy only when PostgreSQL and HugeGraph answer |
| Clean install needs no host Rust, Node, Python or jq | PASS (Podman) | build and run happen in containers; `scripts/smoke.sh` needs only bash, curl, grep and sed |
| Non-root containers, internal-only services, no baked secrets | PASS | core/ai uid 10001, web uid 1000; ai, postgres, hugegraph unpublished; see [security-review.md](security-review.md) |
| Readiness vs liveness | PASS | `/health` liveness; `/ready` 503 within 3 s when HugeGraph is paused, 200 again when resumed |
| Persistence across `down` + `up -d` (no `-v`) | PASS | smoke-stack restart step: demo graph and 21 smoke checks intact |
| Backup and restore | PASS | backup, `down -v`, restore: 9 graphs back, smoke 21/21, demos verified ([backup-restore.md](../operations/backup-restore.md)) |
| Full-stack smoke test (`scripts/smoke.sh`) | PASS | 21/21 through the published ports (Core and web proxy) |
| Final isolated run on the final code (`smoke-stack.sh --e2e`) | PASS (Podman) | fresh image build; fraud demo via REST; smoke 21/21; second load reused, not duplicated; data intact after `down` + `up -d` (smoke 21/21 again); Playwright journeys 4/4; project `mindgraph-smoke` removed afterwards |

## Demos

| Item | Status | Evidence |
|---|---|---|
| `mindgraph demo fraud` | PASS | 13 Customer, 7 Account, 3 Device, 8 Transaction, 4 Merchant; 27 relationships; paths C001→C002 (USES·USES), C001→M004 (OWNS·SENT·TO); 7 resolution candidates |
| `mindgraph demo cyber` | PASS | 4 User, 4 Endpoint, 6 Process, 4 IPAddress, 5 Domain; 23 relationships; both 4-hop paths |
| `mindgraph demo knowledge-graph` | PASS | 6 Person, 3 Company, 3 Project, 5 Document, 4 Topic; 34 relationships; both paths |
| Idempotent re-run (reuses, never overwrites) | PASS | second run "Found the existing demo graph … verifying it"; `--new` creates a timestamped copy |
| Bounded waits, non-zero exit on failure | PASS | unreachable Core: exit 1 after `--timeout`; unknown scenario: exit 2 |
| Compose `demo` profile (`docker compose --profile demo run --rm demo <scenario>`) | PASS (Podman) | all three scenarios loaded through the profile service |
| `make demo-fraud` / `demo-cyber` / `demo-knowledge` | PASS (Podman) | `COMPOSE="podman compose"` on freshly built images, empty volumes: each starts the stack, creates its graph, finds the expected paths (27 / 23 / 34 relationships) and prints working Explorer, resolution and MCP links (explore page 200, Core `/ready` 200) |

## Web app

| Item | Status | Evidence |
|---|---|---|
| Honest first-run: Explore Demo, Demos page backed by the real loader | PASS | `/demos` reads graphs and links loaded demos; others show the command |
| Placeholder UI removed | PASS | no "Coming soon" entries, no fake menu, no empty count placeholders (unit tests assert it) |
| Browser journeys A–D (Playwright, real stack, no mocks) | PASS | 4/4 against the native stack (twice) and the Compose stack (`--e2e`) |
| WebGL in headless CI | PASS | Scenario B asserts the canvas renders with software WebGL (SwiftShader flags) |
| Security headers | PASS | nosniff, Referrer-Policy, X-Frame-Options, frame-ancestors; no X-Powered-By |
| Screenshots in `assets/screenshots/` | PASS | 9 screenshots from the Compose stack |
| Cross-browser (Firefox, Safari) | NOT VERIFIED | Chromium only |

## AI, GraphRAG and MCP

| Item | Status | Evidence |
|---|---|---|
| AI disabled: upload, analysis, manual ontology and mapping, ingestion, explorer, entity resolution, MCP | PASS | Scenario D on the Compose stack (`not_configured`); demos and `make test-mcp-live` without a provider |
| AI enabled with a fake provider: suggestion → Rust validation → reviewable draft → M4 ingestion | PASS | `graph_builder_api` (`suggestions_are_accepted_and_returned_through_the_job_api`, `one_suggestion_can_model_several_data_sources`) and `graph_builder_service` tests |
| GraphRAG with a fake provider: real evidence ids, citations limited to evidence, no fabricated links, truncation reported, unavailable without a provider | PASS | `graphrag_api` (shared-device question), `graphrag_service` (`fabricated_references_never_become_citations`, `missing_evidence_is_insufficient_without_a_model_answer`), `retrieval` (`the_evidence_budget_truncates_without_dangling_relationships`), `questions_need_a_configured_model` |
| Opt-in live model provider smoke test | NOT VERIFIED | Not run in M10 (no paid model calls); last run against a live provider in M7/M8 |
| MCP: init, discovery, schemas, structured output, errors, stdio cleanliness, EOF, one-graph scope, read-only, `ask_graph` off by default, no DB access | PASS | protocol tests on the release binary; `end_to_end` with a write spy; `check-mcp-boundary`; agent flow against the Compose stack |
| MCP client guides (Claude Code, Codex, Cursor) | PASS / NOT VERIFIED | Claude Code tested as a real client in M8; Codex and Cursor not run |

## Docs and governance

| Item | Status |
|---|---|
| README redesign, getting started, tutorials, development guide | PASS |
| Scenario READMEs (fraud, cyber, knowledge-graph) | PASS |
| Default credentials documented | PASS |
| `down -v` danger and safe cleanup documented | PASS |
| Benchmarks with measured numbers only | PASS |
| KNOWN_LIMITATIONS, CHANGELOG, release notes | PASS |
| CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, SUPPORT, issue and PR templates | PASS (contacts: see blockers) |
| GitHub Actions pinned to SHAs, read-only permissions | PASS |
| Manual GHCR image workflow and MCP/CLI binary workflow (no auto-publish) | NOT VERIFIED (never run) |
