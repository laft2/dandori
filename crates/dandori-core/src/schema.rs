//! スキーマとマイグレーション。`PRAGMA user_version` でバージョンを管理する。

use rusqlite::{Connection, TransactionBehavior};

use crate::error::{Error, Result};

const MIGRATIONS: &[&str] = &[
    // v1
    r#"
CREATE TABLE tasks (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    title        TEXT    NOT NULL CHECK (length(trim(title)) > 0),
    memo         TEXT    NOT NULL DEFAULT '',
    due_date     TEXT,
    urls         TEXT    NOT NULL DEFAULT '[]',
    status       TEXT    NOT NULL DEFAULT 'queued'
                 CHECK (status IN ('queued','on_hold','done','deleted')),
    source       TEXT    NOT NULL DEFAULT 'manual'
                 CHECK (source IN ('manual','recurring')),
    position     TEXT    NOT NULL,
    created_at   TEXT    NOT NULL,
    updated_at   TEXT    NOT NULL,
    completed_at TEXT
);
CREATE INDEX idx_tasks_status_position ON tasks(status, position);
CREATE INDEX idx_tasks_completed_at    ON tasks(completed_at);

CREATE TABLE sessions (
    id         TEXT PRIMARY KEY,
    actor      TEXT NOT NULL CHECK (actor IN ('app','cli','mcp')),
    label      TEXT,
    started_at TEXT NOT NULL
);

CREATE TABLE operations (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    op_type        TEXT    NOT NULL,
    summary        TEXT    NOT NULL,
    actor          TEXT    NOT NULL CHECK (actor IN ('app','cli','mcp')),
    session_id     TEXT    REFERENCES sessions(id),
    created_at     TEXT    NOT NULL,
    undone_at      TEXT,
    undo_seq       INTEGER,
    redo_discarded INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_operations_session ON operations(session_id);
CREATE INDEX idx_operations_actor   ON operations(actor);

CREATE TABLE operation_changes (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_id INTEGER NOT NULL REFERENCES operations(id),
    task_id      INTEGER NOT NULL,
    before       TEXT,
    after        TEXT    NOT NULL
);
CREATE INDEX idx_changes_operation ON operation_changes(operation_id);
CREATE INDEX idx_changes_task      ON operation_changes(task_id);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#,
];

pub const LATEST_VERSION: i64 = MIGRATIONS.len() as i64;

pub fn user_version(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

/// 未適用のマイグレーションを順に適用する。
/// アプリと CLI が同時に起動しても安全なように、IMMEDIATE トランザクションの中でバージョンを読み直す。
pub fn migrate(conn: &mut Connection) -> Result<()> {
    loop {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = user_version(&tx)?;
        if current > LATEST_VERSION {
            return Err(Error::SchemaTooNew(current));
        }
        if current == LATEST_VERSION {
            return Ok(());
        }
        tx.execute_batch(MIGRATIONS[current as usize])?;
        tx.pragma_update(None, "user_version", current + 1)?;
        tx.commit()?;
    }
}
