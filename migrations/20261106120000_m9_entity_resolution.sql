-- M9: entity resolution and identity review (ADR-009, refinement M9).
--
-- PostgreSQL stores resolution metadata only:
--
--   entity_resolution_scans       one bounded scan: rule snapshot and coverage
--   entity_resolution_candidates  potential duplicate pairs: ids, score and
--                                 value-free evidence
--   entity_resolution_decisions   immutable review decisions (provenance)
--   entity_identity_aliases       logical identity groups: member → canonical
--
-- No entity property value is stored here. Reviewers read the original
-- entities from the GraphStore, and no resolution decision ever changes the
-- graph: a "same entity" approval is a logical mapping, not a merge.
--
-- Append-only: earlier migrations are untouched.

-- Scans -----------------------------------------------------------------------
CREATE TABLE entity_resolution_scans (
    id                        UUID        PRIMARY KEY,
    graph_id                  UUID        NOT NULL REFERENCES graph_projects (id),
    entity_type               TEXT        NOT NULL CHECK (length(entity_type) BETWEEN 1 AND 64),
    -- The validated rules as used, so later rule changes never rewrite
    -- historical evidence.
    rules_json                JSONB       NOT NULL,
    ontology_version          INTEGER     NOT NULL,
    max_entities              INTEGER     NOT NULL CHECK (max_entities BETWEEN 1 AND 1000),
    entities_examined         INTEGER     NOT NULL CHECK (entities_examined >= 0),
    truncated                 BOOLEAN     NOT NULL,
    pairs_compared            INTEGER     NOT NULL CHECK (pairs_compared >= 0),
    comparison_budget         INTEGER     NOT NULL CHECK (comparison_budget > 0),
    comparison_budget_reached BOOLEAN     NOT NULL,
    oversized_blocks          INTEGER     NOT NULL CHECK (oversized_blocks >= 0),
    candidate_limit_reached   BOOLEAN     NOT NULL,
    candidates_found          INTEGER     NOT NULL CHECK (candidates_found >= 0),
    skipped_pending           INTEGER     NOT NULL CHECK (skipped_pending >= 0),
    skipped_rejected          INTEGER     NOT NULL CHECK (skipped_rejected >= 0),
    skipped_linked            INTEGER     NOT NULL CHECK (skipped_linked >= 0),
    -- Counts and property names only.
    warnings_json             JSONB       NOT NULL,
    duration_ms               BIGINT      NOT NULL CHECK (duration_ms >= 0),
    created_at                TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (graph_id, ontology_version) REFERENCES ontology_versions (graph_id, version),
    -- Lets candidates and decisions reference a scan of the same graph.
    UNIQUE (id, graph_id)
);

-- Serves "recent scans of a graph".
CREATE INDEX entity_resolution_scans_graph
    ON entity_resolution_scans (graph_id, created_at DESC, id DESC);

-- Candidates ------------------------------------------------------------------
-- A pair is stored with its smaller entity id first. Evidence names
-- properties, rule kinds and outcomes, never values.
CREATE TABLE entity_resolution_candidates (
    id             UUID             PRIMARY KEY,
    scan_id        UUID             NOT NULL,
    graph_id       UUID             NOT NULL,
    entity_type    TEXT             NOT NULL CHECK (length(entity_type) BETWEEN 1 AND 64),
    entity_low_id  UUID             NOT NULL,
    entity_high_id UUID             NOT NULL,
    -- A ranking heuristic, not a probability.
    score          DOUBLE PRECISION NOT NULL CHECK (score >= 0 AND score <= 1),
    signals_json   JSONB            NOT NULL,
    status         TEXT             NOT NULL CHECK (status IN ('pending', 'accepted', 'rejected')),
    created_at     TIMESTAMPTZ      NOT NULL,
    updated_at     TIMESTAMPTZ      NOT NULL,
    decided_at     TIMESTAMPTZ,
    CHECK (entity_low_id < entity_high_id),
    CHECK (updated_at >= created_at),
    CHECK ((status = 'pending') = (decided_at IS NULL)),
    FOREIGN KEY (scan_id, graph_id) REFERENCES entity_resolution_scans (id, graph_id),
    -- No duplicate pair within one scan.
    UNIQUE (scan_id, entity_low_id, entity_high_id),
    UNIQUE (id, graph_id)
);

-- At most one pending candidate per pair and graph, whichever scan found it.
CREATE UNIQUE INDEX entity_resolution_candidates_one_pending
    ON entity_resolution_candidates (graph_id, entity_low_id, entity_high_id)
    WHERE status = 'pending';

-- Review queue: score DESC, id ASC per status, and per scan.
CREATE INDEX entity_resolution_candidates_review
    ON entity_resolution_candidates (graph_id, status, score DESC, id);
CREATE INDEX entity_resolution_candidates_scan
    ON entity_resolution_candidates (scan_id, score DESC, id);
-- Pair and entity lookups (skip rules, Graph Explorer).
CREATE INDEX entity_resolution_candidates_pair
    ON entity_resolution_candidates (graph_id, entity_low_id, entity_high_id);
CREATE INDEX entity_resolution_candidates_high
    ON entity_resolution_candidates (graph_id, entity_high_id);

-- Decisions -------------------------------------------------------------------
-- Provenance: why an identity mapping exists. Never updated or deleted; a
-- reconsidered rejection adds a new decision.
CREATE TABLE entity_resolution_decisions (
    id                     UUID             PRIMARY KEY,
    graph_id               UUID             NOT NULL,
    candidate_id           UUID             NOT NULL,
    scan_id                UUID             NOT NULL,
    decision               TEXT             NOT NULL
                                            CHECK (decision IN ('same_entity', 'different_entities')),
    entity_low_id          UUID             NOT NULL,
    entity_high_id         UUID             NOT NULL,
    canonical_entity_id    UUID,
    previous_canonical_ids UUID[]           NOT NULL,
    linked_entity_ids      UUID[]           NOT NULL,
    score                  DOUBLE PRECISION NOT NULL CHECK (score >= 0 AND score <= 1),
    matched_rules          TEXT[]           NOT NULL,
    actor                  TEXT             NOT NULL CHECK (length(btrim(actor)) > 0),
    reconsidered           BOOLEAN          NOT NULL,
    created_at             TIMESTAMPTZ      NOT NULL,
    CHECK ((decision = 'same_entity') = (canonical_entity_id IS NOT NULL)),
    FOREIGN KEY (candidate_id, graph_id) REFERENCES entity_resolution_candidates (id, graph_id),
    FOREIGN KEY (scan_id, graph_id) REFERENCES entity_resolution_scans (id, graph_id),
    UNIQUE (id, graph_id)
);

CREATE INDEX entity_resolution_decisions_candidate
    ON entity_resolution_decisions (candidate_id, created_at, id);

CREATE FUNCTION reject_resolution_history_change() RETURNS trigger
    LANGUAGE plpgsql AS
$$
BEGIN
    RAISE EXCEPTION 'entity resolution scans and decisions are immutable'
        USING ERRCODE = 'restrict_violation';
END;
$$;

CREATE TRIGGER entity_resolution_decisions_immutable
    BEFORE UPDATE OR DELETE ON entity_resolution_decisions
    FOR EACH ROW EXECUTE FUNCTION reject_resolution_history_change();

CREATE TRIGGER entity_resolution_scans_immutable
    BEFORE UPDATE OR DELETE ON entity_resolution_scans
    FOR EACH ROW EXECUTE FUNCTION reject_resolution_history_change();

-- Identity groups -------------------------------------------------------------
-- One row per member of a logical identity group, the canonical entity
-- included (it maps to itself). Groups are flat, so they cannot contain
-- chains or cycles; every member shares the canonical entity's type.
CREATE TABLE entity_identity_aliases (
    graph_id            UUID        NOT NULL REFERENCES graph_projects (id),
    entity_id           UUID        NOT NULL,
    canonical_entity_id UUID        NOT NULL,
    entity_type         TEXT        NOT NULL CHECK (length(entity_type) BETWEEN 1 AND 64),
    -- The decision that last wrote this mapping.
    decision_id         UUID        NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL,
    updated_at          TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (graph_id, entity_id),
    FOREIGN KEY (decision_id, graph_id) REFERENCES entity_resolution_decisions (id, graph_id),
    CHECK (updated_at >= created_at)
);

CREATE INDEX entity_identity_aliases_group
    ON entity_identity_aliases (graph_id, canonical_entity_id);

-- Flatness, checked at commit so a transaction can re-point a whole group:
-- every row's canonical entity maps to itself with the same type, and an
-- entity that others map to is itself canonical.
CREATE FUNCTION check_identity_alias() RETURNS trigger
    LANGUAGE plpgsql AS
$$
DECLARE
    current_row entity_identity_aliases%ROWTYPE;
BEGIN
    SELECT * INTO current_row
      FROM entity_identity_aliases
     WHERE graph_id = NEW.graph_id AND entity_id = NEW.entity_id;
    IF NOT FOUND THEN
        RETURN NULL;
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM entity_identity_aliases c
         WHERE c.graph_id = current_row.graph_id
           AND c.entity_id = current_row.canonical_entity_id
           AND c.canonical_entity_id = current_row.canonical_entity_id
           AND c.entity_type = current_row.entity_type
    ) THEN
        RAISE EXCEPTION 'an identity alias must map to a canonical entity of the same type that maps to itself'
            USING ERRCODE = 'check_violation';
    END IF;
    IF current_row.entity_id <> current_row.canonical_entity_id AND EXISTS (
        SELECT 1 FROM entity_identity_aliases m
         WHERE m.graph_id = current_row.graph_id
           AND m.canonical_entity_id = current_row.entity_id
    ) THEN
        RAISE EXCEPTION 'an entity that other members map to must be canonical'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NULL;
END;
$$;

CREATE CONSTRAINT TRIGGER entity_identity_aliases_flat
    AFTER INSERT OR UPDATE ON entity_identity_aliases
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION check_identity_alias();
