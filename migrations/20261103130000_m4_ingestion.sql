-- MindGraph Open metadata schema, version 2: dataset ingestion (M4).
--
-- Append-only: this migration extends the M3 baseline tables `data_sources`
-- and `ingestion_jobs`; the M3 migration is never edited.
--
-- M3 offered no way to create data sources or jobs, so both tables are empty
-- when this migration runs and new NOT NULL columns need no defaults.
--
-- Source-file bytes are never stored here: `storage_key` points into the
-- SourceStore (the local data directory in v0.1). `profile_json` holds a
-- bounded dataset profile (at most 20 sample rows, 5 sample values per field).

-- Data sources ------------------------------------------------------------------

ALTER TABLE data_sources
    ADD COLUMN storage_key    TEXT   NOT NULL CHECK (length(storage_key) BETWEEN 1 AND 512),
    ADD COLUMN size_bytes     BIGINT NOT NULL CHECK (size_bytes >= 0),
    ADD COLUMN content_sha256 TEXT   NOT NULL CHECK (content_sha256 ~ '^[0-9a-f]{64}$'),
    ADD COLUMN record_count   BIGINT CHECK (record_count >= 0),
    ADD COLUMN profile_json   JSONB;

ALTER TABLE data_sources
    ADD CONSTRAINT data_sources_filename_length CHECK (length(filename) <= 255),
    -- A data source has a profile exactly when its analysis succeeded.
    ADD CONSTRAINT data_sources_profile_matches_status CHECK (
        (status = 'ready') = (profile_json IS NOT NULL AND record_count IS NOT NULL)
    );

-- Ingestion jobs ------------------------------------------------------------------

ALTER TABLE ingestion_jobs
    ADD COLUMN kind             TEXT    NOT NULL CHECK (kind IN ('analyze', 'ingest')),
    -- The ontology version an ingestion job was validated against. Workers
    -- load exactly this version, never "whatever is active now".
    ADD COLUMN ontology_version INTEGER,
    ADD COLUMN mapping_json     JSONB,
    -- Bounded counters only; never record contents.
    ADD COLUMN result_json      JSONB;

ALTER TABLE ingestion_jobs
    ADD CONSTRAINT ingestion_jobs_pinned_ontology_fk
        FOREIGN KEY (graph_id, ontology_version)
        REFERENCES ontology_versions (graph_id, version),
    ADD CONSTRAINT ingestion_jobs_kind_shape CHECK (
        (kind = 'ingest' AND ontology_version IS NOT NULL AND mapping_json IS NOT NULL)
        OR (kind = 'analyze' AND ontology_version IS NULL AND mapping_json IS NULL)
    ),
    ADD CONSTRAINT ingestion_jobs_error_pair CHECK ((error_code IS NULL) = (error_message IS NULL)),
    ADD CONSTRAINT ingestion_jobs_failed_has_error CHECK ((status = 'failed') = (error_code IS NOT NULL)),
    ADD CONSTRAINT ingestion_jobs_completed_progress CHECK (status <> 'completed' OR progress = 1),
    ADD CONSTRAINT ingestion_jobs_result_when_completed CHECK (result_json IS NULL OR status = 'completed'),
    ADD CONSTRAINT ingestion_jobs_idempotency_key_length
        CHECK (idempotency_key IS NULL OR length(idempotency_key) BETWEEN 1 AND 255),
    ADD CONSTRAINT ingestion_jobs_error_message_length
        CHECK (error_message IS NULL OR length(error_message) <= 1000);

-- Serves restart recovery: "fail every queued or running job".
CREATE INDEX ingestion_jobs_incomplete
    ON ingestion_jobs (status)
    WHERE status IN ('queued', 'running');
