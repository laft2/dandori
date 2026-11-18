use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Local, NaiveDate, SecondsFormat, TimeZone, Timelike, Utc};
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::*;
use crate::{position, schema};

const TASK_COLS: &str = "id, title, memo, due_date, urls, status, source, position, created_at, updated_at, completed_at";
const OP_SELECT: &str = "SELECT o.id, o.op_type, o.summary, o.actor, o.session_id, s.label, o.created_at, \
     o.undo_seq IS NOT NULL FROM operations o LEFT JOIN sessions s ON s.id = o.session_id";
const ACTIVE_SESSION_KEY: &str = "cli_active_session";

/// 変更操作の結果
#[derive(Debug, Clone, Serialize)]
pub struct OpResult {
    pub operation_id: OpId,
    /// 操作の主な対象のタスク（変更後）
    pub task: Task,
}

pub struct Store {
    conn: Connection,
    path: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// 低レベルのヘルパー
// ---------------------------------------------------------------------------

/// ミリ秒に丸めた現在時刻（DBに保存する精度とそろえ、スナップショットの比較を安定させる）
fn now() -> DateTime<Utc> {
    let n = Utc::now();
    n.with_nanosecond(n.nanosecond() / 1_000_000 * 1_000_000)
        .unwrap_or(n)
}

fn ts(t: &DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn conv_err<E: std::error::Error + Send + Sync + 'static>(idx: usize, e: E) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, Box::new(e))
}

fn parse_ts(idx: usize, s: &str) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| conv_err(idx, e))
}

fn task_from_row(r: &Row) -> rusqlite::Result<Task> {
    let due: Option<String> = r.get(3)?;
    let urls: String = r.get(4)?;
    let status: String = r.get(5)?;
    let source: String = r.get(6)?;
    let created: String = r.get(8)?;
    let updated: String = r.get(9)?;
    let completed: Option<String> = r.get(10)?;
    Ok(Task {
        id: r.get(0)?,
        title: r.get(1)?,
        memo: r.get(2)?,
        due_date: due
            .map(|d| NaiveDate::parse_from_str(&d, "%Y-%m-%d"))
            .transpose()
            .map_err(|e| conv_err(3, e))?,
        urls: serde_json::from_str(&urls).map_err(|e| conv_err(4, e))?,
        status: Status::parse(&status).map_err(|e| conv_err(5, e))?,
        source: Source::parse(&source).map_err(|e| conv_err(6, e))?,
        position: r.get(7)?,
        created_at: parse_ts(8, &created)?,
        updated_at: parse_ts(9, &updated)?,
        completed_at: completed.map(|c| parse_ts(10, &c)).transpose()?,
    })
}

fn op_from_row(r: &Row) -> rusqlite::Result<Operation> {
    let actor: String = r.get(3)?;
    let created: String = r.get(6)?;
    Ok(Operation {
        id: r.get(0)?,
        op_type: r.get(1)?,
        summary: r.get(2)?,
        actor: Actor::parse(&actor).map_err(|e| conv_err(3, e))?,
        session_id: r.get(4)?,
        session_label: r.get(5)?,
        created_at: parse_ts(6, &created)?,
        undone: r.get(7)?,
    })
}

fn get_task(conn: &Connection, id: TaskId) -> Result<Task> {
    conn.query_row(
        &format!("SELECT {TASK_COLS} FROM tasks WHERE id = ?1"),
        [id],
        task_from_row,
    )
    .optional()?
    .ok_or(Error::NotFound(id))
}

fn query_tasks(conn: &Connection, sql_tail: &str, p: impl rusqlite::Params) -> Result<Vec<Task>> {
    let mut stmt = conn.prepare(&format!("SELECT {TASK_COLS} FROM tasks {sql_tail}"))?;
    let rows = stmt.query_map(p, task_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn write_task(conn: &Connection, t: &Task) -> Result<()> {
    conn.execute(
        "UPDATE tasks SET title = ?2, memo = ?3, due_date = ?4, urls = ?5, status = ?6, source = ?7, \
         position = ?8, created_at = ?9, updated_at = ?10, completed_at = ?11 WHERE id = ?1",
        params![
            t.id,
            t.title,
            t.memo,
            t.due_date.map(|d| d.format("%Y-%m-%d").to_string()),
            serde_json::to_string(&t.urls)?,
            t.status.as_str(),
            t.source.as_str(),
            t.position,
            ts(&t.created_at),
            ts(&t.updated_at),
            t.completed_at.as_ref().map(ts),
        ],
    )?;
    Ok(())
}

fn get_operation(conn: &Connection, id: OpId) -> Result<Operation> {
    conn.query_row(&format!("{OP_SELECT} WHERE o.id = ?1"), [id], op_from_row)
        .optional()?
        .ok_or(Error::OperationNotFound(id))
}

fn label(id: TaskId, title: &str) -> String {
    const MAX: usize = 30;
    let short: String = title.chars().take(MAX).collect();
    let ellipsis = if title.chars().count() > MAX {
        "…"
    } else {
        ""
    };
    format!("#{id} {short}{ellipsis}")
}

/// 1回の操作で変更されたタスクの (変更前, 変更後)
#[derive(Default)]
struct Changes(Vec<(Option<Task>, Task)>);

impl Changes {
    fn record(&mut self, before: Option<Task>, after: Task) {
        if let Some(e) = self.0.iter_mut().find(|(_, a)| a.id == after.id) {
            e.1 = after;
        } else {
            self.0.push((before, after));
        }
    }
}

// ---------------------------------------------------------------------------
// 並び順
// ---------------------------------------------------------------------------

fn queued_positions(conn: &Connection, exclude: Option<TaskId>) -> Result<Vec<(TaskId, String)>> {
    let mut stmt = conn
        .prepare("SELECT id, position FROM tasks WHERE status = 'queued' ORDER BY position, id")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, TaskId>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter(|(id, _)| Some(*id) != exclude)
        .collect())
}

fn resolve_index(list: &[(TaskId, String)], at: &InsertAt) -> Result<usize> {
    let find = |id: TaskId| {
        list.iter()
            .position(|(i, _)| *i == id)
            .ok_or(Error::NotInQueue(id))
    };
    Ok(match at {
        InsertAt::Top => 0,
        InsertAt::Bottom => list.len(),
        InsertAt::At(0) => return Err(Error::InvalidInput("位置は1以上で指定してください".into())),
        InsertAt::At(n) => (n - 1).min(list.len()),
        InsertAt::Before(id) => find(*id)?,
        InsertAt::After(id) => find(*id)? + 1,
    })
}

fn neighbors(list: &[(TaskId, String)], idx: usize) -> (String, Option<String>) {
    let lower = if idx > 0 {
        list[idx - 1].1.clone()
    } else {
        String::new()
    };
    (lower, list.get(idx).map(|(_, p)| p.clone()))
}

/// `at` の位置に入れるための新しいキーを返す。`exclude` は移動中のタスク。
/// 前後のキーが重複していて間に入れられない場合は、キュー全体のキーを振り直す。
fn position_for(
    conn: &Connection,
    at: &InsertAt,
    exclude: Option<TaskId>,
    ch: &mut Changes,
) -> Result<String> {
    let list = queued_positions(conn, exclude)?;
    let idx = resolve_index(&list, at)?;
    let (mut lower, mut upper) = neighbors(&list, idx);
    if upper.as_deref().is_some_and(|u| lower.as_str() >= u) {
        renormalize(conn, &list, ch)?;
        let list = queued_positions(conn, exclude)?;
        (lower, upper) = neighbors(&list, idx);
    }
    Ok(position::key_between(&lower, upper.as_deref()))
}

fn renormalize(conn: &Connection, list: &[(TaskId, String)], ch: &mut Changes) -> Result<()> {
    let ts_now = now();
    for ((id, _), key) in list.iter().zip(position::even_keys(list.len())) {
        let before = get_task(conn, *id)?;
        let mut t = before.clone();
        t.position = key;
        t.updated_at = ts_now;
        write_task(conn, &t)?;
        ch.record(Some(before), get_task(conn, *id)?);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Undo / Redo の部品
// ---------------------------------------------------------------------------

struct ChangeRow {
    task_id: TaskId,
    before: Option<Task>,
    after: Task,
}

impl ChangeRow {
    /// 取り消した後の状態。新規作成の取り消しは「削除済み」にする
    fn undo_target(&self) -> Task {
        self.before.clone().unwrap_or_else(|| {
            let mut t = self.after.clone();
            t.status = Status::Deleted;
            t
        })
    }
}

fn load_changes(conn: &Connection, op_id: OpId) -> Result<Vec<ChangeRow>> {
    let mut stmt = conn.prepare(
        "SELECT task_id, before, after FROM operation_changes WHERE operation_id = ?1 ORDER BY id",
    )?;
    let raw = stmt
        .query_map([op_id], |r| {
            Ok((
                r.get::<_, TaskId>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    raw.into_iter()
        .map(|(task_id, b, a)| {
            Ok(ChangeRow {
                task_id,
                before: b.map(|b| serde_json::from_str(&b)).transpose()?,
                after: serde_json::from_str(&a)?,
            })
        })
        .collect()
}

/// 操作を取り消す。現在の状態が操作直後の状態と一致しないタスクがあれば、何も変更せず競合を返す。
fn apply_undo(conn: &Connection, op_id: OpId) -> Result<std::result::Result<(), Vec<Conflict>>> {
    let changes = load_changes(conn, op_id)?;
    let mut conflicts = Vec::new();
    for c in &changes {
        let cur = get_task(conn, c.task_id)?;
        if !cur.same_content(&c.after) {
            conflicts.push(Conflict {
                task_id: c.task_id,
                title: cur.title,
                operation_id: op_id,
            });
        }
    }
    if !conflicts.is_empty() {
        return Ok(Err(conflicts));
    }
    let t_now = now();
    for c in changes.iter().rev() {
        let mut t = c.undo_target();
        t.updated_at = t_now;
        write_task(conn, &t)?;
    }
    conn.execute(
        "UPDATE operations SET undo_seq = (SELECT COALESCE(MAX(undo_seq), 0) + 1 FROM operations), \
         undone_at = ?2 WHERE id = ?1",
        params![op_id, ts(&t_now)],
    )?;
    Ok(Ok(()))
}

fn apply_redo(conn: &Connection, op_id: OpId) -> Result<std::result::Result<(), Vec<Conflict>>> {
    let changes = load_changes(conn, op_id)?;
    let mut conflicts = Vec::new();
    for c in &changes {
        let cur = get_task(conn, c.task_id)?;
        if !cur.same_content(&c.undo_target()) {
            conflicts.push(Conflict {
                task_id: c.task_id,
                title: cur.title,
                operation_id: op_id,
            });
        }
    }
    if !conflicts.is_empty() {
        return Ok(Err(conflicts));
    }
    let t_now = now();
    for c in &changes {
        let mut t = c.after.clone();
        t.updated_at = t_now;
        write_task(conn, &t)?;
    }
    conn.execute(
        "UPDATE operations SET undo_seq = NULL, undone_at = NULL WHERE id = ?1",
        [op_id],
    )?;
    Ok(Ok(()))
}

/// Undo / Redo の範囲を SQL の条件とパラメータに変換する
fn scope_sql(scope: &Scope) -> (&'static str, String) {
    match scope {
        Scope::Human => ("o.actor = ?1", Actor::App.as_str().to_string()),
        Scope::Session(id) => ("o.session_id = ?1", id.clone()),
    }
}

#[derive(Serialize, Deserialize)]
struct ActiveSession {
    id: String,
    last_used: DateTime<Utc>,
}

/// ローカル時刻での月の範囲を UTC で返す（`[start, end)`）
pub fn month_range_utc(year: i32, month: u32) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    let invalid = || Error::InvalidInput(format!("不正な年月です: {year}-{month:02}"));
    let start = NaiveDate::from_ymd_opt(year, month, 1).ok_or_else(invalid)?;
    let (ny, nm) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let end = NaiveDate::from_ymd_opt(ny, nm, 1).ok_or_else(invalid)?;
    let to_utc = |d: NaiveDate| {
        Local
            .from_local_datetime(&d.and_hms_opt(0, 0, 0).expect("midnight"))
            .earliest()
            .map(|t| t.with_timezone(&Utc))
            .ok_or_else(invalid)
    };
    Ok((to_utc(start)?, to_utc(end)?))
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

impl Store {
    /// DBを開く（なければ作る）。未適用のマイグレーションも適用する。
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        Self::init(conn, Some(path.to_path_buf()))
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?, None)
    }

    fn init(mut conn: Connection, path: Option<PathBuf>) -> Result<Self> {
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")?;
        schema::migrate(&mut conn)?;
        Ok(Self { conn, path })
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn schema_version(&self) -> Result<i64> {
        schema::user_version(&self.conn)
    }

    /// 他の接続（CLI など）から書き込まれると値が変わる。アプリの変更検知に使う
    pub fn data_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("PRAGMA data_version", [], |r| r.get(0))?)
    }

    // ----- 参照 -----------------------------------------------------------

    pub fn get(&self, id: TaskId) -> Result<Task> {
        get_task(&self.conn, id)
    }

    /// 状態ごとの一覧。キューは並び順、それ以外は新しい順
    pub fn list(&self, status: Status) -> Result<Vec<Task>> {
        let order = match status {
            Status::Queued => "ORDER BY position, id",
            Status::OnHold | Status::Deleted => "ORDER BY updated_at DESC, id DESC",
            Status::Done => "ORDER BY completed_at DESC, id DESC",
        };
        query_tasks(
            &self.conn,
            &format!("WHERE status = ?1 {order}"),
            [status.as_str()],
        )
    }

    /// 先頭のタスク（今やること）
    pub fn now(&self) -> Result<Option<Task>> {
        Ok(self.list(Status::Queued)?.into_iter().next())
    }

    /// キューの中の位置（1始まり）
    pub fn queue_position(&self, id: TaskId) -> Result<Option<usize>> {
        Ok(queued_positions(&self.conn, None)?
            .iter()
            .position(|(i, _)| *i == id)
            .map(|i| i + 1))
    }

    /// キューの `n` 番目（1始まり）のタスク
    pub fn task_at(&self, n: usize) -> Result<Task> {
        let list = queued_positions(&self.conn, None)?;
        let (id, _) = n
            .checked_sub(1)
            .and_then(|i| list.get(i))
            .ok_or_else(|| Error::InvalidInput(format!("キューに {n} 番目のタスクはありません")))?;
        self.get(*id)
    }

    /// 完了したタスクのうち、完了日時が `[start, end)` のもの（古い順）
    pub fn list_done_between(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        include_recurring: bool,
    ) -> Result<Vec<Task>> {
        let src = if include_recurring {
            ""
        } else {
            "AND source = 'manual'"
        };
        query_tasks(
            &self.conn,
            &format!(
                "WHERE status = 'done' AND completed_at >= ?1 AND completed_at < ?2 {src} \
                 ORDER BY completed_at, id"
            ),
            params![ts(&start), ts(&end)],
        )
    }

    /// タイトルとメモを検索する。`include_all` が false ならキューと保留のみ
    pub fn search(&self, query: &str, include_all: bool) -> Result<Vec<Task>> {
        let pattern = format!(
            "%{}%",
            query
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let status = if include_all {
            "status != 'deleted'"
        } else {
            "status IN ('queued','on_hold')"
        };
        query_tasks(
            &self.conn,
            &format!(
                "WHERE {status} AND (title LIKE ?1 ESCAPE '\\' OR memo LIKE ?1 ESCAPE '\\') \
                 ORDER BY CASE status WHEN 'queued' THEN 0 WHEN 'on_hold' THEN 1 ELSE 2 END, position, id"
            ),
            [pattern],
        )
    }

    pub fn counts(&self) -> Result<Vec<(Status, i64)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT status, COUNT(*) FROM tasks GROUP BY status")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(s, n)| Ok((Status::parse(&s).map_err(|e| conv_err(0, e))?, n)))
            .collect()
    }

    /// 操作履歴（新しい順）
    pub fn history(&self, limit: usize, session: Option<&str>) -> Result<Vec<Operation>> {
        let mut stmt = self.conn.prepare(&format!(
            "{OP_SELECT} WHERE (?1 IS NULL OR o.session_id = ?1) ORDER BY o.id DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![session, limit as i64], op_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn operation(&self, id: OpId) -> Result<Operation> {
        get_operation(&self.conn, id)
    }

    /// 最近のセッション（最後の操作が新しい順）
    pub fn sessions(&self, limit: usize) -> Result<Vec<Session>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.actor, s.label, s.started_at, COUNT(o.id), \
                    COALESCE(SUM(o.undo_seq IS NULL), 0), MAX(o.created_at) \
             FROM sessions s LEFT JOIN operations o ON o.session_id = s.id \
             GROUP BY s.id ORDER BY COALESCE(MAX(o.created_at), s.started_at) DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |r| {
            let actor: String = r.get(1)?;
            let started: String = r.get(3)?;
            let last: Option<String> = r.get(6)?;
            Ok(Session {
                id: r.get(0)?,
                actor: Actor::parse(&actor).map_err(|e| conv_err(1, e))?,
                label: r.get(2)?,
                started_at: parse_ts(3, &started)?,
                op_count: r.get(4)?,
                active_op_count: r.get(5)?,
                last_op_at: last.map(|l| parse_ts(6, &l)).transpose()?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// AI（CLI / MCP）が最後に操作したセッション
    pub fn last_ai_session(&self) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT session_id FROM operations WHERE actor IN ('cli','mcp') AND session_id IS NOT NULL \
                 ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?)
    }

    // ----- 変更（すべて操作ログに記録される） ------------------------------

    /// 1回のトランザクションで変更を適用し、操作ログを記録する
    fn mutate<F>(&mut self, ctx: &Ctx, op_type: &str, f: F) -> Result<OpResult>
    where
        F: FnOnce(&Connection, &mut Changes) -> Result<(String, TaskId)>,
    {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut changes = Changes::default();
        let (summary, primary) = f(&tx, &mut changes)?;
        changes
            .0
            .retain(|(b, a)| b.as_ref().is_none_or(|b| !b.same_content(a)));
        if changes.0.is_empty() {
            return Err(Error::NoChange);
        }
        let t_now = ts(&now());
        tx.execute(
            "INSERT OR IGNORE INTO sessions (id, actor, label, started_at) VALUES (?1, ?2, NULL, ?3)",
            params![ctx.session_id, ctx.actor.as_str(), t_now],
        )?;
        tx.execute(
            "INSERT INTO operations (op_type, summary, actor, session_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![op_type, summary, ctx.actor.as_str(), ctx.session_id, t_now],
        )?;
        let op_id = tx.last_insert_rowid();
        for (b, a) in &changes.0 {
            tx.execute(
                "INSERT INTO operation_changes (operation_id, task_id, before, after) VALUES (?1, ?2, ?3, ?4)",
                params![
                    op_id,
                    a.id,
                    b.as_ref().map(serde_json::to_string).transpose()?,
                    serde_json::to_string(a)?
                ],
            )?;
        }
        // 新しい操作をしたら、同じ範囲の Redo はできなくする
        tx.execute(
            "UPDATE operations SET redo_discarded = 1 WHERE undo_seq IS NOT NULL \
             AND (session_id = ?1 OR (?2 = 'app' AND actor = 'app'))",
            params![ctx.session_id, ctx.actor.as_str()],
        )?;
        let task = get_task(&tx, primary)?;
        tx.commit()?;
        Ok(OpResult {
            operation_id: op_id,
            task,
        })
    }

    /// 状態の遷移を伴う操作の共通処理
    fn transition<F>(
        &mut self,
        ctx: &Ctx,
        id: TaskId,
        op_type: &str,
        allowed: &[Status],
        verb: &str,
        f: F,
    ) -> Result<OpResult>
    where
        F: FnOnce(&Connection, &mut Task, &mut Changes) -> Result<()>,
    {
        self.mutate(ctx, op_type, |conn, ch| {
            let before = get_task(conn, id)?;
            if !allowed.contains(&before.status) {
                return Err(Error::InvalidState {
                    id,
                    status: before.status,
                });
            }
            let mut t = before.clone();
            f(conn, &mut t, ch)?;
            t.updated_at = now();
            write_task(conn, &t)?;
            let summary = format!("{} {verb}", label(id, &t.title));
            ch.record(Some(before), get_task(conn, id)?);
            Ok((summary, id))
        })
    }

    pub fn add(&mut self, ctx: &Ctx, new: NewTask, at: InsertAt) -> Result<OpResult> {
        let title = new.title.trim().to_string();
        if title.is_empty() {
            return Err(Error::InvalidInput("タイトルを入力してください".into()));
        }
        if !matches!(new.status, Status::Queued | Status::OnHold) {
            return Err(Error::InvalidInput(
                "追加できる状態はキューか保留のみです".into(),
            ));
        }
        let urls = dedup_urls(new.urls);
        self.mutate(ctx, "add", |conn, ch| {
            let t_now = ts(&now());
            let pos = if new.status == Status::Queued {
                position_for(conn, &at, None, ch)?
            } else {
                position::key_between("", None)
            };
            conn.execute(
                "INSERT INTO tasks (title, memo, due_date, urls, status, source, position, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                params![
                    title,
                    new.memo,
                    new.due_date.map(|d| d.format("%Y-%m-%d").to_string()),
                    serde_json::to_string(&urls)?,
                    new.status.as_str(),
                    new.source.as_str(),
                    pos,
                    t_now
                ],
            )?;
            let id = conn.last_insert_rowid();
            ch.record(None, get_task(conn, id)?);
            Ok((format!("{} を追加", label(id, &title)), id))
        })
    }

    pub fn edit(&mut self, ctx: &Ctx, id: TaskId, patch: TaskPatch) -> Result<OpResult> {
        let editable = [Status::Queued, Status::OnHold, Status::Done];
        self.transition(ctx, id, "edit", &editable, "を編集", |_, t, _| {
            if let Some(title) = patch.title {
                let title = title.trim().to_string();
                if title.is_empty() {
                    return Err(Error::InvalidInput("タイトルを空にはできません".into()));
                }
                t.title = title;
            }
            if let Some(memo) = patch.memo {
                t.memo = memo;
            }
            if let Some(extra) = patch.append_memo {
                if !t.memo.is_empty() && !t.memo.ends_with('\n') {
                    t.memo.push('\n');
                }
                t.memo.push_str(&extra);
            }
            if let Some(due) = patch.due_date {
                t.due_date = due;
            }
            if patch.clear_urls {
                t.urls.clear();
            }
            t.urls.retain(|u| !patch.remove_urls.contains(u));
            t.urls.extend(patch.add_urls);
            t.urls = dedup_urls(std::mem::take(&mut t.urls));
            Ok(())
        })
    }

    pub fn move_to(&mut self, ctx: &Ctx, id: TaskId, at: InsertAt) -> Result<OpResult> {
        if matches!(at, InsertAt::Before(r) | InsertAt::After(r) if r == id) {
            return Err(Error::InvalidInput(
                "自分自身を基準に移動することはできません".into(),
            ));
        }
        self.transition(
            ctx,
            id,
            "move",
            &[Status::Queued],
            "を移動",
            |conn, t, ch| {
                t.position = position_for(conn, &at, Some(id), ch)?;
                Ok(())
            },
        )
    }

    pub fn complete(&mut self, ctx: &Ctx, id: TaskId) -> Result<OpResult> {
        self.transition(
            ctx,
            id,
            "done",
            &[Status::Queued, Status::OnHold],
            "を完了",
            |_, t, _| {
                t.status = Status::Done;
                t.completed_at = Some(now());
                Ok(())
            },
        )
    }

    /// 完了を取り消す。並び順のキーを残しているので、元の位置に戻る
    pub fn reopen(&mut self, ctx: &Ctx, id: TaskId) -> Result<OpResult> {
        self.transition(
            ctx,
            id,
            "reopen",
            &[Status::Done],
            "の完了を取り消し",
            |_, t, _| {
                t.status = Status::Queued;
                t.completed_at = None;
                Ok(())
            },
        )
    }

    pub fn hold(&mut self, ctx: &Ctx, id: TaskId) -> Result<OpResult> {
        self.transition(
            ctx,
            id,
            "hold",
            &[Status::Queued],
            "を保留",
            |_, t, _| {
                t.status = Status::OnHold;
                Ok(())
            },
        )
    }

    pub fn resume(&mut self, ctx: &Ctx, id: TaskId, at: InsertAt) -> Result<OpResult> {
        self.transition(
            ctx,
            id,
            "resume",
            &[Status::OnHold],
            "の保留を解除",
            |conn, t, ch| {
                t.position = position_for(conn, &at, Some(id), ch)?;
                t.status = Status::Queued;
                Ok(())
            },
        )
    }

    /// 論理削除。「完了済み」画面や Undo から復元できる
    pub fn delete(&mut self, ctx: &Ctx, id: TaskId) -> Result<OpResult> {
        let from = [Status::Queued, Status::OnHold, Status::Done];
        self.transition(ctx, id, "delete", &from, "を削除", |_, t, _| {
            t.status = Status::Deleted;
            Ok(())
        })
    }

    /// 削除済みのタスクをキューの元の位置に戻す
    pub fn restore(&mut self, ctx: &Ctx, id: TaskId) -> Result<OpResult> {
        self.transition(
            ctx,
            id,
            "restore",
            &[Status::Deleted],
            "を復元",
            |_, t, _| {
                t.status = Status::Queued;
                t.completed_at = None;
                Ok(())
            },
        )
    }

    // ----- Undo / Redo / ロールバック -------------------------------------

    /// 範囲内で最新の操作を1つ取り消す
    pub fn undo(&mut self, scope: &Scope) -> Result<Operation> {
        let (cond, p) = scope_sql(scope);
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let op_id: OpId = tx
            .query_row(
                &format!("SELECT o.id FROM operations o WHERE {cond} AND o.undo_seq IS NULL ORDER BY o.id DESC LIMIT 1"),
                [p],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(Error::NothingToUndo)?;
        apply_undo(&tx, op_id)?.map_err(Error::Conflict)?;
        let op = get_operation(&tx, op_id)?;
        tx.commit()?;
        Ok(op)
    }

    /// 範囲内で最後に取り消した操作をやり直す
    pub fn redo(&mut self, scope: &Scope) -> Result<Operation> {
        let (cond, p) = scope_sql(scope);
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let op_id: OpId = tx
            .query_row(
                &format!(
                    "SELECT o.id FROM operations o WHERE {cond} AND o.undo_seq IS NOT NULL \
                     AND o.redo_discarded = 0 ORDER BY o.undo_seq DESC LIMIT 1"
                ),
                [p],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(Error::NothingToRedo)?;
        apply_redo(&tx, op_id)?.map_err(Error::Conflict)?;
        let op = get_operation(&tx, op_id)?;
        tx.commit()?;
        Ok(op)
    }

    /// セッションの操作をまとめて取り消す（新しい順）。
    /// 競合があった場合、`skip_conflicts` が false なら何も変更せずにエラーを返す。
    /// `dry_run` が true なら結果だけ返して変更は確定しない。
    pub fn rollback_session(
        &mut self,
        session_id: &str,
        skip_conflicts: bool,
        dry_run: bool,
    ) -> Result<UndoOutcome> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ids: Vec<OpId> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM operations WHERE session_id = ?1 AND undo_seq IS NULL ORDER BY id DESC",
            )?;
            stmt.query_map([session_id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        if ids.is_empty() {
            return Err(Error::NothingToUndo);
        }
        let mut out = UndoOutcome::default();
        for id in ids {
            match apply_undo(&tx, id)? {
                Ok(()) => out.undone.push(get_operation(&tx, id)?),
                Err(c) if skip_conflicts => out.skipped.extend(c),
                Err(c) => return Err(Error::Conflict(c)),
            }
        }
        if !dry_run {
            tx.commit()?;
        }
        Ok(out)
    }

    /// 指定した操作より後の操作を、操作者に関係なくすべて取り消す（非常用）
    pub fn restore_to(&mut self, op_id: OpId) -> Result<Vec<Operation>> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        get_operation(&tx, op_id)?;
        let ids: Vec<OpId> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM operations WHERE id > ?1 AND undo_seq IS NULL ORDER BY id DESC",
            )?;
            stmt.query_map([op_id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        let mut undone = Vec::new();
        for id in ids {
            apply_undo(&tx, id)?.map_err(Error::Conflict)?;
            undone.push(get_operation(&tx, id)?);
        }
        tx.commit()?;
        Ok(undone)
    }

    // ----- セッション -------------------------------------------------------

    pub fn new_session_id() -> String {
        format!("s-{}", &uuid::Uuid::new_v4().simple().to_string()[..8])
    }

    /// セッションを作成して ID を返す
    pub fn start_session(&mut self, actor: Actor, label: Option<&str>) -> Result<String> {
        let id = Self::new_session_id();
        self.conn.execute(
            "INSERT INTO sessions (id, actor, label, started_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, actor.as_str(), label, ts(&now())],
        )?;
        Ok(id)
    }

    pub fn get_setting<T: for<'de> Deserialize<'de>>(&self, key: &str) -> Result<Option<T>> {
        let v: Option<String> = self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(v.map(|v| serde_json::from_str(&v)).transpose()?)
    }

    pub fn set_setting<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, serde_json::to_string(value)?],
        )?;
        Ok(())
    }

    /// CLI のアクティブセッションを設定する
    pub fn set_active_session(&self, id: &str) -> Result<()> {
        self.set_setting(
            ACTIVE_SESSION_KEY,
            &ActiveSession {
                id: id.to_string(),
                last_used: now(),
            },
        )
    }

    /// CLI のアクティブセッション。最後に使ってから `idle` を過ぎていれば無効
    pub fn active_session(&self, idle: Duration) -> Result<Option<String>> {
        let Some(s) = self.get_setting::<ActiveSession>(ACTIVE_SESSION_KEY)? else {
            return Ok(None);
        };
        let idle = chrono::Duration::from_std(idle).unwrap_or(chrono::Duration::MAX);
        Ok((now() - s.last_used <= idle).then_some(s.id))
    }

    /// アクティブセッションの最終使用時刻を更新する
    pub fn touch_active_session(&self) -> Result<()> {
        if let Some(mut s) = self.get_setting::<ActiveSession>(ACTIVE_SESSION_KEY)? {
            s.last_used = now();
            self.set_setting(ACTIVE_SESSION_KEY, &s)?;
        }
        Ok(())
    }

    /// アクティブセッションを終了し、その ID を返す
    pub fn clear_active_session(&self) -> Result<Option<String>> {
        let s = self.get_setting::<ActiveSession>(ACTIVE_SESSION_KEY)?;
        self.conn
            .execute("DELETE FROM settings WHERE key = ?1", [ACTIVE_SESSION_KEY])?;
        Ok(s.map(|s| s.id))
    }

    // ----- バックアップ -----------------------------------------------------

    /// `dir` にバックアップを作り、古いものは `keep` 世代を残して削除する
    pub fn backup_to(&self, dir: &Path, keep: usize) -> Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let name = format!("dandori-{}.db", Local::now().format("%Y%m%d-%H%M%S"));
        let dest = dir.join(name);
        let dest_str = dest.to_string_lossy().replace('\'', "''");
        self.conn
            .execute_batch(&format!("VACUUM INTO '{dest_str}'"))?;

        let mut backups: Vec<PathBuf> = std::fs::read_dir(dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("dandori-") && n.ends_with(".db"))
            })
            .collect();
        backups.sort();
        let excess = backups.len().saturating_sub(keep);
        for old in backups.into_iter().take(excess) {
            std::fs::remove_file(old)?;
        }
        Ok(dest)
    }
}

fn dedup_urls(urls: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for u in urls
        .into_iter()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
    {
        if !out.contains(&u) {
            out.push(u);
        }
    }
    out
}
