# Security policy

## Supported versions

MindGraph-OSS is in alpha. Only the latest release and the `main` branch
receive fixes.

| Version | Supported |
|---|---|
| `main` | yes |
| `v0.1.0-alpha.1` | yes, until the next release |
| anything older | no |

## Reporting a vulnerability

Please **do not** open a public issue, discussion or pull request for a
security problem.

Report it privately through GitHub's private vulnerability reporting:
open the repository's **Security** tab and choose **Report a
vulnerability** (direct link:
<https://github.com/mindwalker-ai/MindGraph-Open/security/advisories/new>).
If that form is not available, open a public issue that only asks the
maintainers for a private contact, without any details of the problem.

Include what you can: the affected component (web, core API, AI service,
MCP server, Docker setup), the version or commit, steps to reproduce, and
the impact you expect. Do not include real personal data or live
credentials.

The maintainers will acknowledge the report, keep you informed and credit
you in the advisory unless you prefer otherwise. No response times are
guaranteed during the alpha.

## Security model of this release

Read this before you deploy MindGraph-OSS anywhere but your own machine.

- **No authentication or authorisation.** Anyone who can reach the web app
  (port 3000) or the Core API (port 8080) can read and change every graph.
  There are no users, roles or API keys in v0.1.
- **Local use only by default.** Docker Compose publishes only the web app
  and the Core API, and only on `127.0.0.1`. PostgreSQL, HugeGraph and the
  AI service stay on the internal Docker network.
- **Do not expose it to the internet** or to an untrusted network, and do
  not put it behind a public reverse proxy, without adding your own
  authentication in front of it. It is not designed for that.
- **Default credentials are local-only.** The PostgreSQL user and password
  default to `mindgraph` / `mindgraph` and the database is not published.
  Change them in `.env` before first start if other people can use the
  machine (see [configuration](docs/getting-started/README.md#configuration)).
- **Model providers receive data.** When an LLM provider is configured, AI
  suggestions send field names and sample values, and Ask MindGraph sends
  the question and retrieved graph evidence, to that provider. Use a local
  model for sensitive data. With no provider configured nothing leaves the
  machine.
- **The MCP server is read-only** and scoped to one graph, but it gives the
  connected agent everything that graph contains.

The alpha security review and its findings are in
[docs/release/security-review.md](docs/release/security-review.md).
