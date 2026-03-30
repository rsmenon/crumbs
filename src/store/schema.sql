PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
PRAGMA synchronous = NORMAL;

-- Tasks
CREATE TABLE IF NOT EXISTS tasks (
    id          TEXT PRIMARY KEY,
    title       TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    status      TEXT NOT NULL DEFAULT 'backlog',
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    due_date    TEXT,
    due_time    TEXT,
    priority    TEXT NOT NULL DEFAULT '',
    private     INTEGER NOT NULL DEFAULT 0,
    pinned      INTEGER NOT NULL DEFAULT 0,
    archived    INTEGER NOT NULL DEFAULT 0,
    created_dir TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
CREATE INDEX IF NOT EXISTS idx_tasks_due_date ON tasks(due_date) WHERE due_date IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_tasks_created_at ON tasks(created_at);

CREATE TABLE IF NOT EXISTS task_status_history (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    status  TEXT NOT NULL,
    at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tsh_task ON task_status_history(task_id);

-- Notes
CREATE TABLE IF NOT EXISTS notes (
    id          TEXT PRIMARY KEY,
    title       TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    private     INTEGER NOT NULL DEFAULT 0,
    pinned      INTEGER NOT NULL DEFAULT 0,
    archived    INTEGER NOT NULL DEFAULT 0,
    created_dir TEXT NOT NULL DEFAULT '',
    body        TEXT NOT NULL DEFAULT ''
);

-- People
CREATE TABLE IF NOT EXISTS persons (
    slug       TEXT PRIMARY KEY,
    created_at TEXT NOT NULL,
    pinned     INTEGER NOT NULL DEFAULT 0,
    archived   INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS person_metadata (
    person_slug TEXT NOT NULL REFERENCES persons(slug) ON DELETE CASCADE ON UPDATE CASCADE,
    key         TEXT NOT NULL,
    value       TEXT NOT NULL,
    PRIMARY KEY (person_slug, key)
);

-- Tags
CREATE TABLE IF NOT EXISTS tags (
    slug       TEXT PRIMARY KEY,
    created_at TEXT NOT NULL
);

-- Agendas
CREATE TABLE IF NOT EXISTS agendas (
    id          TEXT PRIMARY KEY,
    title       TEXT NOT NULL DEFAULT '',
    person_slug TEXT NOT NULL REFERENCES persons(slug) ON DELETE CASCADE ON UPDATE CASCADE,
    date        TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    body        TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_agendas_person ON agendas(person_slug);
CREATE INDEX IF NOT EXISTS idx_agendas_date ON agendas(date);

-- Cross-references (unified junction table)
CREATE TABLE IF NOT EXISTS entity_refs (
    source_kind TEXT NOT NULL,
    source_id   TEXT NOT NULL,
    target_kind TEXT NOT NULL,
    target_id   TEXT NOT NULL,
    PRIMARY KEY (source_kind, source_id, target_kind, target_id)
);
CREATE INDEX IF NOT EXISTS idx_entity_refs_target ON entity_refs(target_kind, target_id);

-- Full-Text Search
CREATE VIRTUAL TABLE IF NOT EXISTS fts_entities USING fts5(
    entity_id UNINDEXED,
    entity_kind UNINDEXED,
    content,
    tokenize='porter unicode61'
);
