use std::collections::BTreeMap;
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use chrono::{Datelike, Days, Local, NaiveDate, SecondsFormat, Weekday};
use clap::{Args, Parser, Subcommand, ValueEnum};
use dandori_core::*;
use serde::Serialize;

const SESSION_IDLE: Duration = Duration::from_secs(30 * 60);
const BACKUP_KEEP: usize = 30;

#[derive(Parser)]
#[command(
    name = "dan",
    version,
    about = "Dandori — 仕事用のシングルタスク・キュー CLI"
)]
struct Cli {
    /// JSON で出力する
    #[arg(long, global = true)]
    json: bool,

    /// 操作を記録するセッション ID
    #[arg(long, env = "DANDORI_SESSION", global = true)]
    session: Option<String>,

    /// DB ファイルのパス
    #[arg(long, env = paths::DB_ENV, global = true)]
    db: Option<PathBuf>,

    /// 成功時に何も出力しない
    #[arg(short, long, global = true)]
    quiet: bool,

    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// 「今やること」（先頭のタスク）を表示する
    Now,
    /// タスクを一覧表示する
    List {
        /// 保留中のタスクを表示する
        #[arg(long, conflicts_with_all = ["done", "deleted"])]
        hold: bool,
        /// 完了したタスクを表示する
        #[arg(long, conflicts_with_all = ["hold", "deleted"])]
        done: bool,
        /// 削除済みのタスクを表示する
        #[arg(long, conflicts_with_all = ["hold", "done"])]
        deleted: bool,
        /// 完了タスクの対象月（`this` / `last` / `YYYY-MM`。省略時は今月）
        #[arg(long)]
        month: Option<String>,
    },
    /// タスクの詳細を表示する
    Show {
        /// タスク ID（`12` / `#12`）またはキュー位置（`@1`）
        target: String,
    },
    /// タイトルとメモを検索する
    Search {
        query: String,
        /// 完了済みも含めて検索する
        #[arg(long)]
        all: bool,
    },
    /// タスクを追加する（デフォルトは末尾）
    Add {
        title: String,
        #[arg(long)]
        memo: Option<String>,
        /// メモをファイルから読み込む（`-` で標準入力）
        #[arg(long)]
        memo_file: Option<String>,
        /// 期限（`YYYY-MM-DD` / `today` / `tomorrow` / `+3d` / `fri` など）
        #[arg(long)]
        due: Option<String>,
        #[arg(long = "url")]
        urls: Vec<String>,
        /// 保留として追加する
        #[arg(long)]
        hold: bool,
        #[command(flatten)]
        pos: PosArgs,
    },
    /// タスクを編集する
    Edit {
        target: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        memo: Option<String>,
        #[arg(long)]
        memo_file: Option<String>,
        #[arg(long)]
        append_memo: Option<String>,
        #[arg(long, conflicts_with = "no_due")]
        due: Option<String>,
        #[arg(long)]
        no_due: bool,
        #[arg(long = "url")]
        urls: Vec<String>,
        #[arg(long = "remove-url")]
        remove_urls: Vec<String>,
        #[arg(long)]
        clear_urls: bool,
    },
    /// キュー内の位置を移動する
    Move {
        target: String,
        #[command(flatten)]
        pos: PosArgs,
    },
    /// タスクを完了にする（引数を省略すると先頭のタスク）
    Done { targets: Vec<String> },
    /// 完了を取り消して元の位置に戻す
    Reopen { target: String },
    /// タスクを保留にする
    Hold { target: String },
    /// 保留を解除してキューに戻す（デフォルトは先頭）
    Resume {
        target: String,
        #[command(flatten)]
        pos: PosArgs,
    },
    /// タスクを削除する（論理削除）
    Delete {
        #[arg(required = true)]
        targets: Vec<String>,
    },
    /// 削除したタスクを元の位置に復元する
    Restore { target: String },
    /// AI 作業用のセッションを管理する
    Session {
        #[command(subcommand)]
        sub: SessionCmd,
    },
    /// 直前の操作（現在のセッション内）を 1 つ取り消す
    Undo,
    /// 取り消した操作をやり直す
    Redo,
    /// セッションの操作をまとめて取り消す
    Rollback {
        /// 対象のセッション ID（省略時は現在のセッションまたは直前の AI セッション）
        #[arg(long)]
        session: Option<String>,
        /// 競合しているタスクを飛ばして残りを戻す
        #[arg(long)]
        skip_conflicts: bool,
        /// 実際には変更せず結果だけ確認する
        #[arg(long)]
        dry_run: bool,
    },
    /// 操作履歴を表示する
    History {
        #[arg(long, default_value_t = 30)]
        limit: usize,
        #[arg(long)]
        session: Option<String>,
    },
    /// 月次の振り返り用に完了タスクを出力する
    Report {
        /// 対象月（`last`［デフォルト］/ `this` / `YYYY-MM`）
        #[arg(long)]
        month: Option<String>,
        /// 定期タスクも含める
        #[arg(long)]
        include_recurring: bool,
        #[arg(long, value_enum, default_value_t = ReportFormat::Md)]
        format: ReportFormat,
    },
    /// AI 向けの使い方ガイドを出力する
    Guide,
    /// 手動でバックアップを作成する
    Backup,
    /// DB の場所・バージョン・件数を表示する
    Info,
}

#[derive(Subcommand)]
enum SessionCmd {
    /// アクティブセッションを開始する
    Start {
        #[arg(long)]
        label: Option<String>,
    },
    /// アクティブセッションを終了する
    End,
    /// 最近のセッションを一覧表示する
    List {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
}

#[derive(Args, Clone, Default)]
struct PosArgs {
    /// 先頭に入れる
    #[arg(long, group = "pos")]
    top: bool,
    /// 末尾に入れる
    #[arg(long, group = "pos")]
    bottom: bool,
    /// n 番目（1始まり）に入れる
    #[arg(long, group = "pos")]
    at: Option<usize>,
    /// 指定したタスクの前に入れる
    #[arg(long, group = "pos")]
    before: Option<String>,
    /// 指定したタスクの後に入れる
    #[arg(long, group = "pos")]
    after: Option<String>,
}

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq)]
enum ReportFormat {
    Md,
    Json,
}

// ---------------------------------------------------------------------------
// JSON 出力用構造体
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct TaskView {
    id: TaskId,
    title: String,
    memo: String,
    due_date: Option<String>,
    urls: Vec<String>,
    status: Status,
    source: Source,
    queue_position: Option<usize>,
    created_at: String,
    updated_at: String,
    completed_at: Option<String>,
}

fn to_local_rfc3339(t: &chrono::DateTime<chrono::Utc>) -> String {
    t.with_timezone(&Local)
        .to_rfc3339_opts(SecondsFormat::Secs, false)
}

fn task_view(store: &Store, t: Task) -> Result<TaskView> {
    let qpos = if t.status == Status::Queued {
        store.queue_position(t.id)?
    } else {
        None
    };
    Ok(TaskView {
        id: t.id,
        title: t.title,
        memo: t.memo,
        due_date: t.due_date.map(|d| d.format("%Y-%m-%d").to_string()),
        urls: t.urls,
        status: t.status,
        source: t.source,
        queue_position: qpos,
        created_at: to_local_rfc3339(&t.created_at),
        updated_at: to_local_rfc3339(&t.updated_at),
        completed_at: t.completed_at.as_ref().map(to_local_rfc3339),
    })
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            let _ = e.print();
            return ExitCode::from(if e.use_stderr() { 2 } else { 0 });
        }
    };
    let json = cli.json;
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let code = exit_code(&err);
            if json {
                let conflicts = match &err {
                    Error::Conflict(c) => Some(c),
                    _ => None,
                };
                let body = serde_json::json!({
                    "ok": false,
                    "error": {
                        "code": err.code(),
                        "message": err.to_string(),
                        "conflicts": conflicts,
                    }
                });
                eprintln!(
                    "{}",
                    serde_json::to_string_pretty(&body).unwrap_or_default()
                );
            } else {
                eprintln!("エラー: {err}");
            }
            ExitCode::from(code)
        }
    }
}

fn exit_code(err: &Error) -> u8 {
    match err {
        Error::InvalidInput(_) => 2,
        Error::NotFound(_) | Error::OperationNotFound(_) => 3,
        Error::Conflict(_) => 4,
        _ if err.is_busy() => 5,
        _ => 1,
    }
}

fn run(mut cli: Cli) -> Result<()> {
    let cmd = cli.cmd.take().unwrap_or(Cmd::Now);
    if matches!(cmd, Cmd::Guide) {
        if !cli.quiet {
            print!("{GUIDE}");
        }
        return Ok(());
    }

    let db_path = cli.db.clone().unwrap_or_else(paths::default_db_path);
    let mut store = Store::open(&db_path)?;

    match cmd {
        Cmd::Guide => unreachable!(),
        Cmd::Now => {
            let now = store.now()?;
            if cli.json {
                let v = now.map(|t| task_view(&store, t)).transpose()?;
                print_json(&v);
            } else if !cli.quiet {
                match now {
                    Some(t) => print_task_detail(&store, &t)?,
                    None => println!("キューは空です"),
                }
            }
        }
        Cmd::List {
            hold,
            done,
            deleted,
            month,
        } => {
            let tasks = if hold {
                store.list(Status::OnHold)?
            } else if deleted {
                store.list(Status::Deleted)?
            } else if done {
                let (y, m) = parse_month(month.as_deref().unwrap_or("this"))?;
                let (start, end) = month_range_utc(y, m)?;
                let mut list = store.list_done_between(start, end, true)?;
                list.reverse();
                list
            } else {
                store.list(Status::Queued)?
            };
            if cli.json {
                let views: Vec<TaskView> = tasks
                    .into_iter()
                    .map(|t| task_view(&store, t))
                    .collect::<Result<_>>()?;
                print_json(&views);
            } else if !cli.quiet {
                if !hold && !done && !deleted {
                    print_queue(&store, &tasks)?;
                } else {
                    print_flat_list(&tasks);
                }
            }
        }
        Cmd::Show { target } => {
            let id = resolve_ref(&store, &target)?;
            let t = store.get(id)?;
            if cli.json {
                print_json(&task_view(&store, t)?);
            } else if !cli.quiet {
                print_task_detail(&store, &t)?;
            }
        }
        Cmd::Search { query, all } => {
            let list = store.search(&query, all)?;
            if cli.json {
                let views: Vec<TaskView> = list
                    .into_iter()
                    .map(|t| task_view(&store, t))
                    .collect::<Result<_>>()?;
                print_json(&views);
            } else if !cli.quiet {
                print_flat_list(&list);
            }
        }
        Cmd::Add {
            title,
            memo,
            memo_file,
            due,
            urls,
            hold,
            pos,
        } => {
            let ctx = write_ctx(&store, cli.session.as_deref())?;
            let at = resolve_pos(&store, &pos, InsertAt::Bottom)?;
            let mut new = NewTask::new(title);
            new.memo = read_memo(memo, memo_file)?;
            new.due_date = due.as_deref().map(parse_due).transpose()?;
            new.urls = urls;
            new.status = if hold { Status::OnHold } else { Status::Queued };
            let res = store.add(&ctx, new, at)?;
            output_op(&store, &cli, res, "追加しました")?;
        }
        Cmd::Edit {
            target,
            title,
            memo,
            memo_file,
            append_memo,
            due,
            no_due,
            urls,
            remove_urls,
            clear_urls,
        } => {
            let ctx = write_ctx(&store, cli.session.as_deref())?;
            let id = resolve_ref(&store, &target)?;
            let memo_val = if memo.is_some() || memo_file.is_some() {
                Some(read_memo(memo, memo_file)?)
            } else {
                None
            };
            let due_date = if no_due {
                Some(None)
            } else if let Some(d) = due {
                Some(Some(parse_due(&d)?))
            } else {
                None
            };
            let patch = TaskPatch {
                title,
                memo: memo_val,
                append_memo,
                due_date,
                add_urls: urls,
                remove_urls,
                clear_urls,
            };
            let res = store.edit(&ctx, id, patch)?;
            output_op(&store, &cli, res, "更新しました")?;
        }
        Cmd::Move { target, pos } => {
            let ctx = write_ctx(&store, cli.session.as_deref())?;
            let id = resolve_ref(&store, &target)?;
            let at = resolve_pos_required(&store, &pos)?;
            let res = store.move_to(&ctx, id, at)?;
            output_op(&store, &cli, res, "移動しました")?;
        }
        Cmd::Done { targets } => {
            let ctx = write_ctx(&store, cli.session.as_deref())?;
            let ids = if targets.is_empty() {
                vec![
                    store
                        .now()?
                        .ok_or_else(|| Error::InvalidInput("キューは空です".into()))?
                        .id,
                ]
            } else {
                targets
                    .iter()
                    .map(|t| resolve_ref(&store, t))
                    .collect::<Result<Vec<_>>>()?
            };
            let mut last = None;
            for id in ids {
                last = Some(store.complete(&ctx, id)?);
            }
            if let Some(res) = last {
                output_op(&store, &cli, res, "完了しました")?;
            }
        }
        Cmd::Reopen { target } => {
            let ctx = write_ctx(&store, cli.session.as_deref())?;
            let id = resolve_ref(&store, &target)?;
            let res = store.reopen(&ctx, id)?;
            output_op(&store, &cli, res, "未完了に戻しました")?;
        }
        Cmd::Hold { target } => {
            let ctx = write_ctx(&store, cli.session.as_deref())?;
            let id = resolve_ref(&store, &target)?;
            let res = store.hold(&ctx, id)?;
            output_op(&store, &cli, res, "保留にしました")?;
        }
        Cmd::Resume { target, pos } => {
            let ctx = write_ctx(&store, cli.session.as_deref())?;
            let id = resolve_ref(&store, &target)?;
            let at = resolve_pos(&store, &pos, InsertAt::Top)?;
            let res = store.resume(&ctx, id, at)?;
            output_op(&store, &cli, res, "キューに戻しました")?;
        }
        Cmd::Delete { targets } => {
            let ctx = write_ctx(&store, cli.session.as_deref())?;
            let ids = targets
                .iter()
                .map(|t| resolve_ref(&store, t))
                .collect::<Result<Vec<_>>>()?;
            let mut last = None;
            for id in ids {
                last = Some(store.delete(&ctx, id)?);
            }
            if let Some(res) = last {
                output_op(&store, &cli, res, "削除しました")?;
            }
        }
        Cmd::Restore { target } => {
            let ctx = write_ctx(&store, cli.session.as_deref())?;
            let id = resolve_ref(&store, &target)?;
            let res = store.restore(&ctx, id)?;
            output_op(&store, &cli, res, "復元しました")?;
        }
        Cmd::Session { sub } => match sub {
            SessionCmd::Start { label } => {
                let id = store.start_session(Actor::Cli, label.as_deref())?;
                store.set_active_session(&id)?;
                if cli.json {
                    print_json(
                        &serde_json::json!({ "ok": true, "session_id": id, "label": label }),
                    );
                } else if !cli.quiet {
                    println!("{id}");
                }
            }
            SessionCmd::End => {
                let ended = store.clear_active_session()?;
                if cli.json {
                    print_json(&serde_json::json!({ "ok": true, "ended_session_id": ended }));
                } else if !cli.quiet {
                    match ended {
                        Some(id) => println!("セッション {id} を終了しました"),
                        None => println!("アクティブなセッションはありません"),
                    }
                }
            }
            SessionCmd::List { limit } => {
                let list = store.sessions(limit)?;
                if cli.json {
                    print_json(&list);
                } else if !cli.quiet {
                    let active = store.active_session(SESSION_IDLE)?;
                    for s in list {
                        let mark = if active.as_deref() == Some(s.id.as_str()) {
                            "*"
                        } else {
                            " "
                        };
                        let label = s.label.as_deref().unwrap_or("-");
                        let ts = s.started_at.with_timezone(&Local).format("%Y-%m-%d %H:%M");
                        println!(
                            "{mark} {:<12} {:<4} {ts}  有効 {} / 全 {} 件  {label}",
                            s.id, s.actor, s.active_op_count, s.op_count
                        );
                    }
                }
            }
        },
        Cmd::Undo => {
            let sid = target_ai_session(&store, cli.session.as_deref())?;
            let op = store.undo(&Scope::Session(sid))?;
            if cli.json {
                print_json(&serde_json::json!({ "ok": true, "undone": op }));
            } else if !cli.quiet {
                println!("取り消しました: {}", op.summary);
            }
        }
        Cmd::Redo => {
            let sid = target_ai_session(&store, cli.session.as_deref())?;
            let op = store.redo(&Scope::Session(sid))?;
            if cli.json {
                print_json(&serde_json::json!({ "ok": true, "redone": op }));
            } else if !cli.quiet {
                println!("やり直しました: {}", op.summary);
            }
        }
        Cmd::Rollback {
            session,
            skip_conflicts,
            dry_run,
        } => {
            let sid = target_ai_session(&store, session.as_deref().or(cli.session.as_deref()))?;
            let out = store.rollback_session(&sid, skip_conflicts, dry_run)?;
            if cli.json {
                print_json(&serde_json::json!({
                    "ok": true,
                    "session_id": sid,
                    "dry_run": dry_run,
                    "undone": out.undone,
                    "skipped": out.skipped,
                }));
            } else if !cli.quiet {
                let prefix = if dry_run { "[dry-run] " } else { "" };
                println!(
                    "{prefix}セッション {sid} の操作を {} 件取り消しました（スキップ {} 件）",
                    out.undone.len(),
                    out.skipped.len()
                );
                for op in &out.undone {
                    println!("  - {}", op.summary);
                }
            }
        }
        Cmd::History { limit, session } => {
            let ops = store.history(limit, session.as_deref())?;
            if cli.json {
                print_json(&ops);
            } else if !cli.quiet {
                for op in ops {
                    let ts = op.created_at.with_timezone(&Local).format("%Y-%m-%d %H:%M");
                    let state = if op.undone { " [取消済]" } else { "" };
                    let sess = op.session_id.as_deref().unwrap_or("-");
                    println!(
                        "#{:<4} {ts} {:<4} ({sess}) {}{state}",
                        op.id, op.actor, op.summary
                    );
                }
            }
        }
        Cmd::Report {
            month,
            include_recurring,
            format,
        } => {
            let (y, m) = parse_month(month.as_deref().unwrap_or("last"))?;
            let (start, end) = month_range_utc(y, m)?;
            let all = store.list_done_between(start, end, true)?;
            let recurring_count = all.iter().filter(|t| t.source == Source::Recurring).count();
            let tasks: Vec<Task> = if include_recurring {
                all
            } else {
                all.into_iter()
                    .filter(|t| t.source == Source::Manual)
                    .collect()
            };
            if cli.json || format == ReportFormat::Json {
                let views: Vec<TaskView> = tasks
                    .into_iter()
                    .map(|t| task_view(&store, t))
                    .collect::<Result<_>>()?;
                print_json(&serde_json::json!({
                    "month": format!("{y:04}-{m:02}"),
                    "count": views.len(),
                    "excluded_recurring": if include_recurring { 0 } else { recurring_count },
                    "tasks": views,
                }));
            } else if !cli.quiet {
                print_report_md(
                    y,
                    m,
                    &tasks,
                    if include_recurring {
                        0
                    } else {
                        recurring_count
                    },
                );
            }
        }
        Cmd::Backup => {
            let dir = paths::backup_dir(&db_path);
            let path = store.backup_to(&dir, BACKUP_KEEP)?;
            if cli.json {
                print_json(&serde_json::json!({ "ok": true, "path": path }));
            } else if !cli.quiet {
                println!("バックアップを作成しました: {}", path.display());
            }
        }
        Cmd::Info => {
            let counts = store.counts()?;
            let count_of = |s: Status| counts.iter().find(|(k, _)| *k == s).map_or(0, |(_, n)| *n);
            let active = store.active_session(SESSION_IDLE)?;
            if cli.json {
                print_json(&serde_json::json!({
                    "db_path": db_path,
                    "schema_version": store.schema_version()?,
                    "active_session": active,
                    "counts": {
                        "queued": count_of(Status::Queued),
                        "on_hold": count_of(Status::OnHold),
                        "done": count_of(Status::Done),
                        "deleted": count_of(Status::Deleted),
                    }
                }));
            } else if !cli.quiet {
                println!("DB: {}", db_path.display());
                println!("スキーマ: v{}", store.schema_version()?);
                println!(
                    "件数: キュー {} / 保留 {} / 完了 {} / 削除済み {}",
                    count_of(Status::Queued),
                    count_of(Status::OnHold),
                    count_of(Status::Done),
                    count_of(Status::Deleted)
                );
                if let Some(s) = active {
                    println!("アクティブセッション: {s}");
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 入力の解釈
// ---------------------------------------------------------------------------

fn resolve_ref(store: &Store, raw: &str) -> Result<TaskId> {
    let s = raw.trim();
    if let Some(pos) = s.strip_prefix('@') {
        let n: usize = pos
            .parse()
            .map_err(|_| Error::InvalidInput(format!("不正な位置指定です: {raw}")))?;
        return Ok(store.task_at(n)?.id);
    }
    let id_str = s.strip_prefix('#').unwrap_or(s);
    id_str
        .parse::<TaskId>()
        .map_err(|_| Error::InvalidInput(format!("不正なタスク指定です: {raw}")))
}

fn resolve_pos(store: &Store, p: &PosArgs, default: InsertAt) -> Result<InsertAt> {
    if p.top {
        Ok(InsertAt::Top)
    } else if p.bottom {
        Ok(InsertAt::Bottom)
    } else if let Some(n) = p.at {
        Ok(InsertAt::At(n))
    } else if let Some(r) = &p.before {
        Ok(InsertAt::Before(resolve_ref(store, r)?))
    } else if let Some(r) = &p.after {
        Ok(InsertAt::After(resolve_ref(store, r)?))
    } else {
        Ok(default)
    }
}

fn resolve_pos_required(store: &Store, p: &PosArgs) -> Result<InsertAt> {
    if !p.top && !p.bottom && p.at.is_none() && p.before.is_none() && p.after.is_none() {
        return Err(Error::InvalidInput(
            "移動先を指定してください（--top / --bottom / --at <n> / --before <ref> / --after <ref>）".into(),
        ));
    }
    resolve_pos(store, p, InsertAt::Bottom)
}

fn write_ctx(store: &Store, explicit: Option<&str>) -> Result<Ctx> {
    let session_id = if let Some(s) = explicit.filter(|s| !s.is_empty()) {
        s.to_string()
    } else if let Some(s) = store.active_session(SESSION_IDLE)? {
        store.touch_active_session()?;
        s
    } else {
        Store::new_session_id()
    };
    Ok(Ctx {
        actor: Actor::Cli,
        session_id,
    })
}

fn target_ai_session(store: &Store, explicit: Option<&str>) -> Result<String> {
    if let Some(s) = explicit.filter(|s| !s.is_empty()) {
        return Ok(s.to_string());
    }
    if let Some(s) = store.active_session(SESSION_IDLE)? {
        store.touch_active_session()?;
        return Ok(s);
    }
    store.last_ai_session()?.ok_or(Error::NothingToUndo)
}

fn read_memo(memo: Option<String>, memo_file: Option<String>) -> Result<String> {
    if let Some(path) = memo_file {
        if path == "-" {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            return Ok(buf.trim_end_matches(&['\r', '\n'][..]).to_string());
        }
        return Ok(std::fs::read_to_string(path)?);
    }
    Ok(memo.unwrap_or_default())
}

fn parse_due(raw: &str) -> Result<NaiveDate> {
    let s = raw.trim().to_lowercase();
    let today = Local::now().date_naive();
    if s == "today" {
        return Ok(today);
    }
    if s == "tomorrow" {
        return Ok(today.checked_add_days(Days::new(1)).unwrap());
    }
    if let Some(days) = s.strip_prefix('+').and_then(|r| r.strip_suffix('d'))
        && let Ok(n) = days.parse::<u64>()
    {
        return Ok(today.checked_add_days(Days::new(n)).unwrap());
    }
    let weekday = match s.as_str() {
        "mon" | "monday" => Some(Weekday::Mon),
        "tue" | "tuesday" => Some(Weekday::Tue),
        "wed" | "wednesday" => Some(Weekday::Wed),
        "thu" | "thursday" => Some(Weekday::Thu),
        "fri" | "friday" => Some(Weekday::Fri),
        "sat" | "saturday" => Some(Weekday::Sat),
        "sun" | "sunday" => Some(Weekday::Sun),
        _ => None,
    };
    if let Some(w) = weekday {
        for d in 1..=7 {
            let cand = today.checked_add_days(Days::new(d)).unwrap();
            if cand.weekday() == w {
                return Ok(cand);
            }
        }
    }
    NaiveDate::parse_from_str(&s, "%Y-%m-%d")
        .map_err(|_| Error::InvalidInput(format!("期限の形式が正しくありません: {raw}")))
}

fn parse_month(raw: &str) -> Result<(i32, u32)> {
    let today = Local::now().date_naive();
    match raw.trim() {
        "this" => Ok((today.year(), today.month())),
        "last" => {
            if today.month() == 1 {
                Ok((today.year() - 1, 12))
            } else {
                Ok((today.year(), today.month() - 1))
            }
        }
        s => {
            let (y, m) = s.split_once('-').ok_or_else(|| {
                Error::InvalidInput(format!("月は YYYY-MM で指定してください: {s}"))
            })?;
            Ok((
                y.parse()
                    .map_err(|_| Error::InvalidInput(format!("不正な年: {s}")))?,
                m.parse()
                    .map_err(|_| Error::InvalidInput(format!("不正な月: {s}")))?,
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// 表示
// ---------------------------------------------------------------------------

fn print_json<T: Serialize>(v: &T) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

fn output_op(store: &Store, cli: &Cli, res: OpResult, verb: &str) -> Result<()> {
    if cli.json {
        let tv = task_view(store, res.task)?;
        print_json(&serde_json::json!({
            "ok": true,
            "operation_id": res.operation_id,
            "task": tv,
        }));
    } else if !cli.quiet {
        println!("#{} {} を{verb}", res.task.id, res.task.title);
    }
    Ok(())
}

fn format_due(due: Option<NaiveDate>) -> String {
    let Some(d) = due else { return String::new() };
    let today = Local::now().date_naive();
    let md = d.format("%m/%d");
    if d < today {
        format!("期限 {md}(超過)")
    } else if d == today {
        format!("期限 {md}(今日)")
    } else {
        format!("期限 {md}")
    }
}

fn print_queue(store: &Store, tasks: &[Task]) -> Result<()> {
    if tasks.is_empty() {
        println!("キューは空です");
    }
    for (i, t) in tasks.iter().enumerate() {
        let head = if i == 0 { "▶" } else { " " };
        let due = format_due(t.due_date);
        let url = if t.urls.is_empty() {
            String::new()
        } else {
            format!("🔗{}", t.urls.len())
        };
        let memo = if t.memo.is_empty() { "" } else { "📝" };
        println!(
            "  {head} {:>2} #{:<4} {:<32} {due} {url} {memo}",
            i + 1,
            t.id,
            t.title
        );
    }
    let hold_count = store.list(Status::OnHold)?.len();
    if hold_count > 0 {
        println!("  保留 {hold_count}件（dan list --hold で表示）");
    }
    Ok(())
}

fn print_flat_list(tasks: &[Task]) {
    if tasks.is_empty() {
        println!("該当するタスクはありません");
        return;
    }
    for t in tasks {
        let due = format_due(t.due_date);
        println!("  #{:<4} [{}] {}  {due}", t.id, t.status, t.title);
    }
}

fn print_task_detail(store: &Store, t: &Task) -> Result<()> {
    let pos = store
        .queue_position(t.id)?
        .map(|p| format!("（キュー {p} 番目）"))
        .unwrap_or_default();
    println!("#{} {} [{}]{pos}", t.id, t.title, t.status);
    if let Some(d) = t.due_date {
        println!("期限: {}", format_due(Some(d)));
    }
    for u in &t.urls {
        println!("URL:  {u}");
    }
    if !t.memo.is_empty() {
        println!("---");
        println!("{}", t.memo);
    }
    Ok(())
}

fn print_report_md(year: i32, month: u32, tasks: &[Task], excluded_recurring: usize) {
    let extra = if excluded_recurring > 0 {
        format!(" / 定期タスク {excluded_recurring}件を除外")
    } else {
        String::new()
    };
    println!(
        "# {year:04}-{month:02} 完了タスク（{}件{extra}）",
        tasks.len()
    );
    let mut by_date: BTreeMap<String, Vec<&Task>> = BTreeMap::new();
    for t in tasks {
        let day = t
            .completed_at
            .as_ref()
            .map(|c| c.with_timezone(&Local).format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "日付不明".into());
        by_date.entry(day).or_default().push(t);
    }
    for (day, list) in by_date {
        println!("\n## {day}");
        for t in list {
            println!("- {} (#{})", t.title, t.id);
            for u in &t.urls {
                println!("  - URL: {u}");
            }
            if !t.memo.is_empty() {
                for line in t.memo.lines() {
                    println!("  - {line}");
                }
            }
        }
    }
}

const GUIDE: &str = r#"# Dandori (`dan`) AI 向けガイド

Dandori は仕事用のシングルタスク・キューです。先頭のタスクが「今やること」です。

## 基本ルール
1. 一連の作業を始めるときはセッションを開始し、終わったら終了してください。
   ```sh
   dan session start --label "作業内容の短い説明"
   # ... タスク操作 ...
   dan session end
   ```
2. 参照系コマンドには `--json` を付けて構造化データとして読み取ってください。
   - キュー一覧: `dan list --json`
   - 先頭タスク: `dan now --json`
   - 詳細表示: `dan show <id> --json`
3. タスクの指定には不変の ID（例: `12`）を使ってください。
   - `@1`, `@2` の位置指定は、直前に `dan list` を確認したときだけ使ってください。
4. 追加のデフォルト位置は**末尾**です。位置を指定する場合は `--top` / `--bottom` / `--at <n>` / `--before <id>` / `--after <id>` を使います。
5. 複数行のメモは標準入力から渡してください:
   ```sh
   cat <<'EOF' | dan edit 12 --memo-file -
   ## メモ見出し
   - 箇条書き
   EOF
   ```
6. 完了・削除などの操作に人間の事前確認は不要です（いつでもロールバックできるため）。
   - 1つ戻す: `dan undo`
   - セッション全体を戻す: `dan rollback`
   - 月次の振り返り出力: `dan report --month last`（今月なら `--month this`）
"#;
