import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  HistoryPayload,
  Operation,
  OpResult,
  PosInput,
  Snapshot,
  UndoOutcome,
} from "./types";

const isTauri = () =>
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export async function getSnapshot(): Promise<Snapshot> {
  return invoke<Snapshot>("get_snapshot");
}

export async function getHistory(limit = 100): Promise<HistoryPayload> {
  return invoke<HistoryPayload>("get_history", { limit });
}

export async function addTask(input: {
  title: string;
  memo?: string;
  due_date?: string | null;
  urls?: string[];
  pos?: PosInput;
}): Promise<OpResult> {
  return invoke<OpResult>("add_task", { input });
}

export async function editTask(input: {
  id: number;
  title?: string;
  memo?: string;
  due_date?: string | null;
  urls?: string[];
}): Promise<OpResult> {
  return invoke<OpResult>("edit_task", { input });
}

export async function moveTask(id: number, pos: PosInput): Promise<OpResult> {
  return invoke<OpResult>("move_task", { id, pos });
}

export async function completeTask(id: number): Promise<OpResult> {
  return invoke<OpResult>("complete_task", { id });
}

export async function reopenTask(id: number): Promise<OpResult> {
  return invoke<OpResult>("reopen_task", { id });
}

export async function holdTask(id: number): Promise<OpResult> {
  return invoke<OpResult>("hold_task", { id });
}

export async function resumeTask(
  id: number,
  pos?: PosInput,
): Promise<OpResult> {
  return invoke<OpResult>("resume_task", { id, pos });
}

export async function deleteTask(id: number): Promise<OpResult> {
  return invoke<OpResult>("delete_task", { id });
}

export async function restoreTask(id: number): Promise<OpResult> {
  return invoke<OpResult>("restore_task", { id });
}

export async function undoHuman(): Promise<Operation> {
  return invoke<Operation>("undo_human");
}

export async function redoHuman(): Promise<Operation> {
  return invoke<Operation>("redo_human");
}

export async function rollbackSession(
  sessionId: string,
  skipConflicts: boolean,
): Promise<UndoOutcome> {
  return invoke<UndoOutcome>("rollback_session", {
    sessionId,
    skipConflicts,
  });
}

export async function restoreTo(opId: number): Promise<Operation[]> {
  return invoke<Operation[]>("restore_to", { opId });
}

export async function setFontSize(size: number): Promise<number> {
  return invoke<number>("set_font_size", { size });
}

export async function runBackup(): Promise<string> {
  return invoke<string>("run_backup");
}

export async function revealDbDir(): Promise<void> {
  if (isTauri()) {
    await invoke("reveal_db_dir");
  }
}

export async function openExternalUrl(url: string): Promise<void> {
  if (isTauri()) {
    await invoke("open_external_url", { url });
  } else {
    window.open(url, "_blank", "noopener,noreferrer");
  }
}

export async function onDbChanged(cb: () => void): Promise<() => void> {
  if (!isTauri()) return () => {};
  const unlisten = await listen("db-changed", () => cb());
  return unlisten;
}
