"""Checks docs/api/openapi.yaml against the Axum router (M10).

1. Validates the document against the OpenAPI 3.1 specification
   (openapi-spec-validator).
2. Compares every (method, path) the router in
   crates/mindgraph-api/src/routes.rs serves with the operations the
   document describes, and fails on any difference in either direction.

Run with `make check-openapi`, which provides the two dependencies:

    uv run --no-project --with pyyaml==6.0.3 --with openapi-spec-validator==0.9.0 \
        python scripts/check_openapi.py
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

import yaml
from openapi_spec_validator import validate

ROOT = Path(__file__).resolve().parent.parent
SPEC = ROOT / "docs" / "api" / "openapi.yaml"
ROUTES = ROOT / "crates" / "mindgraph-api" / "src" / "routes.rs"
METHODS = ("get", "post", "put", "patch", "delete")


def call_arguments(text: str, start: int) -> str:
    """The text between the parenthesis at `start` and its matching one."""
    depth = 0
    for index in range(start, len(text)):
        if text[index] == "(":
            depth += 1
        elif text[index] == ")":
            depth -= 1
            if depth == 0:
                return text[start + 1 : index]
    raise ValueError("unbalanced parentheses in routes.rs")


def router_operations(text: str, prefix: str) -> set[tuple[str, str]]:
    operations = set()
    for match in re.finditer(r"\.route\(", text):
        arguments = call_arguments(text, match.end() - 1)
        path = re.match(r'\s*"([^"]+)"', arguments)
        if not path:
            raise ValueError(f"route without a literal path: {arguments[:60]}")
        for method in re.findall(rf"\b({'|'.join(METHODS)})\(", arguments):
            operations.add((method.upper(), prefix + path.group(1)))
    return operations


def served_operations() -> set[tuple[str, str]]:
    source = ROUTES.read_text()
    api_start = source.index("let api = Router::new()")
    api_end = source.index(";", api_start)
    api = source[api_start:api_end]
    top = source[api_end:]
    if '.nest("/api/v1", api)' not in top:
        raise ValueError("the API router is no longer nested under /api/v1")
    return router_operations(api, "/api/v1") | router_operations(top, "")


def documented_operations(spec: dict) -> set[tuple[str, str]]:
    return {
        (method.upper(), path)
        for path, item in spec.get("paths", {}).items()
        for method in item
        if method in METHODS
    }


def main() -> int:
    spec = yaml.safe_load(SPEC.read_text())
    validate(spec)
    print(f"ok    {SPEC.relative_to(ROOT)} is a valid OpenAPI {spec['openapi']} document")

    served = served_operations()
    documented = documented_operations(spec)
    missing = sorted(served - documented)
    stale = sorted(documented - served)
    for method, path in missing:
        print(f"FAIL  served but not documented: {method} {path}")
    for method, path in stale:
        print(f"FAIL  documented but not served: {method} {path}")
    if missing or stale:
        return 1
    print(f"ok    all {len(served)} router operations are documented, and nothing else is")
    return 0


if __name__ == "__main__":
    sys.exit(main())
