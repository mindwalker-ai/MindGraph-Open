-- MindGraph-OSS metadata schema, version 3: AI Graph Builder jobs (M5).
--
-- Append-only: earlier migrations are never edited.
--
-- An `ai_suggest` job asks the AI Graph Builder for an ontology + mapping
-- suggestion for one analysed data source:
--   ontology_version  the active ontology the suggestion builds on (pinned)
--   mapping_json      NULL (the mapping is part of the suggestion)
--   idempotency_key   required: provider calls can cost money
--   result_json       the validated suggestion once completed (bounded;
--                     never the prompt or the raw provider response)

ALTER TABLE ingestion_jobs DROP CONSTRAINT ingestion_jobs_kind_check;
ALTER TABLE ingestion_jobs
    ADD CONSTRAINT ingestion_jobs_kind_check CHECK (kind IN ('analyze', 'ingest', 'ai_suggest'));

ALTER TABLE ingestion_jobs DROP CONSTRAINT ingestion_jobs_kind_shape;
ALTER TABLE ingestion_jobs
    ADD CONSTRAINT ingestion_jobs_kind_shape CHECK (
        (kind = 'ingest' AND ontology_version IS NOT NULL AND mapping_json IS NOT NULL)
        OR (kind = 'analyze' AND ontology_version IS NULL AND mapping_json IS NULL)
        OR (kind = 'ai_suggest' AND ontology_version IS NOT NULL AND mapping_json IS NULL
            AND idempotency_key IS NOT NULL)
    );

-- Suggestions are bounded by the application (types, properties, warnings);
-- this is a second line of defence against oversized results.
ALTER TABLE ingestion_jobs
    ADD CONSTRAINT ingestion_jobs_result_size CHECK (
        result_json IS NULL OR octet_length(result_json::text) <= 524288
    );
