-- Add completed_at to tasks so compaction snapshots can date when each
-- task was closed. Backfills already-done tasks with their creation time.
-- Runs as a single refinery transaction.

ALTER TABLE tasks ADD COLUMN completed_at TEXT;

UPDATE tasks SET completed_at = created_at WHERE status = 'done';
