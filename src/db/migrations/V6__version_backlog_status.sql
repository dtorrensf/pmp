-- Add the `backlog` status to versions: new versions are planned there and
-- move Backlog -> Open -> Released. SQLite cannot alter a CHECK constraint, so
-- `versions` is rebuilt with the new CHECK and default.
--
-- With foreign_keys ON, dropping a parent table performs an implicit DELETE
-- that cascades into its children. `tasks` (version_id) and
-- `version_summaries` (version_id) reference `versions`, and `tasks` is itself
-- referenced by `task_dependencies` and `task_notes`, so every child is staged
-- in a temporary table and restored after the rebuild. Existing versions keep
-- their current status.
--
-- Runs as a single refinery transaction.

CREATE TABLE tmp_version_summaries AS SELECT * FROM version_summaries;
CREATE TABLE tmp_task_dependencies AS SELECT * FROM task_dependencies;
CREATE TABLE tmp_task_notes AS SELECT * FROM task_notes;
DROP TABLE version_summaries;
DROP TABLE task_notes;
DROP TABLE task_dependencies;

CREATE TABLE tmp_tasks AS SELECT * FROM tasks;
DROP TABLE tasks;

CREATE TABLE versions_new (
    id          INTEGER PRIMARY KEY,
    project_id  INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    status      TEXT NOT NULL DEFAULT 'backlog'
                CHECK (status IN ('backlog','open','released')),
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (project_id, name)
);
INSERT INTO versions_new (id, project_id, name, description, status, created_at)
SELECT id, project_id, name, description, status, created_at FROM versions;
DROP TABLE versions;
ALTER TABLE versions_new RENAME TO versions;
CREATE INDEX idx_versions_project ON versions(project_id);

CREATE TABLE version_summaries (
    id            INTEGER PRIMARY KEY,
    version_id    INTEGER NOT NULL REFERENCES versions(id) ON DELETE CASCADE,
    summary       TEXT NOT NULL,
    snapshot_json TEXT NOT NULL,
    generated_at  TEXT NOT NULL DEFAULT (datetime('now')),
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (version_id)
);
INSERT INTO version_summaries (id, version_id, summary, snapshot_json, generated_at, created_at)
SELECT id, version_id, summary, snapshot_json, generated_at, created_at FROM tmp_version_summaries;
DROP TABLE tmp_version_summaries;
CREATE INDEX idx_version_summaries_version ON version_summaries(version_id);

CREATE TABLE tasks (
    id           INTEGER PRIMARY KEY,
    project_id   INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    version_id   INTEGER NOT NULL REFERENCES versions(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    description  TEXT NOT NULL DEFAULT '',
    status       TEXT NOT NULL CHECK (status IN ('todo','in_progress','done')),
    priority     INTEGER NOT NULL CHECK (priority BETWEEN 1 AND 3),
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at TEXT
);
INSERT INTO tasks (id, project_id, version_id, name, description, status, priority, created_at, completed_at)
SELECT id, project_id, version_id, name, description, status, priority, created_at, completed_at
FROM tmp_tasks;
DROP TABLE tmp_tasks;

CREATE INDEX idx_tasks_project ON tasks(project_id);
CREATE INDEX idx_tasks_status  ON tasks(status);
CREATE INDEX idx_tasks_version ON tasks(version_id);

CREATE TABLE task_dependencies (
    task_id       INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    depends_on_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    PRIMARY KEY (task_id, depends_on_id),
    CHECK (task_id <> depends_on_id)
);
INSERT INTO task_dependencies (task_id, depends_on_id)
SELECT task_id, depends_on_id FROM tmp_task_dependencies;
DROP TABLE tmp_task_dependencies;
CREATE INDEX idx_deps_depends ON task_dependencies(depends_on_id);

CREATE TABLE task_notes (
    id INTEGER PRIMARY KEY,
    task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
INSERT INTO task_notes (id, task_id, content, created_at)
SELECT id, task_id, content, created_at FROM tmp_task_notes;
DROP TABLE tmp_task_notes;
CREATE INDEX idx_task_notes_task_id ON task_notes(task_id);
