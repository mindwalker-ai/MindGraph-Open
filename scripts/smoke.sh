#!/usr/bin/env bash
# Full-stack smoke test of a running MindGraph stack (M10).
#
#   docker compose up -d --build
#   make demo-fraud          # loads the fraud demo through the REST API
#   make smoke               # this script
#
# It talks to the published ports only: Core on 127.0.0.1:${MINDGRAPH_CORE_PORT:-8080}
# and the web app on 127.0.0.1:${MINDGRAPH_WEB_PORT:-3000}. It only reads, so
# it is safe on a stack with real graphs. Needs bash, curl, grep and sed (no
# jq, Rust, Node or Python).
#
# Environment:
#   MINDGRAPH_SMOKE_CORE_URL   default http://127.0.0.1:${MINDGRAPH_CORE_PORT:-8080}
#   MINDGRAPH_SMOKE_WEB_URL    default http://127.0.0.1:${MINDGRAPH_WEB_PORT:-3000}
#   MINDGRAPH_SMOKE_TIMEOUT    seconds to wait for the stack (default 300)
#   MINDGRAPH_SMOKE_GRAPH      demo graph name (default "Demo: Fraud investigation")

set -euo pipefail

CORE=${MINDGRAPH_SMOKE_CORE_URL:-http://127.0.0.1:${MINDGRAPH_CORE_PORT:-8080}}
WEB=${MINDGRAPH_SMOKE_WEB_URL:-http://127.0.0.1:${MINDGRAPH_WEB_PORT:-3000}}
TIMEOUT=${MINDGRAPH_SMOKE_TIMEOUT:-300}
GRAPH_NAME=${MINDGRAPH_SMOKE_GRAPH:-Demo: Fraud investigation}

passed=0
pass() {
  passed=$((passed + 1))
  printf '  ok    %s\n' "$1"
}
fail() {
  printf '  FAIL  %s\n' "$1" >&2
  if [ -n "${2:-}" ]; then printf '        %s\n' "$2" >&2; fi
  exit 1
}

# request METHOD URL [JSON BODY]: sets $status and $body, never exits.
request() {
  local method=$1 url=$2 data=${3:-} out
  if [ -n "$data" ]; then
    out=$(curl -sS --max-time 30 -X "$method" -H 'Content-Type: application/json' \
      --data "$data" -w '\n%{http_code}' "$url" 2>&1) || true
  else
    out=$(curl -sS --max-time 30 -X "$method" -w '\n%{http_code}' "$url" 2>&1) || true
  fi
  status=${out##*$'\n'}
  body=${out%$'\n'*}
}

# expect STATUS DESCRIPTION [BODY REGEX]
expect() {
  if [ "$status" != "$1" ]; then
    fail "$2" "expected HTTP $1, got ${status:-no response}: ${body:0:300}"
  fi
  if [ -n "${3:-}" ] && ! grep -qE "$3" <<<"$body"; then
    fail "$2" "the response does not match /$3/: ${body:0:300}"
  fi
  pass "$2"
}

# wait_for URL DESCRIPTION: bounded by $TIMEOUT.
wait_for() {
  local deadline=$((SECONDS + TIMEOUT))
  until curl -fsS --max-time 5 -o /dev/null "$1" 2>/dev/null; do
    if [ $SECONDS -ge $deadline ]; then fail "$2" "$1 did not answer within ${TIMEOUT}s"; fi
    sleep 2
  done
  pass "$2"
}

# The "id" of the object in a graph list ($body) whose "name" is exactly $1.
graph_id_named() {
  sed 's/},{/}\n{/g' <<<"$body" | grep -F "\"name\":\"$1\"" | head -n1 |
    sed -nE 's/.*"id":"([0-9a-f-]{36})".*/\1/p'
}

# The id of the first entity in a search response ($body).
first_entity_id() {
  sed -nE 's/.*"data":\[\{"id":"([0-9a-f-]{36})".*/\1/p' <<<"$body"
}

echo "MindGraph smoke test: core $CORE, web $WEB"

echo "Core"
wait_for "$CORE/ready" "core is ready (PostgreSQL and HugeGraph answer)"
request GET "$CORE/health"
expect 200 "liveness: GET /health" '"status":"ok"'
request GET "$CORE/api/v1/graphs/not-a-graph-id"
expect 400 "invalid ids are rejected with the error envelope" '"error":\{"code":"[A-Z_]+"'
request GET "$CORE/api/v1/no-such-route"
expect 404 "unknown routes answer ROUTE_NOT_FOUND" 'ROUTE_NOT_FOUND'
request GET "$CORE/api/v1/graph-builder/status"
expect 200 "AI Graph Builder status is reported (configured or not)" '"configured":(true|false)'

echo "Web"
wait_for "$WEB/" "web app answers"
request GET "$WEB/"
expect 200 "home page renders" 'MindGraph'
request GET "$WEB/api/v1/graphs?limit=100"
expect 200 "web proxies the REST API" '"data":\['

GRAPH=$(graph_id_named "$GRAPH_NAME")
if [ -z "$GRAPH" ]; then fail "demo graph \"$GRAPH_NAME\" exists" "load it first: make demo-fraud"; fi
pass "demo graph \"$GRAPH_NAME\" exists ($GRAPH)"

echo "Demo graph through the web proxy"
API="$WEB/api/v1/graphs/$GRAPH"
request GET "$API/ontology"
expect 200 "active ontology includes Transaction" '"name":"Transaction"'
request POST "$API/entities/search" '{"entity_type":"Customer","property_filters":{"customer_id":"C001"},"limit":2}'
expect 200 "exact search finds Alice Tan (C001)" '"count":1'
ALICE=$(first_entity_id)
request POST "$API/entities/search" '{"entity_type":"Customer","property_filters":{"customer_id":"C002"},"limit":2}'
expect 200 "exact search finds Budi Santoso (C002)" '"count":1'
BUDI=$(first_entity_id)
request GET "$API/entities/$ALICE/neighbors?direction=outgoing&relationship_type=OWNS"
expect 200 "neighbours of Alice" '"relationship_type":"OWNS"'
request POST "$API/paths" "{\"source_id\":\"$ALICE\",\"target_id\":\"$BUDI\",\"direction\":\"both\",\"max_depth\":4}"
expect 200 "Alice and Budi share a device (2 hops)" '"hops":2'
if ! grep -q 'DEVICE-921' <<<"$body"; then fail "the shared path goes through DEVICE-921" "${body:0:300}"; fi
pass "the shared path goes through DEVICE-921"
request GET "$API/resolution/candidates?status=all&limit=1"
expect 200 "entity resolution candidates are listed" '"counts":\{'

echo "Pages"
for page in "/graphs/$GRAPH/explore?entity=$ALICE" "/graphs/$GRAPH/resolution" \
  "/graphs/$GRAPH/integrations/mcp" "/graphs/$GRAPH/upload" "/demos"; do
  request GET "$WEB$page"
  expect 200 "page $page renders"
done

echo "Passed $passed checks."
