.DEFAULT_GOAL := help
SHELL := /bin/bash

WEB_DIR := apps/web
AI_DIR := services/ai

# Container engine for the throwaway HugeGraph used by live tests
# (`make hugegraph-test-up CONTAINER=podman` works too).
CONTAINER ?= docker
HUGEGRAPH_IMAGE := docker.io/hugegraph/hugegraph:1.7.0
HUGEGRAPH_TEST_PORT ?= 18080

# Crates mindgraph-domain must never depend on (ADR-002).
FORBIDDEN_DOMAIN_DEPS := axum|axum-core|hyper|http|reqwest|sqlx|sqlx-core|tokio|tower|tonic

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

.PHONY: check fmt rust-check web-check ai-check check-domain-boundary
check: rust-check web-check ai-check ## Run every CI check locally

fmt: ## Format Rust, TypeScript and Python code
	cargo fmt --all
	cd $(WEB_DIR) && npm run format
	cd $(AI_DIR) && uv run ruff format .

rust-check: check-domain-boundary ## cargo fmt --check, clippy and tests
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

test-hugegraph: ## Run the GraphStore contract against a live HugeGraph (see hugegraph-test-up)
	MINDGRAPH_HUGEGRAPH_TEST_URL=http://127.0.0.1:$(HUGEGRAPH_TEST_PORT) \
		cargo test -p mindgraph-graph --test hugegraph_integration --locked -- --ignored
