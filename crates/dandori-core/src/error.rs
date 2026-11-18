use crate::model::{Conflict, Status, TaskId};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("タスク #{0} が見つかりません")]
    NotFound(TaskId),
    #[error("タスク #{0} はキューにありません")]
    NotInQueue(TaskId),
    #[error("タスク #{id} は「{status}」のため、この操作はできません")]
    InvalidState { id: TaskId, status: Status },
    #[error("{0}")]
    InvalidInput(String),
    #[error("変更はありません")]
    NoChange,
    #[error("取り消せる操作がありません")]
    NothingToUndo,
    #[error("やり直せる操作がありません")]
    NothingToRedo,
    #[error("競合のため取り消せません（{}）", format_conflicts(.0))]
    Conflict(Vec<Conflict>),
    #[error("操作 #{0} が見つかりません")]
    OperationNotFound(i64),
    #[error(
        "DBのスキーマ (v{0}) がこのバージョンより新しいため開けません。Dandori を更新してください"
    )]
    SchemaTooNew(i64),
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

fn format_conflicts(c: &[Conflict]) -> String {
    c.iter()
        .map(|c| format!("#{} {} は後から変更されています", c.task_id, c.title))
        .collect::<Vec<_>>()
        .join(", ")
}

impl Error {
    /// CLI の JSON 出力などで使う機械向けのエラーコード
    pub fn code(&self) -> &'static str {
        match self {
            Error::NotFound(_) | Error::OperationNotFound(_) => "not_found",
            Error::NotInQueue(_) | Error::InvalidState { .. } => "invalid_state",
            Error::InvalidInput(_) => "invalid_input",
            Error::NoChange => "no_change",
            Error::NothingToUndo => "nothing_to_undo",
            Error::NothingToRedo => "nothing_to_redo",
            Error::Conflict(_) => "conflict",
            Error::SchemaTooNew(_) => "schema_too_new",
            Error::Db(_) if self.is_busy() => "busy",
            Error::Db(_) => "db",
            Error::Json(_) => "json",
            Error::Io(_) => "io",
        }
    }

    /// 他のプロセスがロックを保持していてタイムアウトした場合
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            Error::Db(rusqlite::Error::SqliteFailure(e, _))
                if matches!(e.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
        )
    }
}

pub type Result<T> = std::result::Result<T, Error>;
