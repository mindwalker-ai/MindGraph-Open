# Changelog

All notable changes to MindGraph-OSS are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/) (pre-releases until 1.0).

## [0.1.0-alpha.1] - Unreleased

The first public alpha. It bundles milestones M0 to M10. Release notes:
[docs/release/v0.1.0-alpha.1.md](docs/release/v0.1.0-alpha.1.md).

### Added

- **Local stack** (M0): Docker Compose with the web app, Rust Core, Python AI
  service, PostgreSQL 17 and Apache HugeGraph 1.7; only web and Core
  published, on 127.0.0.1.
- **Domain and storage** (M1–M3): typed UUIDv7 ids, versioned ontologies,
  validated entities and relationships; the `GraphStore` port with in-memory
  and HugeGraph adapters sharing one contract suite; PostgreSQL metadata with
  migrations at startup; graph project and ontology APIs.
- **Deterministic ingestion** (M4): CSV and JSON upload, schema analysis,
  mappings with `create_or_match` / `match_only`, idempotent ingestion jobs
  with progress over SSE.
- **AI Graph Builder** (M5): model-suggested ontology and mappings across
  several files, validated by Rust, reviewed on a canvas, applied by the M4
  engine; manual path without AI.
- **Graph Explorer** (M6): exact search, neighbourhood expansion, shortest
  paths, layouts and filters on a WebGL canvas.
- **Ask MindGraph** (M7): graph-first GraphRAG with an evidence graph,
  checked citations, honest partial and insufficient-evidence answers and
  session follow-ups.
- **MCP server** (M8): read-only tools over one graph for Claude Code, Codex,
  Cursor and other MCP clients, over stdio and Core's REST API.
- **Entity resolution** (M9): explicit matching rules, bounded scans, side by
  side review and logical identities without merging graph data.
- **Demo scenarios** (M10): fraud investigation (extended with transactions
  and merchants), cyber investigation and company knowledge graph, each with
  a README and a `demo.json` manifest; `mindgraph demo <scenario>` loads one
  through the public REST API, verifies it and prints the Explorer link;
  `make demo-fraud`, `make demo-cyber`, `make demo-knowledge` run it in an
  opt-in Compose profile; re-running reuses the existing demo graph.
- **Demo scenarios page** (`/demos`) showing which demos are loaded, with
  links into the Explorer and the command for the others.
- **Full-stack smoke test**: `scripts/smoke.sh` (running stack, read-only)
  and `scripts/smoke-stack.sh` (isolated Compose project: build, demo, smoke,
  idempotent re-run, restart persistence, browser journeys, cleanup of its
  own volumes only).
- **Browser end-to-end tests** with Playwright against the real stack:
  first run, Graph Explorer, entity resolution and the manual flow without
  AI.
- **Release checks**: `make audit` (cargo-deny, npm audit, pip-audit),
  `make check-openapi` (OpenAPI 3.1 validation and router comparison), MCP
  protocol tests against the release binary; CI jobs for all of them and a
  release gate. Manual workflows for container images and MCP/CLI binaries
  (nothing is published automatically).
- **Docs**: new README, getting started, tutorials, development guide, data
  and backup guide (tested backup and restore), benchmarks, security review,
  dependency and license audits, known limitations, release checklist.
- **Governance**: CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, SUPPORT, issue and
  pull request templates.
- Web status indicator backed by Core's readiness (`/api/ready`).

### Changed

- `GET /ready` checks HugeGraph as well as PostgreSQL, with a 3-second bound
  per check; the Compose health check of Core uses it.
- HugeGraph runs with a fixed heap (`MINDGRAPH_HUGEGRAPH_HEAP`, default 1g)
  instead of half of the free memory at start.
- The Core image also contains the `mindgraph` CLI.
- Navigation shows only destinations that exist in v0.1; placeholder entries
  ("Coming soon") were removed. "Explore Demo" and the demo list lead to the
  demo scenarios; graph cards show ontology type counts instead of empty
  placeholders.
- Version set to `0.1.0-alpha.1` across the workspace, web app, AI service
  and OpenAPI document.

### Fixed

- Path search on HugeGraph could exhaust the server's memory
  (`OutOfMemoryError` in HugeGraph's `shortestpath` traverser, even with
  2 GiB); the adapter now runs the reference breadth-first search, bounded to
  2,000 expanded entities.
- Pasting an entity id into the Path Finder and pressing "Use" reloaded the
  page (a form nested inside another form); Enter in the field now works too.

### Security

- Web responses carry `X-Content-Type-Options`, `Referrer-Policy`,
  `X-Frame-Options` and `frame-ancestors 'none'`; `X-Powered-By` removed.
- GitHub Actions pinned to commit SHAs; workflows default to read-only
  permissions.
- See [docs/release/security-review.md](docs/release/security-review.md).

[0.1.0-alpha.1]: https://github.com/mindwalker-ai/MindGraph-Open/compare/84b711d...HEAD
