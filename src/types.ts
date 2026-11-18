export type Status = "queued" | "on_hold" | "done" | "deleted";
export type Source = "manual" | "recurring";
export type Actor = "app" | "cli" | "mcp";

export interface Task {
  id: number;
  title: string;
  memo: string;
  due_date: string | null;
  urls: string[];
  status: Status;
  source: Source;
  position: string;
  created_at: string;
  updated_at: string;
  completed_at: string | null;
}

export interface Operation {
  id: number;
  op_type: string;
  summary: string;
  actor: Actor;
  session_id: string | null;
  session_label: string | null;
  created_at: string;
  undone: boolean;
}

export interface Session {
  id: string;
  actor: Actor;
  label: string | null;
  started_at: string;
  op_count: number;
  active_op_count: number;
  last_op_at: string | null;
}

export interface Conflict {
  task_id: number;
  title: string;
  operation_id: number;
}

export interface UndoOutcome {
  undone: Operation[];
  skipped: Conflict[];
}

export interface OpResult {
  operation_id: number;
  task: Task;
}

export interface Snapshot {
  queued: Task[];
  on_hold: Task[];
  done: Task[];
  deleted: Task[];
  font_size: number;
  db_path: string;
  last_backup: string | null;
}

export interface HistoryPayload {
  operations: Operation[];
  sessions: Session[];
}

export type PosInput =
  | { kind: "top" }
  | { kind: "bottom" }
  | { kind: "at"; index: number }
  | { kind: "before"; id: number }
  | { kind: "after"; id: number };

export interface CommandError {
  code: string;
  message: string;
  conflicts?: Conflict[] | null;
}
