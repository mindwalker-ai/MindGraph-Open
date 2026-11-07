# License audit (v0.1.0-alpha.1)

**Status: NOT LEGALLY APPROVED.** This is an engineering inventory prepared
for counsel. It records what the repository declares and what its
dependencies declare. It is not legal advice and makes no claim that the
licensing is compliant. Release remains blocked on the open questions at the
end until counsel answers them.

Prepared 2026-11-07 from the lockfiles on branch `feat/m10-launch-hardening`.

## 1. What MindGraph Open declares

| Part | Declared license | Where declared |
|---|---|---|
| Server: Rust workspace crates except `mindgraph-mcp` (`mindgraph-api`, `-core`, `-domain`, `-graph`, `-metadata`, `-ingestion`, `-ontology`, `-ai-client`, `-graphrag`, `-entity-resolution`, `-cli`) | AGPL-3.0-only | workspace `Cargo.toml` (`license = "AGPL-3.0-only"`), root `LICENSE` |
| `crates/mindgraph-mcp` (MCP server) | Apache-2.0 (proposed) | `crates/mindgraph-mcp/Cargo.toml` |
| `services/ai` (Python AI service) | AGPL-3.0-only | `services/ai/pyproject.toml` |
| `apps/web` (Next.js app) | none declared | `apps/web/package.json` has `"private": true` and no `license` field; the root `LICENSE` (AGPL-3.0) applies by default |
| `examples/` datasets | Apache-2.0 (proposed in ADR-015) | `examples/README.md` only; no license file in `examples/` |
| Documentation | none declared | root `LICENSE` applies by default |
| Logo and mark (`assets/logo.png`, `apps/web/public/logo.png`, `mark.png`) | none declared | ownership and trademark status are not recorded in the repository |

ADR-015 proposes: server AGPLv3, SDKs Apache-2.0, MCP packages Apache-2.0,
examples Apache-2.0, enterprise modules commercial. It states: "Final legal
approval is required before release." That approval has not been recorded.

There are no SPDX headers in source files and no per-package license files
(`crates/mindgraph-mcp/LICENSE`, `examples/LICENSE`). Adding them would apply
the proposed split, so they wait for counsel.

## 2. Third-party dependencies

### Rust (Cargo.lock, all targets)

Checked with `cargo deny check licenses` (cargo-deny 0.20.2, config
`deny.toml`): **licenses ok**. Every third-party crate offers at least one
license from the allow list. Counts are crates offering that license (a
dual-licensed crate counts under each):

| License | Crates | Notes |
|---|---|---|
| MIT | 237 | |
| Apache-2.0 | 185 | |
| Unicode-3.0 | 21 | ICU data crates (via `url`/`idna`) |
| Unlicense | 5 | `aho-corasick`, `memchr`, `csv`, `csv-core`, `byteorder`; all also MIT |
| Zlib | 3 | `foldhash`, `miniz_oxide`, `tinyvec` |
| BSD-3-Clause | 2 | `encoding_rs`, `matchit` |
| BSL-1.0 | 2 | `ryu` (also Apache-2.0), `whoami` |
| 0BSD | 1 | `adler2` |
| Apache-2.0 WITH LLVM-exception | 1 | `wasi` (wasm targets only) |
| LGPL-2.1-or-later | 1 | `r-efi`: offered as MIT OR Apache-2.0 OR LGPL-2.1-or-later; UEFI targets only, not compiled for supported platforms |

No third-party crate is GPL- or AGPL-only. The normal (non-dev) dependency
tree of `mindgraph-mcp` contains no MindGraph crate and only permissive
licenses (enforced by `make check-mcp-boundary`).

### Web app (apps/web/package-lock.json)

Production dependency closure (`npm ls --omit=dev --all`): 39 packages on
macOS arm64. The Docker image installs the Linux variants of the native
packages instead (`@img/sharp-linux-*`, `@next/swc-linux-*`), with the same
licenses.

| License | Packages | Notes |
|---|---|---|
| MIT | 23 | next, react, react-dom, sigma, graphology, clsx, tailwind-merge, … |
| Apache-2.0 | 8 | sharp, @swc/helpers, detect-libc, and `@playwright/test`/`playwright`/`playwright-core` (listed because Next.js declares Playwright as an optional peer; it is a test tool and not part of the production image) |
| ISC | 3 | lucide-react (icons), picocolors, semver |
| LGPL-3.0-or-later | 1 | `@img/sharp-libvips-*`: the libvips native library used by `sharp` (Next.js image optimisation), shipped as a separate shared library |
| Apache-2.0 AND LGPL-3.0-or-later AND MIT | 1 | `@img/sharp-wasm32` (WebAssembly fallback, not used on supported platforms) |
| CC-BY-4.0 | 1 | `caniuse-lite` (browser support data, used at build time) |
| BSD-3-Clause | 1 | source-map-js |
| 0BSD | 1 | tslib |

Fonts: the web app loads **Titillium Web** and **IBM Plex Mono** through
`next/font/google`, which downloads them at build time and serves them from
the app. Both are published under the SIL Open Font License 1.1. The OFL
text is not currently shipped with the app.

### AI service (services/ai/uv.lock)

Runtime closure (`uv export --no-dev`): 24 packages. MIT (12, including
fastapi, pydantic, anyio, h11, pyyaml, watchfiles), BSD-3-Clause (8,
including starlette, uvicorn, click, httpx2), Apache-2.0
(opentelemetry-api), PSF-2.0 (typing-extensions), MIT OR Apache-2.0
(uvloop). `httpx2-jsfetch` (a browser-only optional transport) is pinned in
the lockfile but not installed in the service environment; its license was
not checked.

### Services run as separate containers (not linked)

| Component | License | How it is used |
|---|---|---|
| Apache HugeGraph 1.7.0 (`hugegraph/hugegraph:1.7.0`) | Apache-2.0 (the image also contains an OpenJDK runtime, GPL-2.0 with Classpath Exception) | separate process, called over HTTP |
| PostgreSQL 17 (`postgres:17-alpine`) | PostgreSQL License | separate process, SQL over TCP |
| Base images `debian:bookworm-slim`, `node:22-bookworm-slim`, `python:3.13-slim-bookworm`, `rust:1.99-slim-bookworm` (build only) | various, including GPL system packages | operating system layers of the locally built images |

MindGraph does not publish container images in this release. Users build
them locally with `docker compose up --build`.

## 3. Open questions for counsel (release blockers)

1. **Approve or change the ADR-015 split**: AGPL-3.0-only for the server and
   AI service, Apache-2.0 for `mindgraph-mcp` and `examples/`.
2. **Web app license**: `apps/web` declares none. Confirm it is AGPL-3.0 like
   the server, or declare otherwise.
3. **MCP boundary** (from `docs/mcp/README.md`): the Apache-2.0
   `mindgraph-mcp` has AGPL dev-dependencies for its end-to-end tests, and
   the AGPL `mindgraph` CLI links `mindgraph-mcp`. Confirm both are
   acceptable.
4. **Per-package license files and SPDX headers**: add after approval
   (`crates/mindgraph-mcp/LICENSE`, `examples/LICENSE`, headers or a
   `REUSE.toml`).
5. **Third-party notices**: decide whether the release must ship a NOTICE
   file and the OFL text for the bundled fonts, and how to handle the
   LGPL-3.0 libvips library shipped with `sharp` (or disable Next.js image
   optimisation so it is not needed).
6. **Logo and name**: record ownership and trademark terms for the
   MindGraph name and logo, and whether forks may use them.
7. **Contributions**: no CLA or DCO is imposed in this release (see
   `CONTRIBUTING.md`). Decide whether one is needed before accepting outside
   contributions, especially if a commercial edition will reuse code.

## 4. How to reproduce

```bash
cargo deny check licenses                          # Rust, against deny.toml
cargo deny list                                    # Rust inventory by license
cd apps/web && npm ls --omit=dev --all             # web production closure
cd services/ai && uv export --no-dev --no-hashes   # Python runtime closure
```
