//! Dandori core: すべてのロジック（タスク操作・操作ログ・Undo）をここに集約する。
//! アプリ（Tauri）・CLI・MCP はこのクレートを呼ぶだけの薄い入り口とする。

mod error;
mod model;
pub mod paths;
pub mod position;
mod schema;
mod store;

pub use error::{Error, Result};
pub use model::*;
pub use store::{OpResult, Store, month_range_utc};
