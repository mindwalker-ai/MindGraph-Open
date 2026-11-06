.DEFAULT_GOAL := help
SHELL := /bin/bash

WEB_DIR := apps/web
AI_DIR := services/ai

# Container engine for the throwaway HugeGraph used by live tests
# (`make hugegraph-test-up CONTAINER=podman` works too).
CONTAINER ?= docker
HUGEGRAPH_IMAGE := docker.io/hugegraph/hugegraph:1.7.0
HUGEGRAPH_TEST_PORT ?= 18080
POSTGRES_IMAGE := docker.io/library/postgres:17-alpine
POSTGRES_TEST_PORT ?= 15432
# Throwaway test credentials for the local test container only.
POSTGRES_TEST_URL ?= postgres://mindgraph:mindgraph@127.0.0.1:$(POSTGRES_TEST_PORT)/mindgraph

# Crates mindgraph-domain must never depend on (ADR-002).
FORBIDDEN_DOMAIN_DEPS := axum|axum-core|hyper|http|reqwest|sqlx|sqlx-core|tokio|tower|tonic
# Crates mindgraph-core (application layer) must never depend on (ADR-002).
FORBIDDEN_CORE_DEPS := axum|axum-core|hyper|reqwest|sqlx|sqlx-core|sqlx-postgres|tokio-postgres|tower
# mindgraph-mcp (Apache-2.0, ADR-015) reaches MindGraph only over Core's REST
# API (ADR-011, M8): it must not link another MindGraph crate or a database.
FORBIDDEN_MCP_DEPS := mindgraph-[a-z-]+|sqlx|sqlx-core|sqlx-postgres|tokio-postgres

.PHONY: help
help: ## Show available targets
	@grep -hE '^[a-zA-Z0-9_-]+:.*?## ' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-22s\033[0m %s\n", $$1, $$2}'

# --- Docker Compose ----------------------------------------------------------

.PHONY: up down logs ps
up: ## Build and start the full stack (web, core, ai, postgres, hugegraph)
	docker compose up --build

down: ## Stop the stack (keeps data volumes)
	docker compose down

logs: ## Follow logs of all services
	docker compose logs -f

ps: ## Show service status and health
	docker compose ps

# --- Setup -------------------------------------------------------------------

.PHONY: setup
setup: ## Install web and AI dependencies for local development
	cd $(WEB_DIR) && npm ci
	cd $(AI_DIR) && uv sync

# --- Local development servers -----------------------------------------------

.PHONY: dev-core dev-web dev-ai
dev-core: ## Run mindgraph-core on 127.0.0.1:8080
	cargo run -p mindgraph-api --bin mindgraph-core

dev-web: ## Run the Next.js dev server on http://localhost:3000
	cd $(WEB_DIR) && npm run dev

dev-ai: ## Run the AI service on 127.0.0.1:8000
	cd $(AI_DIR) && uv run uvicorn mindgraph_ai.main:app --reload --port 8000

# --- Quality gates (mirror CI) -----------------------------------------------

.PHONY: check fmt rust-check web-check ai-check check-domain-boundary check-core-boundary check-mcp-boundary
check: rust-check web-check ai-check ## Run every CI check locally

fmt: ## Format Rust, TypeScript and Python code
	cargo fmt --all
	cd $(WEB_DIR) && npm run format
	cd $(AI_DIR) && uv run ruff format .

rust-check: check-domain-boundary check-core-boundary check-mcp-boundary ## cargo fmt --check, clippy and tests
	cargo fmt --all --check
	cargo clippy --workspace --all-targets --locked -- -D warnings
	cargo test --workspace --locked

web-check: ## Typecheck, lint, format check and tests for apps/web
	cd $(WEB_DIR) && npm run typecheck && npm run lint && npm run format:check && npm test

ai-check: ## Ruff lint, format check and pytest for services/ai
	cd $(AI_DIR) && uv run ruff check . && uv run ruff format --check . && uv run pytest

check-domain-boundary: ## Fail if mindgraph-domain depends on infrastructure crates
	@deps=$$(cargo tree -p mindgraph-domain -e normal --locked --prefix none --format '{p}' | awk '{print $$1}' | sort -u); \
	bad=$$(echo "$$deps" | grep -xE '$(FORBIDDEN_DOMAIN_DEPS)' || true); \
	if [ -n "$$bad" ]; then \
		echo "mindgraph-domain must not depend on: $$bad (ADR-002)"; exit 1; \
	fi; \
	echo "mindgraph-domain dependency boundary OK"

check-core-boundary: ## Fail if mindgraph-core depends on HTTP or database crates
	@deps=$$(cargo tree -p mindgraph-core -e normal --locked --prefix none --format '{p}' | awk '{print $$1}' | sort -u); \
	bad=$$(echo "$$deps" | grep -xE '$(FORBIDDEN_CORE_DEPS)' || true); \
	if [ -n "$$bad" ]; then \
		echo "mindgraph-core must not depend on: $$bad (ADR-002)"; exit 1; \
	fi; \
	echo "mindgraph-core dependency boundary OK"

check-mcp-boundary: ## Fail if mindgraph-mcp links a MindGraph server crate or a database driver
	@deps=$$(cargo tree -p mindgraph-mcp -e normal --locked --prefix none --format '{p}' | awk '{print $$1}' | grep -vx 'mindgraph-mcp' | sort -u); \
	bad=$$(echo "$$deps" | grep -xE '$(FORBIDDEN_MCP_DEPS)' || true); \
	if [ -n "$$bad" ]; then \
		echo "mindgraph-mcp must not depend on: $$bad (ADR-011, ADR-015)"; exit 1; \
	fi; \
	echo "mindgraph-mcp dependency boundary OK"

# --- MCP server (M8) ---------------------------------------------------------

.PHONY: mcp-build test-mcp-live
mcp-build: ## Build the mindgraph-mcp binary (target/release/mindgraph-mcp)
	cargo build -p mindgraph-mcp --release --locked

test-mcp-live: ## Run the MCP agent flow against a running Core (MINDGRAPH_MCP_LIVE_CORE_URL, default http://127.0.0.1:8080)
	MINDGRAPH_MCP_LIVE_CORE_URL=$${MINDGRAPH_MCP_LIVE_CORE_URL:-http://127.0.0.1:8080} \
		cargo test -p mindgraph-mcp --test end_to_end --locked -- --ignored --exact \
		an_agent_finds_the_shared_device_over_mcp_on_a_running_core

# --- Live HugeGraph integration tests (opt-in) -------------------------------

.PHONY: hugegraph-test-up hugegraph-test-down test-hugegraph
hugegraph-test-up: ## Start a throwaway HugeGraph 1.7 on 127.0.0.1:18080 for live tests
	$(CONTAINER) run -d --rm --name mindgraph-hugegraph-test \
		-p 127.0.0.1:$(HUGEGRAPH_TEST_PORT):8080 $(HUGEGRAPH_IMAGE)
	@echo "Waiting for HugeGraph..."
	@curl -fsS --retry 60 --retry-all-errors --retry-delay 3 \
		http://127.0.0.1:$(HUGEGRAPH_TEST_PORT)/versions >/dev/null && echo "HugeGraph is ready"

hugegraph-test-down: ## Stop the throwaway HugeGraph (its data is discarded)
	$(CONTAINER) rm -f mindgraph-hugegraph-test

test-hugegraph: ## Run the GraphStore contract, the M4 ingestion flow and the M8 MCP flow against a live HugeGraph (see hugegraph-test-up)
	MINDGRAPH_HUGEGRAPH_TEST_URL=http://127.0.0.1:$(HUGEGRAPH_TEST_PORT) \
		cargo test -p mindgraph-graph --test hugegraph_integration --locked -- --ignored
	MINDGRAPH_HUGEGRAPH_TEST_URL=http://127.0.0.1:$(HUGEGRAPH_TEST_PORT) \
		cargo test -p mindgraph-api --test hugegraph_ingestion --locked -- --ignored
	MINDGRAPH_HUGEGRAPH_TEST_URL=http://127.0.0.1:$(HUGEGRAPH_TEST_PORT) \
		cargo test -p mindgraph-mcp --test end_to_end --locked -- --ignored --exact \
		an_agent_finds_the_shared_device_over_mcp_on_hugegraph

# --- Live PostgreSQL integration tests (opt-in) ------------------------------

.PHONY: postgres-test-up postgres-test-down test-postgres
postgres-test-up: ## Start a throwaway PostgreSQL 17 on 127.0.0.1:15432 for live tests
	$(CONTAINER) run -d --rm --name mindgraph-postgres-test \
		-e POSTGRES_USER=mindgraph -e POSTGRES_PASSWORD=mindgraph -e POSTGRES_DB=mindgraph \
		-p 127.0.0.1:$(POSTGRES_TEST_PORT):5432 $(POSTGRES_IMAGE)
	@echo "Waiting for PostgreSQL..."
	@until $(CONTAINER) exec mindgraph-postgres-test pg_isready -U mindgraph -d mindgraph >/dev/null 2>&1; do sleep 1; done
	@echo "PostgreSQL is ready"

postgres-test-down: ## Stop the throwaway PostgreSQL (its data is discarded)
	$(CONTAINER) rm -f mindgraph-postgres-test

test-postgres: ## Run the metadata store tests against a live PostgreSQL (see postgres-test-up)
	MINDGRAPH_POSTGRES_TEST_URL=$(POSTGRES_TEST_URL) \
		cargo test -p mindgraph-metadata --test postgres_integration --locked -- --ignored
