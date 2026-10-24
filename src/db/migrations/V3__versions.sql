-- Introduce versions between projects and tasks.
-- Runs as a single refinery transaction.

CREATE TABLE versions (
    id          INTEGER PRIMARY KEY,
    project_id  INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    status      TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open','released')),
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (project_id, name)
);
CREATE INDEX idx_versions_project ON versions(project_id);

-- Every existing project gets a legacy "v0" version that adopts its tasks.
INSERT INTO versions (project_id, name)
SELECT id, 'v0' FROM projects;

ALTER TABLE tasks ADD COLUMN version_id INTEGER REFERENCES versions(id) ON DELETE CASCADE;

UPDATE tasks
SET version_id = (
    SELECT v.id FROM versions v
    WHERE v.project_id = tasks.project_id AND v.name = 'v0'
);

-- SQLite cannot add NOT NULL to an existing column, so rebuild `tasks`.
-- Dropping `tasks` would cascade into its children while foreign_keys is ON,
-- so the children are staged in temporary tables and restored afterwards.
CREATE TABLE tmp_task_dependencies AS SELECT * FROM task_dependencies;
CREATE TABLE tmp_task_notes AS SELECT * FROM task_notes;
DROP TABLE task_notes;
DROP TABLE task_dependencies;

CREATE TABLE tasks_new (
    id          INTEGER PRIMARY KEY,
    project_id  INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    version_id  INTEGER NOT NULL REFERENCES versions(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    status      TEXT NOT NULL CHECK (status IN ('todo','in_progress','done')),
    priority    INTEGER NOT NULL CHECK (priority BETWEEN 1 AND 3),
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
INSERT INTO tasks_new (id, project_id, version_id, name, description, status, priority, created_at)
SELECT id, project_id, version_id, name, description, status, priority, created_at FROM tasks;
DROP TABLE tasks;
ALTER TABLE tasks_new RENAME TO tasks;

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
