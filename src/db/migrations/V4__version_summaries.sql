-- Persist per-version summaries for compaction.
-- Runs as a single refinery transaction.

CREATE TABLE version_summaries (
    id            INTEGER PRIMARY KEY,
    version_id    INTEGER NOT NULL REFERENCES versions(id) ON DELETE CASCADE,
    summary       TEXT NOT NULL,
    snapshot_json TEXT NOT NULL,
    generated_at  TEXT NOT NULL DEFAULT (datetime('now')),
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (version_id)
);
CREATE INDEX idx_version_summaries_version ON version_summaries(version_id);
