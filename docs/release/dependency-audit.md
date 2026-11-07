# Dependency audit (v0.1.0-alpha.1)

Run on 2026-11-07 against the lockfiles of branch `feat/m10-launch-hardening`.
CI repeats these checks on every push (`.github/workflows/ci.yml`, job
`audit`). License findings are in [LICENSE_AUDIT.md](../legal/LICENSE_AUDIT.md).

## Summary

| Ecosystem | Tool | Result |
|---|---|---|
| Rust (Cargo.lock) | `cargo deny check` (cargo-deny 0.20.2, RustSec advisory DB) | advisories ok · licenses ok · bans ok · sources ok; 1 advisory ignored with a reason (below) |
| Web, production dependencies | `npm audit --omit=dev` | 0 vulnerabilities |
| Web, development tooling | `npm audit` | 10 moderate, all from one advisory in the lint toolchain (below) |
| AI service runtime | `pip-audit 2.9.0 -r <uv export --no-dev>` | no known vulnerabilities |
| AI service incl. dev tools | `pip-audit 2.9.0 -r <uv export>` | no known vulnerabilities |

## Findings and decisions

### RUSTSEC-2023-0089: `atomic-polyfill` is unmaintained (ignored)

- Not a vulnerability: the crate is deprecated in favour of
  `portable-atomic`.
- Path: `heapless 0.7` ← `postcard` ← build dependency of `phonenumber`
  (entity resolution phone normalisation).
- It is compiled only for targets without atomic compare-and-swap;
  `cargo tree -i atomic-polyfill` prints nothing for the supported targets
  (x86_64 and aarch64 Linux and macOS).
- Ignored in `deny.toml` with this reason. Revisit when `phonenumber`
  updates `postcard`.

### GHSA-g3hv-75wg-m249: `minimatch` ReDoS (development only, accepted)

- `minimatch >=7 <11.0.1` can be slow on crafted glob patterns.
- Reached only through ESLint 10, `eslint-config-next` and
  `typescript-eslint`, which run on developer machines and in CI with the
  repository's own glob patterns. None of it is in the production image.
- The only fix npm offers is a breaking downgrade (ESLint 9,
  eslint-config-next 15). Accepted until the lint packages move to
  `minimatch >=11.0.1`.

### Duplicate Rust crate versions (warning only)

`cargo deny` reports two versions each of `base64`, `block-buffer`,
`cpufeatures`, `crypto-common`, `digest`, `hashbrown`, `regex-syntax`, `sha2`
and `syn`, pulled by different upstream crates. They cost build time and
binary size only.

## Supply chain notes

- All three lockfiles are committed and builds use them
  (`cargo build --locked`, `npm ci`, `uv sync --frozen`).
- Tools added in M10 are pinned to releases at least two weeks old:
  `@playwright/test 1.64.1` (not the day-old 1.65.0) and `cargo-deny 0.20.2`.
- GitHub Actions are pinned to commit SHAs with the version in a comment,
  and workflows default to read-only `contents: read` permissions.

## Reproduce

```bash
cargo install cargo-deny --version 0.20.2 --locked && cargo deny check
cd apps/web && npm audit --omit=dev && npm audit
cd services/ai && uv export --no-dev --no-hashes --no-emit-project > /tmp/ai.txt \
  && uv tool run --from pip-audit==2.9.0 pip-audit -r /tmp/ai.txt --no-deps --disable-pip
```
