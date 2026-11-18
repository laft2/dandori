#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Local, NaiveDate, Utc};
use dandori_core::*;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

const SETTING_FONT_SIZE: &str = "ui_font_size";
const SETTING_LAST_BACKUP: &str = "last_backup_at";
const DEFAULT_FONT_SIZE: u32 = 15;
const BACKUP_KEEP: usize = 30;

struct AppState {
    store: Mutex<Store>,
    ctx: Ctx,
    db_path: PathBuf,
}

#[derive(Serialize)]
struct CommandError {
    code: &'static str,
    message: String,
    conflicts: Option<Vec<Conflict>>,
}

impl From<Error> for CommandError {
    fn from(e: Error) -> Self {
        let code = e.code();
        let message = e.to_string();
        let conflicts = match e {
            Error::Conflict(c) => Some(c),
            _ => None,
        };
        Self {
            code,
            message,
            conflicts,
        }
    }
}

type CmdResult<T> = std::result::Result<T, CommandError>;

#[derive(Serialize)]
struct Snapshot {
    queued: Vec<Task>,
    on_hold: Vec<Task>,
    done: Vec<Task>,
    deleted: Vec<Task>,
    font_size: u32,
    db_path: String,
    last_backup: Option<String>,
}

#[derive(Serialize)]
struct HistoryPayload {
    operations: Vec<Operation>,
    sessions: Vec<Session>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum PosInput {
    Top,
    Bottom,
    At { index: usize },
    Before { id: TaskId },
    After { id: TaskId },
}

impl From<PosInput> for InsertAt {
    fn from(p: PosInput) -> Self {
        match p {
            PosInput::Top => InsertAt::Top,
            PosInput::Bottom => InsertAt::Bottom,
            PosInput::At { index } => InsertAt::At(index),
            PosInput::Before { id } => InsertAt::Before(id),
            PosInput::After { id } => InsertAt::After(id),
        }
    }
}

#[derive(Deserialize)]
struct AddTaskInput {
    title: String,
    #[serde(default)]
    memo: String,
    due_date: Option<String>,
    #[serde(default)]
    urls: Vec<String>,
    pos: Option<PosInput>,
}

#[derive(Deserialize)]
struct EditTaskInput {
    id: TaskId,
    title: Option<String>,
    memo: Option<String>,
    /// `Some("")` または `Some(null)` で期限クリア、`Some("YYYY-MM-DD")` で設定
    due_date: Option<Option<String>>,
    urls: Option<Vec<String>>,
}

fn parse_opt_date(raw: Option<&str>) -> Result<Option<NaiveDate>> {
    let Some(s) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map(Some)
        .map_err(|_| Error::InvalidInput(format!("日付の形式が正しくありません: {s}")))
}

#[tauri::command]
fn get_snapshot(state: State<'_, AppState>) -> CmdResult<Snapshot> {
    let s = state.store.lock().unwrap();
    let font_size = s
        .get_setting::<u32>(SETTING_FONT_SIZE)?
        .unwrap_or(DEFAULT_FONT_SIZE);
    let last_backup = s
        .get_setting::<DateTime<Utc>>(SETTING_LAST_BACKUP)?
        .map(|t| t.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string());
    Ok(Snapshot {
        queued: s.list(Status::Queued)?,
        on_hold: s.list(Status::OnHold)?,
        done: s.list(Status::Done)?,
        deleted: s.list(Status::Deleted)?,
        font_size,
        db_path: state.db_path.display().to_string(),
        last_backup,
    })
}

#[tauri::command]
fn get_history(state: State<'_, AppState>, limit: Option<usize>) -> CmdResult<HistoryPayload> {
    let s = state.store.lock().unwrap();
    let lim = limit.unwrap_or(100);
    Ok(HistoryPayload {
        operations: s.history(lim, None)?,
        sessions: s.sessions(50)?,
    })
}

#[tauri::command]
fn add_task(state: State<'_, AppState>, input: AddTaskInput) -> CmdResult<OpResult> {
    let mut s = state.store.lock().unwrap();
    let mut new = NewTask::new(input.title);
    new.memo = input.memo;
    new.due_date = parse_opt_date(input.due_date.as_deref())?;
    new.urls = input.urls;
    let at = input.pos.map_or(InsertAt::Bottom, Into::into);
    Ok(s.add(&state.ctx, new, at)?)
}

#[tauri::command]
fn edit_task(state: State<'_, AppState>, input: EditTaskInput) -> CmdResult<OpResult> {
    let mut s = state.store.lock().unwrap();
    let due_date = match input.due_date {
        Some(inner) => Some(parse_opt_date(inner.as_deref())?),
        None => None,
    };
    let (clear_urls, add_urls) = match input.urls {
        Some(u) => (true, u),
        None => (false, Vec::new()),
    };
    let patch = TaskPatch {
        title: input.title,
        memo: input.memo,
        due_date,
        clear_urls,
        add_urls,
        ..Default::default()
    };
    Ok(s.edit(&state.ctx, input.id, patch)?)
}

#[tauri::command]
fn move_task(state: State<'_, AppState>, id: TaskId, pos: PosInput) -> CmdResult<OpResult> {
    let mut s = state.store.lock().unwrap();
    Ok(s.move_to(&state.ctx, id, pos.into())?)
}

#[tauri::command]
fn complete_task(state: State<'_, AppState>, id: TaskId) -> CmdResult<OpResult> {
    let mut s = state.store.lock().unwrap();
    Ok(s.complete(&state.ctx, id)?)
}

#[tauri::command]
fn reopen_task(state: State<'_, AppState>, id: TaskId) -> CmdResult<OpResult> {
    let mut s = state.store.lock().unwrap();
    Ok(s.reopen(&state.ctx, id)?)
}

#[tauri::command]
fn hold_task(state: State<'_, AppState>, id: TaskId) -> CmdResult<OpResult> {
    let mut s = state.store.lock().unwrap();
    Ok(s.hold(&state.ctx, id)?)
}

#[tauri::command]
fn resume_task(
    state: State<'_, AppState>,
    id: TaskId,
    pos: Option<PosInput>,
) -> CmdResult<OpResult> {
    let mut s = state.store.lock().unwrap();
    let at = pos.map_or(InsertAt::Bottom, Into::into);
    Ok(s.resume(&state.ctx, id, at)?)
}

#[tauri::command]
fn delete_task(state: State<'_, AppState>, id: TaskId) -> CmdResult<OpResult> {
    let mut s = state.store.lock().unwrap();
    Ok(s.delete(&state.ctx, id)?)
}

#[tauri::command]
fn restore_task(state: State<'_, AppState>, id: TaskId) -> CmdResult<OpResult> {
    let mut s = state.store.lock().unwrap();
    Ok(s.restore(&state.ctx, id)?)
}

#[tauri::command]
fn undo_human(state: State<'_, AppState>) -> CmdResult<Operation> {
    let mut s = state.store.lock().unwrap();
    Ok(s.undo(&Scope::Human)?)
}

#[tauri::command]
fn redo_human(state: State<'_, AppState>) -> CmdResult<Operation> {
    let mut s = state.store.lock().unwrap();
    Ok(s.redo(&Scope::Human)?)
}

#[tauri::command]
fn rollback_session(
    state: State<'_, AppState>,
    session_id: String,
    skip_conflicts: bool,
) -> CmdResult<UndoOutcome> {
    let mut s = state.store.lock().unwrap();
    Ok(s.rollback_session(&session_id, skip_conflicts, false)?)
}

#[tauri::command]
fn restore_to(state: State<'_, AppState>, op_id: OpId) -> CmdResult<Vec<Operation>> {
    let mut s = state.store.lock().unwrap();
    Ok(s.restore_to(op_id)?)
}

#[tauri::command]
fn set_font_size(state: State<'_, AppState>, size: u32) -> CmdResult<u32> {
    let clamped = size.clamp(12, 24);
    let s = state.store.lock().unwrap();
    s.set_setting(SETTING_FONT_SIZE, &clamped)?;
    Ok(clamped)
}

#[tauri::command]
fn run_backup(state: State<'_, AppState>) -> CmdResult<String> {
    let s = state.store.lock().unwrap();
    let dest = perform_backup(&s, &state.db_path)?;
    Ok(dest.display().to_string())
}

#[tauri::command]
fn reveal_db_dir(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    let _ = app.opener().reveal_item_in_dir(&state.db_path);
    Ok(())
}

#[tauri::command]
fn open_external_url(app: AppHandle, url: String) -> CmdResult<()> {
    let _ = app.opener().open_url(&url, None::<&str>);
    Ok(())
}

fn perform_backup(store: &Store, db_path: &std::path::Path) -> Result<PathBuf> {
    let dir = paths::backup_dir(db_path);
    let dest = store.backup_to(&dir, BACKUP_KEEP)?;
    store.set_setting(SETTING_LAST_BACKUP, &Utc::now())?;
    Ok(dest)
}

fn maybe_auto_backup(store: &Store, db_path: &std::path::Path) {
    let should = store
        .get_setting::<DateTime<Utc>>(SETTING_LAST_BACKUP)
        .ok()
        .flatten()
        .is_none_or(|last| Utc::now() - last >= chrono::Duration::hours(24));
    if should {
        let _ = perform_backup(store, db_path);
    }
}

fn start_background_watcher(app: AppHandle) {
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let mut last_ver = state.store.lock().unwrap().data_version().unwrap_or(0);
        let mut ticks: u64 = 0;
        loop {
            std::thread::sleep(Duration::from_millis(500));
            ticks = ticks.wrapping_add(1);
            let s = state.store.lock().unwrap();
            if let Ok(ver) = s.data_version()
                && ver != last_ver
            {
                last_ver = ver;
                let _ = app.emit("db-changed", ());
            }
            // 1 時間ごとに 24 時間経過したか確認して自動バックアップ
            if ticks.is_multiple_of(7200) {
                maybe_auto_backup(&s, &state.db_path);
            }
        }
    });
}

fn main() {
    let db_path = paths::default_db_path();
    let mut store = Store::open(&db_path).expect("failed to open Dandori database");
    let session_id = store
        .start_session(Actor::App, Some("App"))
        .unwrap_or_else(|_| Store::new_session_id());
    maybe_auto_backup(&store, &db_path);

    let state = AppState {
        store: Mutex::new(store),
        ctx: Ctx {
            actor: Actor::App,
            session_id,
        },
        db_path,
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        .setup(|app| {
            start_background_watcher(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            get_history,
            add_task,
            edit_task,
            move_task,
            complete_task,
            reopen_task,
            hold_task,
            resume_task,
            delete_task,
            restore_task,
            undo_human,
            redo_human,
            rollback_session,
            restore_to,
            set_font_size,
            run_backup,
            reveal_db_dir,
            open_external_url,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Dandori application");
}
