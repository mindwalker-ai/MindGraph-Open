#!/usr/bin/env bash
# Isolated full-stack smoke test (M10).
#
# Builds and starts a throwaway copy of the stack under its own Compose
# project name (default "mindgraph-smoke"), with its own volumes and ports
# (scripts/smoke.env), then:
#
#   1. loads the fraud demo through the REST API (create graph, ontology,
#      upload, analyse, ingest, verify GraphStore data);
#   2. runs scripts/smoke.sh against the published ports (Core and web);
#   3. loads the demo again: it must be reused, not duplicated;
#   4. restarts the stack with `down` + `up` (volumes kept) and checks that
#      the data is still there;
#   5. with --e2e, runs the Playwright browser journeys (needs Node and
#      `npx playwright install chromium` in apps/web).
#
# Afterwards it removes that project's containers and volumes, and nothing
# else: your own stack (project "mindgraph") and its data are never touched.
#
#   scripts/smoke-stack.sh                         # Docker
#   COMPOSE="podman compose" scripts/smoke-stack.sh
#   scripts/smoke-stack.sh --e2e --keep            # keep it running afterwards

set -euo pipefail
cd "$(dirname "$0")/.."

COMPOSE=${COMPOSE:-docker compose}
PROJECT=${MINDGRAPH_SMOKE_PROJECT:-mindgraph-smoke}
ENV_FILE=scripts/smoke.env
E2E=false
KEEP=false
for arg in "$@"; do
  case $arg in
    --e2e) E2E=true ;;
    --keep) KEEP=true ;;
    -h | --help)
      sed -n '2,24p' "$0"
      exit 0
      ;;
    *)
      echo "unknown option: $arg" >&2
      exit 2
      ;;
  esac
done

if [ "$PROJECT" = "mindgraph" ]; then
  echo "refusing to use the project name of your own stack (mindgraph)" >&2
  exit 2
fi

# Ports from smoke.env, unless overridden in the environment.
env_value() { sed -n "s/^$1=//p" "$ENV_FILE"; }
WEB_PORT=${MINDGRAPH_WEB_PORT:-$(env_value MINDGRAPH_WEB_PORT)}
CORE_PORT=${MINDGRAPH_CORE_PORT:-$(env_value MINDGRAPH_CORE_PORT)}
export MINDGRAPH_WEB_PORT=$WEB_PORT MINDGRAPH_CORE_PORT=$CORE_PORT
CORE_URL=http://127.0.0.1:$CORE_PORT
WEB_URL=http://127.0.0.1:$WEB_PORT

compose() { $COMPOSE -p "$PROJECT" --env-file "$ENV_FILE" "$@"; }

# Fail early, before building, if something already listens on our ports.
for port in "$WEB_PORT" "$CORE_PORT"; do
  if (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
    echo "port $port is already in use; set MINDGRAPH_WEB_PORT / MINDGRAPH_CORE_PORT" >&2
    exit 2
  fi
done
step() { printf '\n==> %s\n' "$*"; }

cleanup() {
  status=$?
  if [ $status -ne 0 ]; then
    step "FAILED (exit $status); service status and recent Core logs"
    compose ps || true
    compose logs --tail 80 core || true
  fi
  if [ "$KEEP" = true ]; then
    step "Keeping project $PROJECT: web $WEB_URL, core $CORE_URL"
    echo "Remove it with: $COMPOSE -p $PROJECT --env-file $ENV_FILE down -v"
  else
    step "Removing project $PROJECT (its containers and volumes only)"
    compose --profile demo down -v --remove-orphans || true
  fi
  exit $status
}
trap cleanup EXIT

step "Validating the Compose file"
compose config -q

step "Building and starting project $PROJECT (web $WEB_URL, core $CORE_URL)"
compose up -d --build

step "Loading the fraud demo through the REST API"
compose --profile demo run --rm demo fraud

step "Smoke test through the published ports"
MINDGRAPH_SMOKE_CORE_URL=$CORE_URL MINDGRAPH_SMOKE_WEB_URL=$WEB_URL scripts/smoke.sh

step "Loading the demo again: it must be reused, not duplicated"
output=$(compose --profile demo run --rm demo fraud)
echo "$output"
grep -q "Found the existing demo graph" <<<"$output"

step "Restarting with down + up (volumes kept): the data must survive"
compose down
compose up -d
MINDGRAPH_SMOKE_CORE_URL=$CORE_URL MINDGRAPH_SMOKE_WEB_URL=$WEB_URL scripts/smoke.sh

if [ "$E2E" = true ]; then
  step "Browser journeys (Playwright)"
  (cd apps/web && MINDGRAPH_E2E_WEB_URL=$WEB_URL npx playwright test)
fi

step "Full-stack smoke test passed"
