# Alpha security review (v0.1.0-alpha.1)

A focused engineering review for the first public alpha, done on
2026-11-07. Probes ran against the Docker Compose stack of this release
(isolated project `mindgraph-smoke`, Podman 6.1) unless noted. It is not a
penetration test or an external audit.

## Summary

MindGraph-OSS v0.1 is a **local, single-user tool without authentication**.
Within that model the review found no open issue that blocks an alpha. It
found and fixed two problems (a HugeGraph memory exhaustion triggered by path
search, and missing browser hardening headers), and records the accepted
limitations below.

| Area | Result |
|---|---|
| Network exposure | only web (3000) and Core (8080) published, on 127.0.0.1 |
| Authentication | none; documented as a limitation in SECURITY.md, README and getting started |
| Upload safety | size limit, type allow-list, filename sanitising, malformed input handled; no leftovers on rejection |
| API inputs | strict JSON parsing, body size limit (413), typed ids, error envelope without internals |
| Containers | non-root users; no secrets baked into images |
| Secrets and logs | no keys or data values in logs; test stack isolated from the user's `.env` |
| Browser | no CORS for foreign origins; `nosniff`, `Referrer-Policy`, frame denial added |
| Denial of service | HugeGraph OOM via path search fixed; bounded jobs, scans and queries |
| Dependencies | see [dependency-audit.md](dependency-audit.md) |

## Findings

### Fixed: path search could exhaust HugeGraph's memory

Finding Path between two entities (Explorer, `POST /paths`, MCP `find_path`,
GraphRAG `find_path`) used HugeGraph's `traversers/shortestpath`. That
traverser allocates fixed-size records for every search level: on a
35-entity graph it threw `OutOfMemoryError` with heaps of 512 MiB, 1 GiB,
1.5 GiB and (intermittently) 2 GiB, after which HugeGraph refused other
requests ("available memory below threshold"). Anyone who can reach the
app could take the graph store down with a few path searches.

Fix: the HugeGraph adapter now runs the same bounded breadth-first search as
the reference in-memory store (`graph/edges` per expanded entity, at most
2,000 expanded entities, then `400 INVALID_REQUEST`). All 47 GraphStore
contract tests and all three demos pass against HugeGraph with a 512 MiB
heap. Compose also pins HugeGraph's heap (`MINDGRAPH_HUGEGRAPH_HEAP`, default
1g) instead of the image's "half of free memory".

### Fixed: no browser hardening headers

The web app sent `X-Powered-By: Next.js` and no hardening headers. It now
sends `X-Content-Type-Options: nosniff`, `Referrer-Policy: same-origin`,
`X-Frame-Options: DENY` and `Content-Security-Policy: frame-ancestors 'none'`
on every route, and no `X-Powered-By`.

### Accepted for the alpha

- **No authentication, authorisation or CSRF protection.** Any process or
  person that can reach 127.0.0.1:3000 or :8080 can read and change every
  graph, including a malicious web page in the same browser that sends
  simple cross-origin `POST` requests (no CORS headers are sent, so it cannot
  read the responses). Mitigation: local use only, documented prominently.
- **No full Content-Security-Policy.** Script rules need per-request nonces
  for Next.js inline scripts; planned after the alpha.
- **HugeGraph and PostgreSQL credentials.** HugeGraph runs without
  authentication and PostgreSQL defaults to `mindgraph` / `mindgraph`; both
  are reachable only on the internal Docker network.
- **Model providers receive data** (field names and sample values for
  suggestions; questions and graph evidence for Ask MindGraph) when the user
  configures one. Documented in the AI guides and `.env.example`.
- **Prompt injection.** Dataset values and graph properties are passed to
  models as escaped, untrusted data; models have no tools; every suggestion
  is validated by Rust and every citation checked against retrieved
  evidence. Covered by `test_injected_graph_properties_stay_escaped_data`,
  `test_injected_questions_stay_escaped_data` and the MCP test
  `instructions_inside_graph_properties_stay_data`. A model can still phrase
  a misleading answer from real evidence.

## Evidence

### Network exposure

`podman port` on the running stack:

| Service | Published |
|---|---|
| web | `3000/tcp -> 127.0.0.1:23000` (test ports; default 3000) |
| core | `8080/tcp -> 127.0.0.1:28180` (default 8080) |
| ai, postgres, hugegraph | nothing |

The `demo` service is in a Compose profile and never starts with `up`.

### Containers

| Image | User |
|---|---|
| core (and demo) | uid 10001 (`mindgraph`) |
| ai | uid 10001 |
| web | uid 1000 (`node`) |

No secret is copied into an image: the Core build context contains only
`Cargo.toml`, `Cargo.lock`, `crates/` and `migrations/` (`.dockerignore`);
configuration arrives as environment variables at run time.

### Uploads

| Probe | Result |
|---|---|
| 51 MiB CSV (limit 50 MiB) | `413 PAYLOAD_TOO_LARGE`, nothing stored (file count in `/data` and `/tmp` unchanged) |
| filename `../../../etc/passwd.csv` | accepted as `passwd.csv`, stored as `/data/sources/<uuid>/source`; nothing written outside |
| `x.exe`, `application/octet-stream` | `UNSUPPORTED_FILE_TYPE` ("only .csv and .json files are supported") |
| CSV with an unclosed quote | analysis job `failed` with `MALFORMED_DATASET` (line number, no values) |
| truncated JSON array | analysis job `failed` with `MALFORMED_DATASET` (position, no values) |
| record, field and value limits | enforced while reading (`MINDGRAPH_MAX_INGESTION_RECORDS`, default 100,000); covered by the ingestion unit test `limits_are_enforced` |

### API inputs and errors

| Probe | Result |
|---|---|
| invalid JSON body | `400 INVALID_JSON` with the parser message and a `request_id` |
| 3 MB JSON body | `413` |
| unknown graph id | `404 GRAPH_NOT_FOUND`, envelope only |
| malformed id | `400` with the error envelope |
| unknown route | `404 ROUTE_NOT_FOUND` |
| `Origin: https://evil.example` | no `Access-Control-*` headers |

Errors never include stack traces, SQL, file paths or stored values.

### Secrets and logs

- After loading all demos and running the probes, `podman logs` of core and
  ai contain none of the demo names, emails, ids or device ids, and not the
  traversal filename.
- `scripts/smoke-stack.sh` uses `--env-file scripts/smoke.env`, so the test
  stack never reads the developer's `.env`; verified with `compose config`
  (empty provider, empty API key).
- `.env` is git-ignored; `.env.example` holds placeholders only.

### MCP server

The MCP protocol tests ran against the release binary
(`cargo test -p mindgraph-mcp --release --test mcp_protocol`, 7 passed):
discovery lists exactly the read-only tools, `ask_graph` is absent unless
enabled, results are structured and size-limited, failures are tool errors,
graph text stays data, stdout carries only protocol messages and EOF ends
the process. The agent flow passed against the Compose stack
(`make test-mcp-live`). The binary links no database driver or server crate
(`make check-mcp-boundary`).

## Re-running the review

```bash
COMPOSE="podman compose" scripts/smoke-stack.sh --keep   # or plain docker
# then repeat the probes above against 127.0.0.1:28180 and :23000
podman compose -p mindgraph-smoke --env-file scripts/smoke.env down -v
```
