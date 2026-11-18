use std::time::Duration;

use chrono::NaiveDate;
use dandori_core::*;

fn human() -> Ctx {
    Ctx {
        actor: Actor::App,
        session_id: "app-1".into(),
    }
}

fn ai(sid: &str) -> Ctx {
    Ctx {
        actor: Actor::Cli,
        session_id: sid.into(),
    }
}

fn titles(store: &Store, status: Status) -> Vec<String> {
    store
        .list(status)
        .unwrap()
        .into_iter()
        .map(|t| t.title)
        .collect()
}

#[test]
fn queue_insert_and_move_positions() {
    let mut s = Store::open_in_memory().unwrap();
    let c = human();

    let a = s.add(&c, NewTask::new("A"), InsertAt::Bottom).unwrap().task;
    let b = s.add(&c, NewTask::new("B"), InsertAt::Bottom).unwrap().task;
    let top = s.add(&c, NewTask::new("Top"), InsertAt::Top).unwrap().task;
    let mid = s
        .add(&c, NewTask::new("Mid"), InsertAt::At(2))
        .unwrap()
        .task;
    assert_eq!(titles(&s, Status::Queued), vec!["Top", "Mid", "A", "B"]);

    s.move_to(&c, b.id, InsertAt::Before(a.id)).unwrap();
    assert_eq!(titles(&s, Status::Queued), vec!["Top", "Mid", "B", "A"]);

    s.move_to(&c, top.id, InsertAt::After(mid.id)).unwrap();
    assert_eq!(titles(&s, Status::Queued), vec!["Mid", "Top", "B", "A"]);

    assert_eq!(s.now().unwrap().unwrap().id, mid.id);
    assert_eq!(s.task_at(3).unwrap().id, b.id);
    assert_eq!(s.queue_position(a.id).unwrap(), Some(4));
}

#[test]
fn reopen_and_restore_return_to_original_position_and_handle_collision() {
    let mut s = Store::open_in_memory().unwrap();
    let c = human();

    let t1 = s.add(&c, NewTask::new("1"), InsertAt::Bottom).unwrap().task;
    let t3 = s.add(&c, NewTask::new("3"), InsertAt::Bottom).unwrap().task;
    let t2 = s
        .add(&c, NewTask::new("2"), InsertAt::After(t1.id))
        .unwrap()
        .task;

    // 2 を完了 → 1 と 3 の間に新しく 2b を入れる（2 と同じ position になる）
    s.complete(&c, t2.id).unwrap();
    let t2b = s
        .add(&c, NewTask::new("2b"), InsertAt::After(t1.id))
        .unwrap()
        .task;
    assert_eq!(t2.position, t2b.position);

    // 2 の完了を取り消すと元の位置付近に戻り、さらにその間に挿入しても壊れない（キー振り直し）
    s.reopen(&c, t2.id).unwrap();
    assert_eq!(titles(&s, Status::Queued), vec!["1", "2", "2b", "3"]);

    s.add(&c, NewTask::new("between"), InsertAt::Before(t2b.id))
        .unwrap();
    assert_eq!(
        titles(&s, Status::Queued),
        vec!["1", "2", "between", "2b", "3"]
    );

    // 削除 → 復元でも元の位置に戻る
    s.delete(&c, t3.id).unwrap();
    assert_eq!(titles(&s, Status::Deleted), vec!["3"]);
    s.restore(&c, t3.id).unwrap();
    assert_eq!(
        titles(&s, Status::Queued),
        vec!["1", "2", "between", "2b", "3"]
    );
}

#[test]
fn human_undo_and_ai_rollback_are_isolated_with_conflict_detection() {
    let mut s = Store::open_in_memory().unwrap();
    let h = human();
    let a1 = ai("s-ai1");

    let t1 = s
        .add(&h, NewTask::new("Human task"), InsertAt::Bottom)
        .unwrap()
        .task;
    let t2 = s
        .add(&a1, NewTask::new("AI task 1"), InsertAt::Bottom)
        .unwrap()
        .task;
    let _t3 = s
        .add(&a1, NewTask::new("AI task 2"), InsertAt::Bottom)
        .unwrap()
        .task;

    // 人間の Undo は AI のタスクを飛ばして t1 の作成を取り消す
    let undone = s.undo(&Scope::Human).unwrap();
    assert!(undone.summary.contains("Human task"));
    assert_eq!(titles(&s, Status::Queued), vec!["AI task 1", "AI task 2"]);

    // Redo で t1 が戻る
    s.redo(&Scope::Human).unwrap();
    assert_eq!(
        titles(&s, Status::Queued),
        vec!["Human task", "AI task 1", "AI task 2"]
    );
    assert_eq!(s.get(t1.id).unwrap().status, Status::Queued);

    // 人間が AI のタスク t2 を編集したあと、AI セッションをロールバックすると競合を検出する
    s.edit(
        &h,
        t2.id,
        TaskPatch {
            title: Some("AI task 1 (edited by human)".into()),
            ..Default::default()
        },
    )
    .unwrap();

    let err = s.rollback_session("s-ai1", false, false).unwrap_err();
    assert!(matches!(err, Error::Conflict(ref c) if c.len() == 1 && c[0].task_id == t2.id));
    // 失敗時は何も変わらない（t3 も残っている）
    assert_eq!(titles(&s, Status::Queued).len(), 3);

    // skip_conflicts=true なら競合していない t3 だけ取り消される
    let out = s.rollback_session("s-ai1", true, false).unwrap();
    assert_eq!(out.undone.len(), 1);
    assert_eq!(out.skipped.len(), 1);
    assert_eq!(
        titles(&s, Status::Queued),
        vec!["Human task", "AI task 1 (edited by human)"]
    );
}

#[test]
fn edit_memo_urls_due_and_restore_to() {
    let mut s = Store::open_in_memory().unwrap();
    let h = human();

    let op1 = s
        .add(&h, NewTask::new("Write spec"), InsertAt::Bottom)
        .unwrap();
    let id = op1.task.id;

    s.edit(
        &h,
        id,
        TaskPatch {
            memo: Some("line1".into()),
            append_memo: Some("line2".into()),
            due_date: Some(NaiveDate::from_ymd_opt(2026, 11, 20)),
            add_urls: vec![
                "https://example.com/1".into(),
                "https://example.com/1".into(),
            ],
            ..Default::default()
        },
    )
    .unwrap();

    let t = s.get(id).unwrap();
    assert_eq!(t.memo, "line1\nline2");
    assert_eq!(t.urls, vec!["https://example.com/1"]);
    assert_eq!(t.due_date, NaiveDate::from_ymd_opt(2026, 11, 20));

    // op1 の時点まで戻すと編集が取り消される
    let undone = s.restore_to(op1.operation_id).unwrap();
    assert_eq!(undone.len(), 1);
    let t = s.get(id).unwrap();
    assert_eq!(t.memo, "");
    assert!(t.urls.is_empty());
    assert_eq!(t.due_date, None);
}

#[test]
fn active_session_and_backup_rotation() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("dandori.db");
    let mut s = Store::open(&db).unwrap();

    let sid = s.start_session(Actor::Cli, Some("整理")).unwrap();
    s.set_active_session(&sid).unwrap();
    assert_eq!(
        s.active_session(Duration::from_secs(60)).unwrap(),
        Some(sid.clone())
    );
    assert_eq!(s.clear_active_session().unwrap(), Some(sid));
    assert_eq!(s.active_session(Duration::from_secs(60)).unwrap(), None);

    let bdir = dir.path().join("backups");
    let p = s.backup_to(&bdir, 2).unwrap();
    assert!(p.exists());

    // 別接続（CLI）からの書き込みで data_version が変わることを確認
    let v0 = s.data_version().unwrap();
    let mut cli_store = Store::open(&db).unwrap();
    cli_store
        .add(&ai("s-cli"), NewTask::new("From CLI"), InsertAt::Bottom)
        .unwrap();
    let v1 = s.data_version().unwrap();
    assert_ne!(v0, v1);
}
