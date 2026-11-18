use std::path::Path;
use std::process::Command;

use serde_json::Value;

fn dan(db: &Path, args: &[&str]) -> (i32, String, String) {
    let bin = env!("CARGO_BIN_EXE_dan");
    let out = Command::new(bin)
        .env("DANDORI_DB", db)
        .env_remove("DANDORI_SESSION")
        .args(args)
        .output()
        .expect("failed to execute dan");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

#[test]
fn cli_end_to_end_session_rollback_and_report() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");

    // 1. セッション開始
    let (code, out, _) = dan(&db, &["session", "start", "--label", "整理テスト"]);
    assert_eq!(code, 0);
    let sid = out.trim().to_string();
    assert!(sid.starts_with("s-"));

    // 2. タスク追加（デフォルトは末尾、--top で先頭、--at で任意位置）
    let (code, _, _) = dan(
        &db,
        &[
            "add",
            "タスクA",
            "--due",
            "2026-11-20",
            "--url",
            "https://example.com/a",
        ],
    );
    assert_eq!(code, 0);
    let (code, _, _) = dan(&db, &["add", "タスクB", "--top"]);
    assert_eq!(code, 0);
    let (code, _, _) = dan(&db, &["add", "タスクC", "--at", "2"]);
    assert_eq!(code, 0);

    // 3. JSON 一覧で並び順（B -> C -> A）を確認
    let (code, out, _) = dan(&db, &["list", "--json"]);
    assert_eq!(code, 0);
    let list: Vec<Value> = serde_json::from_str(&out).unwrap();
    let titles: Vec<&str> = list.iter().map(|v| v["title"].as_str().unwrap()).collect();
    assert_eq!(titles, vec!["タスクB", "タスクC", "タスクA"]);

    // 4. 先頭の完了（引数なし dan done）と report --month this
    let (code, _, _) = dan(&db, &["done"]);
    assert_eq!(code, 0);
    let (code, rep, _) = dan(&db, &["report", "--month", "this"]);
    assert_eq!(code, 0);
    assert!(rep.contains("タスクB"));

    // 5. Undo で直前の完了を取り消す
    let (code, _, _) = dan(&db, &["undo"]);
    assert_eq!(code, 0);
    let (code, now_out, _) = dan(&db, &["now", "--json"]);
    assert_eq!(code, 0);
    let now_val: Value = serde_json::from_str(&now_out).unwrap();
    assert_eq!(now_val["title"].as_str(), Some("タスクB"));

    // 6. セッション全体をロールバックすると追加した3件もすべて取り消され、キューが空になる
    let (code, _, _) = dan(&db, &["rollback", "--session", &sid]);
    assert_eq!(code, 0);
    let (code, list_out, _) = dan(&db, &["list", "--json"]);
    assert_eq!(code, 0);
    let empty: Vec<Value> = serde_json::from_str(&list_out).unwrap();
    assert!(empty.is_empty());

    let (code, _, _) = dan(&db, &["session", "end"]);
    assert_eq!(code, 0);
}
