# Contributing to MindGraph-OSS

Thanks for your interest. MindGraph-OSS is in **alpha**: interfaces still
change, and the maintainers review every change against the architecture
decisions in [`docs/adr/`](docs/adr/).

## Before you start

- **Bugs:** open an issue with the bug template. Include the version
  (`mindgraph --version` or the commit), how you run it (Docker, Podman or
  native) and the steps to reproduce. Never paste API keys, real personal
  data or `.env` contents.
- **Security issues:** do not open a public issue. Follow
  [SECURITY.md](SECURITY.md).
- **Features:** open a feature request first, so scope can be agreed before
  you write code. New infrastructure or services need an ADR.

## Development setup

You need [Rust](https://rustup.rs) stable (1.99+), Node.js 22+ and
[uv](https://docs.astral.sh/uv/) (it installs Python 3.13). Docker with
Compose v2, or Podman with `podman compose`, is only needed for the full
stack and the live tests.

```bash
make setup     # npm ci (apps/web) + uv sync (services/ai)
make check     # everything CI runs offline: Rust, web and AI checks
make help      # all targets
```

See the [development guide](docs/development/README.md) for running the
services natively, the live HugeGraph and PostgreSQL tests, the browser
end-to-end tests and the full-stack smoke test.

## Rules the code follows

1. **ADRs win.** When an ADR and a code convention disagree, follow the ADR
   or propose a change to it.
2. **Boundaries are checked in CI.** `mindgraph-domain` has no
   infrastructure dependencies; `mindgraph-core` has no HTTP or database
   crates; graph data goes through the `GraphStore` port only;
   `mindgraph-mcp` reaches MindGraph over the REST API only. `make check`
   fails when one is broken.
3. **No AI in the critical path.** Manual flows (upload, ontology, mapping,
   ingestion, exploration, entity resolution) must keep working with no
   model configured.
4. **Privacy in logs.** Logs never contain prompts, model output, graph
   property values, dataset rows or credentials. Log ids, counts and
   timings.
5. **Tests with the change.** Bug fixes come with a test that fails without
   the fix. Prefer the in-memory stores for unit tests; add a live test when
   behaviour depends on HugeGraph or PostgreSQL.
6. **Honest UI and docs.** Do not show controls that do nothing, invented
   numbers or features that are not implemented.

## Pull requests

- Keep a pull request to one topic. Describe what changed, why, and how you
  tested it (the template asks for this).
- Run `make check` before pushing. CI also runs the live HugeGraph and
  PostgreSQL suites, the dependency audit, the OpenAPI check, the full-stack
  smoke test and the browser journeys.
- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/),
  for example `fix(explorer): keep the path finder open after "Use"`.
- Update the docs and `CHANGELOG.md` when behaviour changes.

## Licensing of contributions

MindGraph-OSS does not ask for a contributor license agreement (CLA) or a
DCO sign-off in this alpha. Your contribution is offered under the license
that applies to the files you change (see [LICENSE](LICENSE) and
[docs/legal/LICENSE_AUDIT.md](docs/legal/LICENSE_AUDIT.md)). The project's
licensing split is still awaiting legal approval, and whether a CLA or DCO
will be required later is one of the open questions recorded there.

## Code of conduct

Everyone taking part is expected to follow the
[Code of Conduct](CODE_OF_CONDUCT.md).
