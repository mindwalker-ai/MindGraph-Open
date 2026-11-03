-- MindGraph Open metadata schema, version 1 (ADR-005).
--
-- PostgreSQL holds control-plane metadata only. Entities and relationships
-- live in the GraphStore (HugeGraph), never here.
--
-- Migrations are append-only once released: change the schema with a new
-- migration file, never by editing this one.
--
-- Constraints are a second line of defence. Domain validation in Rust
-- remains the primary business-rule layer.

-- Workspaces ------------------------------------------------------------------
-- v0.1 runs one local workspace, found by its stable `system_key = 'local'`.
-- `system_key` is infrastructure metadata and is not part of the domain model.
CREATE TABLE workspaces (
    id          UUID        PRIMARY KEY,
    system_key  TEXT        UNIQUE,
    name        TEXT        NOT NULL CHECK (length(btrim(name)) > 0),
    created_at  TIMESTAMPTZ NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL,
    CHECK (updated_at >= created_at)
);

-- Graph projects --------------------------------------------------------------
CREATE TABLE graph_projects (
    id                      UUID        PRIMARY KEY,
    workspace_id            UUID        NOT NULL REFERENCES workspaces (id),
    name                    TEXT        NOT NULL CHECK (length(btrim(name)) > 0),
    description             TEXT,
    status                  TEXT        NOT NULL
                                        CHECK (status IN ('draft', 'active', 'archived')),
    -- Every graph always has an active ontology (see the deferred foreign key
    -- below): a graph and its ontology v1 are created in one transaction.
    active_ontology_version INTEGER     NOT NULL CHECK (active_ontology_version > 0),
    created_at              TIMESTAMPTZ NOT NULL,
    updated_at              TIMESTAMPTZ NOT NULL,
    CHECK (updated_at >= created_at)
);

-- Serves "list graphs of a workspace, newest first".
CREATE INDEX graph_projects_workspace_listing
    ON graph_projects (workspace_id, created_at DESC, id DESC);

-- Ontology versions -----------------------------------------------------------
-- Immutable snapshots: v1, v2, v3, ... per graph. The schema is stored as a
-- versioned JSON document written by the metadata adapter's codec.
CREATE TABLE ontology_versions (
    graph_id    UUID        NOT NULL REFERENCES graph_projects (id),
    version     INTEGER     NOT NULL CHECK (version > 0),
    schema_json JSONB       NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (graph_id, version)
);

-- The active version must exist. Deferred, so a graph row and its first
-- ontology row can be inserted in either order inside one transaction.
ALTER TABLE graph_projects
    ADD CONSTRAINT graph_projects_active_ontology_fk
    FOREIGN KEY (id, active_ontology_version)
    REFERENCES ontology_versions (graph_id, version)
    DEFERRABLE INITIALLY DEFERRED;

-- Ontology versions are never updated in place.
CREATE FUNCTION reject_ontology_version_update() RETURNS trigger
    LANGUAGE plpgsql AS
$$
BEGIN
    RAISE EXCEPTION 'ontology versions are immutable; create a new version instead'
        USING ERRCODE = 'restrict_violation';
END;
$$;

CREATE TRIGGER ontology_versions_immutable
    BEFORE UPDATE ON ontology_versions
    FOR EACH ROW EXECUTE FUNCTION reject_ontology_version_update();

-- Baseline tables for M4+ -------------------------------------------------------
-- Created now so the v0.1 schema is complete. No application logic uses them
-- yet; M4 (dataset ingestion) adds behaviour and, if needed, new migrations.

CREATE TABLE data_sources (
    id          UUID        PRIMARY KEY,
    graph_id    UUID        NOT NULL REFERENCES graph_projects (id),
    kind        TEXT        NOT NULL CHECK (kind IN ('csv', 'json')),
    filename    TEXT        NOT NULL CHECK (length(btrim(filename)) > 0),
    status      TEXT        NOT NULL CHECK (status IN ('pending', 'ready', 'failed')),
    created_at  TIMESTAMPTZ NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL,
    CHECK (updated_at >= created_at),
    -- Lets ingestion jobs reference a data source of the same graph.
    UNIQUE (id, graph_id)
);

CREATE INDEX data_sources_graph ON data_sources (graph_id);

CREATE TABLE ingestion_jobs (
    id              UUID             PRIMARY KEY,
    graph_id        UUID             NOT NULL REFERENCES graph_projects (id),
    data_source_id  UUID             NOT NULL,
    status          TEXT             NOT NULL
                                     CHECK (status IN ('queued', 'running', 'completed', 'failed', 'cancelled')),
    progress        DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (progress >= 0 AND progress <= 1),
    -- ADR-008: a repeated request with the same key must not ingest twice.
    idempotency_key TEXT,
    error_code      TEXT,
    error_message   TEXT,
    created_at      TIMESTAMPTZ      NOT NULL,
    updated_at      TIMESTAMPTZ      NOT NULL,
    CHECK (updated_at >= created_at),
    FOREIGN KEY (data_source_id, graph_id) REFERENCES data_sources (id, graph_id),
    UNIQUE (graph_id, idempotency_key)
);

CREATE INDEX ingestion_jobs_graph ON ingestion_jobs (graph_id, created_at DESC);

CREATE TABLE saved_views (
    id          UUID        PRIMARY KEY,
    graph_id    UUID        NOT NULL REFERENCES graph_projects (id),
    name        TEXT        NOT NULL CHECK (length(btrim(name)) > 0),
    view_json   JSONB       NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL,
    CHECK (updated_at >= created_at)
);

CREATE INDEX saved_views_graph ON saved_views (graph_id);
