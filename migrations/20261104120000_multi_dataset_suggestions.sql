-- MindGraph-OSS metadata schema, version 4: AI suggestions over several
-- data sources.
--
-- Append-only: earlier migrations are never edited.
--
-- An `ai_suggest` job may model up to 10 analysed data sources of its graph
-- together. `ingestion_jobs.data_source_id` stays the first of them (so
-- every existing query, foreign key and index keeps working); this table
-- lists all of them, in the client's order, for `ai_suggest` jobs only.
-- Analysis and ingestion jobs read exactly one data source and have no rows
-- here.

ALTER TABLE ingestion_jobs
    ADD CONSTRAINT ingestion_jobs_id_graph UNIQUE (id, graph_id);

CREATE TABLE ingestion_job_data_sources (
    job_id         UUID     NOT NULL,
    graph_id       UUID     NOT NULL,
    position       SMALLINT NOT NULL CHECK (position BETWEEN 0 AND 9),
    data_source_id UUID     NOT NULL,
    PRIMARY KEY (job_id, position),
    UNIQUE (job_id, data_source_id),
    -- Same graph as the job, and a data source of that graph.
    FOREIGN KEY (job_id, graph_id) REFERENCES ingestion_jobs (id, graph_id) ON DELETE CASCADE,
    FOREIGN KEY (data_source_id, graph_id) REFERENCES data_sources (id, graph_id)
);

CREATE INDEX ingestion_job_data_sources_source ON ingestion_job_data_sources (data_source_id);
