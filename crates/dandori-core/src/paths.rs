//! DBファイルの場所の解決。アプリと CLI で同じロジックを使う。

use std::path::{Path, PathBuf};

pub const DB_ENV: &str = "DANDORI_DB";
pub const APP_DIR_NAME: &str = "Dandori";
pub const DB_FILE_NAME: &str = "dandori.db";

/// 環境変数 `DANDORI_DB` があればそれを、なければ OS ごとのアプリデータフォルダを使う。
/// - macOS: `~/Library/Application Support/Dandori/dandori.db`
/// - Windows: `%APPDATA%\Dandori\dandori.db`
pub fn default_db_path() -> PathBuf {
    if let Some(p) = std::env::var_os(DB_ENV).filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_DIR_NAME)
        .join(DB_FILE_NAME)
}

/// バックアップの保存先（DBファイルと同じフォルダの `backups/`）
pub fn backup_dir(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .map_or_else(|| PathBuf::from("backups"), |p| p.join("backups"))
}
