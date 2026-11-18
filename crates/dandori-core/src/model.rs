use std::fmt;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

pub type TaskId = i64;
pub type OpId = i64;

#[derive(Debug, thiserror::Error)]
#[error("unknown value: {0}")]
pub struct UnknownValue(pub String);

macro_rules! str_enum {
    ($name:ident { $($variant:ident => $s:literal / $label:literal),* $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),* }

        impl $name {
            pub fn as_str(&self) -> &'static str {
                match self { $($name::$variant => $s),* }
            }
            /// 画面表示用の日本語ラベル
            pub fn label(&self) -> &'static str {
                match self { $($name::$variant => $label),* }
            }
            pub fn parse(s: &str) -> Result<Self, UnknownValue> {
                match s { $($s => Ok($name::$variant),)* _ => Err(UnknownValue(s.to_string())) }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(self.label()) }
        }
    };
}

str_enum!(Status {
    Queued => "queued" / "キュー",
    OnHold => "on_hold" / "保留",
    Done => "done" / "完了",
    Deleted => "deleted" / "削除済み",
});

str_enum!(Source {
    Manual => "manual" / "手動",
    Recurring => "recurring" / "定期",
});

str_enum!(Actor {
    App => "app" / "自分",
    Cli => "cli" / "CLI",
    Mcp => "mcp" / "MCP",
});

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub memo: String,
    pub due_date: Option<NaiveDate>,
    pub urls: Vec<String>,
    pub status: Status,
    pub source: Source,
    /// 並び順のキー（fractional indexing）。辞書順で比較する
    pub position: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl Task {
    /// `updated_at` 以外の項目が一致するか（Undo の競合判定に使う）
    pub fn same_content(&self, other: &Task) -> bool {
        let mut a = self.clone();
        a.updated_at = other.updated_at;
        &a == other
    }
}

#[derive(Debug, Clone)]
pub struct NewTask {
    pub title: String,
    pub memo: String,
    pub due_date: Option<NaiveDate>,
    pub urls: Vec<String>,
    /// `Queued` または `OnHold`
    pub status: Status,
    pub source: Source,
}

impl NewTask {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            memo: String::new(),
            due_date: None,
            urls: Vec::new(),
            status: Status::Queued,
            source: Source::Manual,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub memo: Option<String>,
    pub append_memo: Option<String>,
    /// `Some(None)` で期限を消す
    pub due_date: Option<Option<NaiveDate>>,
    pub add_urls: Vec<String>,
    pub remove_urls: Vec<String>,
    pub clear_urls: bool,
}

/// キューのどこに入れるか
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertAt {
    Top,
    Bottom,
    /// 1 始まりの位置
    At(usize),
    Before(TaskId),
    After(TaskId),
}

/// 誰が・どのセッションで操作したか
#[derive(Debug, Clone)]
pub struct Ctx {
    pub actor: Actor,
    pub session_id: String,
}

/// Undo / Redo の範囲
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// アプリ上の人間の操作（actor = app）
    Human,
    /// 特定のセッション（主に AI）
    Session(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Operation {
    pub id: OpId,
    pub op_type: String,
    pub summary: String,
    pub actor: Actor,
    pub session_id: Option<String>,
    pub session_label: Option<String>,
    pub created_at: DateTime<Utc>,
    pub undone: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub actor: Actor,
    pub label: Option<String>,
    pub started_at: DateTime<Utc>,
    pub op_count: i64,
    pub active_op_count: i64,
    pub last_op_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conflict {
    pub task_id: TaskId,
    pub title: String,
    pub operation_id: OpId,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UndoOutcome {
    pub undone: Vec<Operation>,
    pub skipped: Vec<Conflict>,
}
