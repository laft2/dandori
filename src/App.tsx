import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import * as api from "./api";
import type {
  CommandError,
  Conflict,
  HistoryPayload,
  Operation,
  PosInput,
  Session,
  Snapshot,
  Task,
  UpdateInfo,
} from "./types";

type View = "main" | "done" | "history" | "settings";
type DoneSubTab = "done" | "deleted";

const FONT_SIZES = [
  { label: "小", px: 14 },
  { label: "中", px: 15 },
  { label: "大", px: 17 },
  { label: "特大", px: 19 },
];

function todayStr(): string {
  const d = new Date();
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

function formatShortDateTime(iso: string | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  return `${m}/${day} ${hh}:${mm}`;
}

function monthKey(iso: string | null): string {
  if (!iso) return "不明";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso.slice(0, 7);
  return `${d.getFullYear()}年${String(d.getMonth() + 1).padStart(2, "0")}月`;
}

function DueBadge({ due }: { due: string | null }) {
  if (!due) return null;
  const [, m, d] = due.split("-");
  const label = `${Number(m)}/${Number(d)}`;
  const today = todayStr();
  if (due < today) {
    return (
      <span
        className="text-xs font-medium px-2 py-0.5 rounded whitespace-nowrap"
        style={{ color: "var(--danger)", background: "var(--danger-weak)" }}
      >
        {label} 期限切れ
      </span>
    );
  }
  if (due === today) {
    return (
      <span
        className="text-xs font-medium px-2 py-0.5 rounded whitespace-nowrap"
        style={{ color: "var(--warn)", background: "var(--warn-weak)" }}
      >
        今日まで
      </span>
    );
  }
  return (
    <span
      className="text-xs px-2 py-0.5 rounded dd-sub whitespace-nowrap"
      style={{ background: "#F0EFEA" }}
    >
      {label}まで
    </span>
  );
}

export function App() {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [history, setHistory] = useState<HistoryPayload>({
    operations: [],
    sessions: [],
  });
  const [view, setView] = useState<View>("main");
  const [doneTab, setDoneTab] = useState<DoneSubTab>("done");
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [holdOpen, setHoldOpen] = useState(false);

  // 追加フォーム
  const [newTitle, setNewTitle] = useState("");
  const [newPos, setNewPos] = useState<string>("bottom");
  const newInputRef = useRef<HTMLInputElement>(null);

  // 検索
  const [queueQuery, setQueueQuery] = useState("");
  const [showQueueSearch, setShowQueueSearch] = useState(false);
  const queueSearchRef = useRef<HTMLInputElement>(null);
  const [doneQuery, setDoneQuery] = useState("");
  const doneSearchRef = useRef<HTMLInputElement>(null);

  // ドラッグ・右クリックメニュー
  const [dragId, setDragId] = useState<number | null>(null);
  const [overId, setOverId] = useState<number | null>(null);
  const [ctxMenu, setCtxMenu] = useState<{
    x: number;
    y: number;
    task: Task;
  } | null>(null);

  // トースト・ハイライト・ダイアログ
  const [toast, setToast] = useState<{ msg: string; canUndo: boolean } | null>(
    null,
  );
  const toastTimer = useRef<number | null>(null);
  const [highlightedIds, setHighlightedIds] = useState<Set<number>>(new Set());
  const prevUpdatedMap = useRef<Map<number, string>>(new Map());

  const [conflictModal, setConflictModal] = useState<{
    sessionId: string;
    conflicts: Conflict[];
  } | null>(null);
  const [restoreConfirmOp, setRestoreConfirmOp] = useState<Operation | null>(
    null,
  );
  const [availableUpdate, setAvailableUpdate] = useState<UpdateInfo | null>(
    null,
  );

  const showToast = useCallback((msg: string, canUndo = true) => {
    setToast({ msg, canUndo });
    if (toastTimer.current) window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), 4000);
  }, []);

  const handleError = useCallback(
    (err: unknown) => {
      const ce = err as CommandError;
      showToast(ce?.message || String(err), false);
    },
    [showToast],
  );

  const refresh = useCallback(async (fromExternal = false) => {
    try {
      const next = await api.getSnapshot();
      if (fromExternal) {
        const changed = new Set<number>();
        for (const t of [...next.queued, ...next.on_hold]) {
          const prev = prevUpdatedMap.current.get(t.id);
          if (!prev || prev !== t.updated_at) {
            changed.add(t.id);
          }
        }
        if (changed.size > 0) {
          setHighlightedIds(changed);
          window.setTimeout(() => setHighlightedIds(new Set()), 1600);
        }
      }
      const nextMap = new Map<number, string>();
      for (const t of [
        ...next.queued,
        ...next.on_hold,
        ...next.done,
        ...next.deleted,
      ]) {
        nextMap.set(t.id, t.updated_at);
      }
      prevUpdatedMap.current = nextMap;
      setSnap(next);
    } catch (e) {
      console.error(e);
    }
  }, []);

  const refreshHistory = useCallback(async () => {
    try {
      setHistory(await api.getHistory(120));
    } catch (e) {
      console.error(e);
    }
  }, []);

  useEffect(() => {
    refresh(false);
    // 起動時にバックグラウンドで更新有無を確認（オフラインや公開鍵未設定時は無視）
    api.checkUpdate().then((u) => {
      if (u) setAvailableUpdate(u);
    }).catch(() => {});
    let cleanup: (() => void) | undefined;
    api.onDbChanged(() => {
      refresh(true);
      refreshHistory();
    }).then((fn) => {
      cleanup = fn;
    });
    return () => cleanup?.();
  }, [refresh, refreshHistory]);

  useEffect(() => {
    if (view === "history") {
      refreshHistory();
    }
  }, [view, refreshHistory]);

  // 選択中のタスク
  const selectedTask = useMemo(() => {
    if (!snap || selectedId == null) return null;
    return (
      [...snap.queued, ...snap.on_hold, ...snap.done, ...snap.deleted].find(
        (t) => t.id === selectedId,
      ) ?? null
    );
  }, [snap, selectedId]);

  // ----- 操作ハンドラ ------------------------------------------------------

  const handleUndo = useCallback(async () => {
    try {
      const op = await api.undoHuman();
      await refresh(false);
      if (view === "history") await refreshHistory();
      showToast(`取り消しました：${op.summary}`, false);
    } catch (e) {
      handleError(e);
    }
  }, [refresh, refreshHistory, view, showToast, handleError]);

  const handleRedo = useCallback(async () => {
    try {
      const op = await api.redoHuman();
      await refresh(false);
      if (view === "history") await refreshHistory();
      showToast(`やり直しました：${op.summary}`, true);
    } catch (e) {
      handleError(e);
    }
  }, [refresh, refreshHistory, view, showToast, handleError]);

  const changeFontSize = useCallback(
    async (size: number) => {
      try {
        const next = await api.setFontSize(size);
        setSnap((s) => (s ? { ...s, font_size: next } : s));
      } catch (e) {
        handleError(e);
      }
    },
    [handleError],
  );

  // グローバルショートカット
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = document.activeElement;
      const typing =
        el instanceof HTMLInputElement ||
        el instanceof HTMLTextAreaElement ||
        el instanceof HTMLSelectElement;
      const mod = e.metaKey || e.ctrlKey;

      if (mod && e.key.toLowerCase() === "n") {
        e.preventDefault();
        setView("main");
        window.setTimeout(() => newInputRef.current?.focus(), 0);
        return;
      }
      if (mod && e.key.toLowerCase() === "f") {
        e.preventDefault();
        if (view === "done") {
          doneSearchRef.current?.focus();
        } else {
          setView("main");
          setShowQueueSearch(true);
          window.setTimeout(() => queueSearchRef.current?.focus(), 0);
        }
        return;
      }
      if (mod && e.key === ",") {
        e.preventDefault();
        setView("settings");
        return;
      }
      if (mod && (e.key === "+" || e.key === "=")) {
        e.preventDefault();
        if (snap) changeFontSize(snap.font_size + 1);
        return;
      }
      if (mod && e.key === "-") {
        e.preventDefault();
        if (snap) changeFontSize(snap.font_size - 1);
        return;
      }
      if (mod && e.key === "0") {
        e.preventDefault();
        changeFontSize(15);
        return;
      }
      if (!typing && mod && e.key.toLowerCase() === "z") {
        e.preventDefault();
        if (e.shiftKey) {
          handleRedo();
        } else {
          handleUndo();
        }
        return;
      }
      if (!typing && mod && e.key.toLowerCase() === "y") {
        e.preventDefault();
        handleRedo();
        return;
      }
      if (e.key === "Escape") {
        if (ctxMenu) {
          setCtxMenu(null);
        } else if (!typing && selectedId != null) {
          setSelectedId(null);
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [
    view,
    snap,
    selectedId,
    ctxMenu,
    handleUndo,
    handleRedo,
    changeFontSize,
  ]);

  useEffect(() => {
    const closeMenu = () => setCtxMenu(null);
    window.addEventListener("click", closeMenu);
    return () => window.removeEventListener("click", closeMenu);
  }, []);

  const handleAdd = async () => {
    const title = newTitle.trim();
    if (!title) return;
    let pos: PosInput = { kind: "bottom" };
    if (newPos === "top") pos = { kind: "top" };
    else if (newPos !== "bottom") pos = { kind: "at", index: Number(newPos) };
    try {
      const res = await api.addTask({ title, pos });
      setNewTitle("");
      setNewPos("bottom"); // 毎回デフォルトの末尾に戻す
      await refresh(false);
      showToast(`「${res.task.title}」を追加しました`);
    } catch (e) {
      handleError(e);
    }
  };

  const handleComplete = async (t: Task) => {
    try {
      await api.completeTask(t.id);
      if (selectedId === t.id) setSelectedId(null);
      await refresh(false);
      showToast(`「${t.title}」を完了しました`);
    } catch (e) {
      handleError(e);
    }
  };

  const handleHold = async (t: Task) => {
    try {
      await api.holdTask(t.id);
      await refresh(false);
      showToast(`「${t.title}」を保留にしました`);
    } catch (e) {
      handleError(e);
    }
  };

  const handleResume = async (t: Task, pos: PosInput = { kind: "bottom" }) => {
    try {
      await api.resumeTask(t.id, pos);
      await refresh(false);
      showToast(`「${t.title}」をキューに戻しました`);
    } catch (e) {
      handleError(e);
    }
  };

  const handleDelete = async (t: Task) => {
    try {
      await api.deleteTask(t.id);
      if (selectedId === t.id) setSelectedId(null);
      await refresh(false);
      showToast(`「${t.title}」を削除しました`);
    } catch (e) {
      handleError(e);
    }
  };

  const handleReopenOrRestore = async (t: Task) => {
    try {
      if (t.status === "done") {
        await api.reopenTask(t.id);
        showToast(`「${t.title}」を未完了に戻しました`);
      } else {
        await api.restoreTask(t.id);
        showToast(`「${t.title}」を復元しました`);
      }
      await refresh(false);
    } catch (e) {
      handleError(e);
    }
  };

  const handleMove = async (id: number, pos: PosInput) => {
    try {
      const res = await api.moveTask(id, pos);
      await refresh(false);
      showToast(`「${res.task.title}」を移動しました`);
    } catch (e) {
      handleError(e);
    }
  };

  const handleRollback = async (sessionId: string, skipConflicts: boolean) => {
    try {
      const out = await api.rollbackSession(sessionId, skipConflicts);
      setConflictModal(null);
      await refresh(false);
      await refreshHistory();
      showToast(
        `AIセッションの操作を ${out.undone.length} 件巻き戻しました`,
        false,
      );
    } catch (e) {
      const ce = e as CommandError;
      if (ce?.code === "conflict" && ce.conflicts?.length) {
        setConflictModal({ sessionId, conflicts: ce.conflicts });
      } else {
        handleError(e);
      }
    }
  };

  const handleRestoreTo = async (op: Operation) => {
    try {
      const undone = await api.restoreTo(op.id);
      setRestoreConfirmOp(null);
      await refresh(false);
      await refreshHistory();
      showToast(`${undone.length} 件の操作を取り消しました`, false);
    } catch (e) {
      handleError(e);
    }
  };

  if (!snap) {
    return <div className="p-8 dd-sub">読み込み中…</div>;
  }

  const fontSize = snap.font_size || 15;
  const topTask = snap.queued[0] ?? null;
  const restQueue = snap.queued.slice(1).filter((t) => {
    if (!queueQuery.trim()) return true;
    const q = queueQuery.toLowerCase();
    return (
      t.title.toLowerCase().includes(q) || t.memo.toLowerCase().includes(q)
    );
  });

  return (
    <div
      className="min-h-screen flex flex-col select-none"
      style={{ fontSize: `${fontSize}px` }}
    >
      {/* ヘッダー */}
      <header className="flex items-center justify-between px-6 py-3 border-b dd-border dd-surface shrink-0">
        <div className="flex items-center gap-3">
          <div className="font-semibold tracking-wide">Dandori</div>
          {availableUpdate && (
            <button
              onClick={() => {
                setView("settings");
                setSelectedId(null);
              }}
              className="text-xs font-medium px-2 py-0.5 rounded-full cursor-pointer"
              style={{
                background: "var(--accent-weak)",
                color: "var(--accent)",
              }}
            >
              v{availableUpdate.version} あり
            </button>
          )}
        </div>
        <nav className="flex items-center gap-1 text-sm">
          {(
            [
              ["main", "キュー"],
              ["done", "完了済み"],
              ["history", "履歴"],
              ["settings", "設定"],
            ] as const
          ).map(([k, label]) => {
            const active = view === k;
            return (
              <button
                key={k}
                onClick={() => {
                  setView(k);
                  if (k !== "main" && k !== "done") setSelectedId(null);
                }}
                className="px-3 py-1 rounded-md cursor-pointer transition-colors"
                style={{
                  background: active ? "var(--accent-weak)" : "transparent",
                  color: active ? "var(--accent)" : "var(--sub)",
                  fontWeight: active ? 600 : 400,
                }}
              >
                {label}
              </button>
            );
          })}
        </nav>
      </header>

      {/* 本体 */}
      <div className="flex flex-1 min-h-0">
        {/* メイン領域 */}
        <main className="flex-1 overflow-y-auto px-8 py-6 select-text">
          {view === "main" && (
            <div className="max-w-[640px] mx-auto">
              <div className="text-sm font-medium dd-sub mb-2">今やること</div>
              <div
                className={`dd-surface border dd-border rounded-xl p-6 mb-6 shadow-xs ${
                  topTask && highlightedIds.has(topTask.id)
                    ? "dd-highlight"
                    : ""
                }`}
              >
                {topTask ? (
                  <div className="flex items-start gap-4">
                    <button
                      onClick={() => handleComplete(topTask)}
                      title="完了"
                      className="mt-1 w-7 h-7 rounded-full border-2 shrink-0 cursor-pointer hover:bg-[var(--accent-weak)]"
                      style={{ borderColor: "var(--accent)" }}
                    />
                    <div className="flex-1 min-w-0">
                      <div
                        onClick={() => setSelectedId(topTask.id)}
                        className="text-xl font-semibold leading-snug cursor-pointer"
                      >
                        {topTask.title}
                      </div>
                      <div className="flex flex-wrap items-center gap-2 mt-2">
                        <DueBadge due={topTask.due_date} />
                        {topTask.source === "recurring" && (
                          <span className="text-xs px-2 py-0.5 rounded border dd-border dd-sub">
                            定期
                          </span>
                        )}
                        {topTask.urls.map((u) => (
                          <button
                            key={u}
                            onClick={() => api.openExternalUrl(u)}
                            className="text-sm dd-accent underline underline-offset-2 truncate max-w-[280px] cursor-pointer"
                            title={u}
                          >
                            🔗 {u.replace(/^https?:\/\//, "")}
                          </button>
                        ))}
                      </div>
                      {topTask.memo && (
                        <div className="dd-md text-sm mt-3 dd-sub border-t dd-border pt-3">
                          <ReactMarkdown remarkPlugins={[remarkGfm]}>
                            {topTask.memo}
                          </ReactMarkdown>
                        </div>
                      )}
                      <div className="flex gap-2 mt-4">
                        <button
                          onClick={() => handleComplete(topTask)}
                          className="px-4 py-1.5 rounded-md text-sm font-medium text-white cursor-pointer"
                          style={{ background: "var(--accent)" }}
                        >
                          完了
                        </button>
                        <button
                          onClick={() => handleHold(topTask)}
                          className="px-4 py-1.5 rounded-md text-sm border dd-border cursor-pointer"
                        >
                          保留
                        </button>
                        <button
                          onClick={() => setSelectedId(topTask.id)}
                          className="px-4 py-1.5 rounded-md text-sm border dd-border cursor-pointer"
                        >
                          詳細
                        </button>
                      </div>
                    </div>
                  </div>
                ) : (
                  <div className="text-center py-6 dd-sub">
                    🎉 キューは空です
                  </div>
                )}
              </div>

              {/* 追加フォーム */}
              <div className="dd-surface border dd-border rounded-lg flex items-center gap-2 px-3 py-2 mb-4">
                <span className="dd-accent text-xl leading-none">＋</span>
                <input
                  ref={newInputRef}
                  value={newTitle}
                  onChange={(e) => setNewTitle(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") handleAdd();
                    if (e.key === "Escape") e.currentTarget.blur();
                  }}
                  placeholder="タスクを追加…"
                  className="flex-1 bg-transparent py-1 outline-none"
                />
                <select
                  value={newPos}
                  onChange={(e) => setNewPos(e.target.value)}
                  className="text-sm border dd-border rounded-md px-2 py-1 bg-white outline-none"
                >
                  <option value="bottom">末尾に追加</option>
                  <option value="top">先頭に追加</option>
                  {snap.queued.map((_, idx) =>
                    idx >= 1 ? (
                      <option key={idx + 1} value={String(idx + 1)}>
                        {idx + 1}番目に追加
                      </option>
                    ) : null,
                  )}
                </select>
                <span className="text-xs border dd-border rounded px-1.5 py-0.5 dd-sub">
                  ⌘N
                </span>
              </div>

              {/* 検索バー（⌘F で表示） */}
              {showQueueSearch && (
                <div className="flex items-center gap-2 mb-3">
                  <input
                    ref={queueSearchRef}
                    value={queueQuery}
                    onChange={(e) => setQueueQuery(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Escape") {
                        setQueueQuery("");
                        setShowQueueSearch(false);
                      }
                    }}
                    placeholder="キュー内を検索…"
                    className="flex-1 dd-surface border dd-border rounded-md px-3 py-1 text-sm outline-none"
                  />
                  <button
                    onClick={() => {
                      setQueueQuery("");
                      setShowQueueSearch(false);
                    }}
                    className="text-sm dd-sub px-2 cursor-pointer"
                  >
                    閉じる
                  </button>
                </div>
              )}

              {/* 次以降 */}
              <div className="text-sm font-medium dd-sub mb-1 flex justify-between">
                <span>次以降</span>
                <span>
                  {snap.queued.length > 1 ? `${snap.queued.length - 1}件` : ""}
                </span>
              </div>
              <div className="dd-surface border dd-border rounded-lg divide-y divide-[var(--line)]">
                {restQueue.length > 0 ? (
                  restQueue.map((t) => {
                    const qIndex =
                      snap.queued.findIndex((q) => q.id === t.id) + 1;
                    const isOver = overId === t.id && dragId !== t.id;
                    return (
                      <div
                        key={t.id}
                        draggable
                        onDragStart={() => setDragId(t.id)}
                        onDragEnd={() => {
                          setDragId(null);
                          setOverId(null);
                        }}
                        onDragOver={(e) => {
                          e.preventDefault();
                          setOverId(t.id);
                        }}
                        onDragLeave={() => {
                          if (overId === t.id) setOverId(null);
                        }}
                        onDrop={(e) => {
                          e.preventDefault();
                          setOverId(null);
                          if (dragId != null && dragId !== t.id) {
                            handleMove(dragId, { kind: "before", id: t.id });
                          }
                        }}
                        onContextMenu={(e) => {
                          e.preventDefault();
                          setCtxMenu({ x: e.clientX, y: e.clientY, task: t });
                        }}
                        className={`flex items-center gap-3 px-3 py-2.5 hover:bg-[#FAFAF8] ${
                          highlightedIds.has(t.id) ? "dd-highlight" : ""
                        }`}
                        style={{
                          boxShadow: isOver
                            ? "inset 0 2px 0 var(--accent)"
                            : undefined,
                          opacity: dragId === t.id ? 0.4 : 1,
                        }}
                      >
                        <span
                          className="dd-sub cursor-grab select-none"
                          title="ドラッグで並べ替え"
                        >
                          ⋮⋮
                        </span>
                        <span className="text-xs dd-sub w-5 text-right">
                          {qIndex}
                        </span>
                        <button
                          onClick={() => handleComplete(t)}
                          title="完了"
                          className="w-5 h-5 rounded-full border-2 shrink-0 cursor-pointer hover:bg-[var(--accent-weak)]"
                          style={{ borderColor: "#B9B6AE" }}
                        />
                        <span
                          onClick={() => setSelectedId(t.id)}
                          className="flex-1 truncate cursor-pointer"
                        >
                          {t.title}
                        </span>
                        <DueBadge due={t.due_date} />
                        {t.urls.length > 0 && (
                          <span className="text-xs dd-sub">
                            🔗{t.urls.length}
                          </span>
                        )}
                        {t.memo && <span className="text-xs dd-sub">📝</span>}
                      </div>
                    );
                  })
                ) : (
                  <div className="px-3 py-3 text-sm dd-sub">なし</div>
                )}
              </div>

              {/* 保留中 */}
              <button
                onClick={() => setHoldOpen((v) => !v)}
                className="mt-5 text-sm dd-sub flex items-center gap-1 cursor-pointer"
              >
                <span>{holdOpen ? "▾" : "▸"}</span>
                <span>保留中 ({snap.on_hold.length})</span>
              </button>
              {holdOpen && (
                <div className="mt-2 dd-surface border dd-border rounded-lg divide-y divide-[var(--line)]">
                  {snap.on_hold.length > 0 ? (
                    snap.on_hold.map((t) => (
                      <div
                        key={t.id}
                        className="flex items-center gap-3 px-3 py-2.5"
                      >
                        <span
                          onClick={() => setSelectedId(t.id)}
                          className="flex-1 dd-sub truncate cursor-pointer"
                        >
                          {t.title}
                        </span>
                        <button
                          onClick={() => handleResume(t, { kind: "bottom" })}
                          className="text-sm dd-accent cursor-pointer"
                        >
                          キューに戻す
                        </button>
                      </div>
                    ))
                  ) : (
                    <div className="px-3 py-3 text-sm dd-sub">なし</div>
                  )}
                </div>
              )}
              <div className="h-10" />
            </div>
          )}

          {view === "done" && (
            <DoneView
              tasks={doneTab === "done" ? snap.done : snap.deleted}
              tab={doneTab}
              setTab={setDoneTab}
              query={doneQuery}
              setQuery={setDoneQuery}
              searchRef={doneSearchRef}
              onSelect={(t) => setSelectedId(t.id)}
              onRestore={handleReopenOrRestore}
            />
          )}

          {view === "history" && (
            <HistoryView
              payload={history}
              onRollback={(sid) => handleRollback(sid, false)}
              onRestoreTo={(op) => setRestoreConfirmOp(op)}
            />
          )}

          {view === "settings" && (
            <SettingsView
              snap={snap}
              availableUpdate={availableUpdate}
              onUpdateFound={setAvailableUpdate}
              onFontSize={changeFontSize}
              onBackup={async () => {
                try {
                  await api.runBackup();
                  await refresh(false);
                  showToast("バックアップを作成しました", false);
                } catch (e) {
                  handleError(e);
                }
              }}
              onRevealDb={() => api.revealDbDir()}
            />
          )}
        </main>

        {/* 詳細パネル */}
        {selectedTask && (
          <DetailPanel
            key={selectedTask.id}
            task={selectedTask}
            onClose={() => setSelectedId(null)}
            onSave={async (patch, label) => {
              try {
                await api.editTask({ id: selectedTask.id, ...patch });
                await refresh(false);
                showToast(`${label}を変更しました`);
              } catch (e) {
                handleError(e);
              }
            }}
            onComplete={() => handleComplete(selectedTask)}
            onHold={() => handleHold(selectedTask)}
            onResume={() => handleResume(selectedTask)}
            onReopenOrRestore={() => handleReopenOrRestore(selectedTask)}
            onDelete={() => handleDelete(selectedTask)}
          />
        )}
      </div>

      {/* 右クリックメニュー */}
      {ctxMenu && (
        <div
          className="fixed dd-surface border dd-border rounded-lg shadow-md py-1 text-sm z-40"
          style={{ left: ctxMenu.x, top: ctxMenu.y, minWidth: 160 }}
        >
          <button
            onClick={() => {
              handleMove(ctxMenu.task.id, { kind: "top" });
              setCtxMenu(null);
            }}
            className="w-full text-left px-3 py-1.5 hover:bg-[#FAFAF8] cursor-pointer"
          >
            先頭へ移動
          </button>
          <button
            onClick={() => {
              handleMove(ctxMenu.task.id, { kind: "bottom" });
              setCtxMenu(null);
            }}
            className="w-full text-left px-3 py-1.5 hover:bg-[#FAFAF8] cursor-pointer"
          >
            末尾へ移動
          </button>
          <button
            onClick={() => {
              handleHold(ctxMenu.task);
              setCtxMenu(null);
            }}
            className="w-full text-left px-3 py-1.5 hover:bg-[#FAFAF8] cursor-pointer"
          >
            保留にする
          </button>
          <div className="border-t dd-border my-1" />
          <button
            onClick={() => {
              handleDelete(ctxMenu.task);
              setCtxMenu(null);
            }}
            className="w-full text-left px-3 py-1.5 hover:bg-[#FAFAF8] cursor-pointer"
            style={{ color: "var(--danger)" }}
          >
            削除
          </button>
        </div>
      )}

      {/* 競合ダイアログ（AIセッション巻き戻し時） */}
      {conflictModal && (
        <div className="fixed inset-0 bg-black/30 flex items-center justify-center z-50">
          <div className="dd-surface border dd-border rounded-xl p-6 max-w-md w-full shadow-lg">
            <h3 className="font-semibold text-lg mb-2">
              後から変更されたタスクがあります
            </h3>
            <p className="text-sm dd-sub mb-3">
              以下のタスクはこのセッションの後に変更されているため、そのまま巻き戻すと変更が失われます。
            </p>
            <ul className="text-sm border dd-border rounded-md p-3 mb-4 space-y-1 max-h-40 overflow-y-auto">
              {conflictModal.conflicts.map((c) => (
                <li key={`${c.operation_id}-${c.task_id}`}>
                  #{c.task_id} {c.title}
                </li>
              ))}
            </ul>
            <div className="flex justify-end gap-2">
              <button
                onClick={() => setConflictModal(null)}
                className="px-3 py-1.5 rounded-md text-sm border dd-border cursor-pointer"
              >
                中止
              </button>
              <button
                onClick={() =>
                  handleRollback(conflictModal.sessionId, true)
                }
                className="px-3 py-1.5 rounded-md text-sm font-medium text-white cursor-pointer"
                style={{ background: "var(--accent)" }}
              >
                競合しているタスクを除いて戻す
              </button>
            </div>
          </div>
        </div>
      )}

      {/* 任意時点への復元確認ダイアログ */}
      {restoreConfirmOp && (
        <div className="fixed inset-0 bg-black/30 flex items-center justify-center z-50">
          <div className="dd-surface border dd-border rounded-xl p-6 max-w-md w-full shadow-lg">
            <h3 className="font-semibold text-lg mb-2">
              この時点まで戻しますか？
            </h3>
            <p className="text-sm dd-sub mb-4">
              「{restoreConfirmOp.summary}」より後に行われたすべての操作（自分・AIの両方）を取り消します。
            </p>
            <div className="flex justify-end gap-2">
              <button
                onClick={() => setRestoreConfirmOp(null)}
                className="px-3 py-1.5 rounded-md text-sm border dd-border cursor-pointer"
              >
                キャンセル
              </button>
              <button
                onClick={() => handleRestoreTo(restoreConfirmOp)}
                className="px-3 py-1.5 rounded-md text-sm font-medium text-white cursor-pointer"
                style={{ background: "var(--danger)" }}
              >
                この時点まで戻す
              </button>
            </div>
          </div>
        </div>
      )}

      {/* トースト */}
      {toast && (
        <div
          className="fixed bottom-6 left-1/2 -translate-x-1/2 rounded-lg px-4 py-2 text-sm shadow-lg flex items-center gap-3 z-40"
          style={{ background: "#2B2B2B", color: "#fff" }}
        >
          <span>{toast.msg}</span>
          {toast.canUndo && (
            <button
              onClick={handleUndo}
              className="font-semibold cursor-pointer"
              style={{ color: "#9DB5FF" }}
            >
              元に戻す ⌘Z
            </button>
          )}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// 完了済み画面
// ---------------------------------------------------------------------------

function DoneView({
  tasks,
  tab,
  setTab,
  query,
  setQuery,
  searchRef,
  onSelect,
  onRestore,
}: {
  tasks: Task[];
  tab: DoneSubTab;
  setTab: (t: DoneSubTab) => void;
  query: string;
  setQuery: (q: string) => void;
  searchRef: React.RefObject<HTMLInputElement | null>;
  onSelect: (t: Task) => void;
  onRestore: (t: Task) => void;
}) {
  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return tasks.filter(
      (t) =>
        !q ||
        t.title.toLowerCase().includes(q) ||
        t.memo.toLowerCase().includes(q),
    );
  }, [tasks, query]);

  const groups = useMemo(() => {
    const map = new Map<string, Task[]>();
    for (const t of filtered) {
      const k = monthKey(tab === "done" ? t.completed_at : t.updated_at);
      const arr = map.get(k) ?? [];
      arr.push(t);
      map.set(k, arr);
    }
    return Array.from(map.entries());
  }, [filtered, tab]);

  return (
    <div className="max-w-[640px] mx-auto">
      <div className="flex items-center justify-between mb-4">
        <h2 className="font-semibold text-lg">完了済み</h2>
        <div className="text-sm border dd-border rounded-md overflow-hidden dd-surface">
          <button
            onClick={() => setTab("done")}
            className="px-3 py-1 cursor-pointer"
            style={{
              background: tab === "done" ? "var(--accent-weak)" : undefined,
            }}
          >
            完了
          </button>
          <button
            onClick={() => setTab("deleted")}
            className="px-3 py-1 border-l dd-border cursor-pointer"
            style={{
              background: tab === "deleted" ? "var(--accent-weak)" : undefined,
            }}
          >
            削除済み
          </button>
        </div>
      </div>

      <input
        ref={searchRef}
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder="タイトル・メモを検索…"
        className="w-full dd-surface border dd-border rounded-md px-3 py-1.5 text-sm mb-4 outline-none"
      />

      {groups.length > 0 ? (
        <div className="space-y-5">
          {groups.map(([m, list]) => (
            <div key={m}>
              <div className="text-sm font-medium dd-sub mb-1">
                {m} <span className="font-normal">{list.length}件</span>
              </div>
              <div className="dd-surface border dd-border rounded-lg divide-y divide-[var(--line)]">
                {list.map((t) => (
                  <div
                    key={t.id}
                    className="flex items-center gap-3 px-3 py-2.5"
                  >
                    <span className={tab === "done" ? "dd-accent" : "dd-sub"}>
                      {tab === "done" ? "✓" : "🗑"}
                    </span>
                    <span
                      onClick={() => onSelect(t)}
                      className={`flex-1 truncate cursor-pointer dd-sub ${
                        tab === "done" ? "line-through" : ""
                      }`}
                    >
                      {t.title}
                    </span>
                    {t.source === "recurring" && (
                      <span className="text-xs px-2 py-0.5 rounded border dd-border dd-sub">
                        定期
                      </span>
                    )}
                    <span className="text-xs dd-sub whitespace-nowrap">
                      {formatShortDateTime(
                        tab === "done" ? t.completed_at : t.updated_at,
                      )}
                    </span>
                    <button
                      onClick={() => onRestore(t)}
                      className="text-sm dd-accent whitespace-nowrap cursor-pointer"
                    >
                      {tab === "done" ? "未完了に戻す" : "復元"}
                    </button>
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>
      ) : (
        <div className="text-sm dd-sub">該当するタスクはありません</div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// 履歴画面
// ---------------------------------------------------------------------------

type HistoryBlock =
  | { kind: "human"; op: Operation }
  | {
      kind: "session";
      sessionId: string;
      label: string;
      ops: Operation[];
      session?: Session;
    };

function HistoryView({
  payload,
  onRollback,
  onRestoreTo,
}: {
  payload: HistoryPayload;
  onRollback: (sessionId: string) => void;
  onRestoreTo: (op: Operation) => void;
}) {
  const blocks = useMemo<HistoryBlock[]>(() => {
    const sessMap = new Map<string, Session>();
    for (const s of payload.sessions) sessMap.set(s.id, s);

    const out: HistoryBlock[] = [];
    for (const op of payload.operations) {
      if (op.actor === "app" || !op.session_id) {
        out.push({ kind: "human", op });
        continue;
      }
      const prev = out[out.length - 1];
      if (prev && prev.kind === "session" && prev.sessionId === op.session_id) {
        prev.ops.push(op);
      } else {
        const s = sessMap.get(op.session_id);
        out.push({
          kind: "session",
          sessionId: op.session_id,
          label: op.session_label || s?.label || `AIセッション (${op.session_id})`,
          ops: [op],
          session: s,
        });
      }
    }
    return out;
  }, [payload]);

  return (
    <div className="max-w-[640px] mx-auto">
      <h2 className="font-semibold text-lg mb-1">履歴</h2>
      <p className="text-sm dd-sub mb-4">
        AIの操作はセッションごとにまとめて表示します。差分は表示しません。
      </p>
      {blocks.length > 0 ? (
        <div className="space-y-3">
          {blocks.map((b, i) =>
            b.kind === "session" ? (
              <div
                key={`${b.sessionId}-${i}`}
                className="dd-surface border dd-border rounded-lg overflow-hidden"
              >
                <div
                  className="flex items-center justify-between px-4 py-2.5 border-b dd-border"
                  style={{ background: "var(--accent-weak)" }}
                >
                  <div className="flex items-center gap-2 min-w-0">
                    <span
                      className="text-xs font-semibold px-2 py-0.5 rounded text-white shrink-0"
                      style={{ background: "var(--accent)" }}
                    >
                      AI
                    </span>
                    <span className="font-medium truncate">{b.label}</span>
                    <span className="text-sm dd-sub shrink-0">
                      {formatShortDateTime(b.ops[0].created_at)}・{b.ops.length}
                      件
                    </span>
                  </div>
                  {b.ops.some((o) => !o.undone) && (
                    <button
                      onClick={() => onRollback(b.sessionId)}
                      className="text-sm border dd-border rounded-md px-3 py-1 bg-white shrink-0 cursor-pointer"
                    >
                      このセッションを巻き戻す
                    </button>
                  )}
                </div>
                <div className="divide-y divide-[var(--line)]">
                  {b.ops.map((o) => (
                    <div
                      key={o.id}
                      className={`px-4 py-2 text-sm flex justify-between ${
                        o.undone ? "line-through dd-sub" : ""
                      }`}
                    >
                      <span>{o.summary}</span>
                      <span className="text-xs dd-sub">
                        {formatShortDateTime(o.created_at)}
                      </span>
                    </div>
                  ))}
                </div>
              </div>
            ) : (
              <div
                key={b.op.id}
                className="dd-surface border dd-border rounded-lg flex items-center gap-3 px-4 py-2.5 text-sm"
              >
                <span className="text-xs px-2 py-0.5 rounded border dd-border dd-sub">
                  自分
                </span>
                <span
                  className={`flex-1 ${
                    b.op.undone ? "line-through dd-sub" : ""
                  }`}
                >
                  {b.op.summary}
                </span>
                <span className="text-xs dd-sub">
                  {formatShortDateTime(b.op.created_at)}
                </span>
                {!b.op.undone && (
                  <button
                    onClick={() => onRestoreTo(b.op)}
                    className="text-xs dd-sub underline cursor-pointer"
                    title="この操作の時点まで戻す（非常用）"
                  >
                    この時点まで戻す
                  </button>
                )}
              </div>
            ),
          )}
        </div>
      ) : (
        <div className="text-sm dd-sub">操作履歴はまだありません</div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// 設定画面
// ---------------------------------------------------------------------------

function SettingsView({
  snap,
  availableUpdate,
  onUpdateFound,
  onFontSize,
  onBackup,
  onRevealDb,
}: {
  snap: Snapshot;
  availableUpdate: UpdateInfo | null;
  onUpdateFound: (u: UpdateInfo | null) => void;
  onFontSize: (px: number) => void;
  onBackup: () => void;
  onRevealDb: () => void;
}) {
  const [updateState, setUpdateState] = useState<
    "idle" | "checking" | "up_to_date" | "installing" | "error"
  >("idle");
  const [updateError, setUpdateError] = useState<string | null>(null);

  const handleCheckUpdate = async () => {
    setUpdateState("checking");
    setUpdateError(null);
    try {
      const info = await api.checkUpdate();
      onUpdateFound(info);
      setUpdateState(info ? "idle" : "up_to_date");
    } catch (e) {
      const ce = e as CommandError;
      setUpdateError(ce?.message || String(e));
      setUpdateState("error");
    }
  };

  const handleInstallUpdate = async () => {
    setUpdateState("installing");
    setUpdateError(null);
    try {
      await api.installUpdate();
    } catch (e) {
      const ce = e as CommandError;
      setUpdateError(ce?.message || String(e));
      setUpdateState("error");
    }
  };

  return (
    <div className="max-w-[640px] mx-auto space-y-5">
      <h2 className="font-semibold text-lg">設定</h2>
      <div className="dd-surface border dd-border rounded-lg p-5 space-y-4">
        <div className="flex justify-between items-center">
          <div>
            <div className="font-medium">文字サイズ</div>
            <div className="text-sm dd-sub">⌘+ / ⌘- / ⌘0 でも変更できます</div>
          </div>
          <div className="flex gap-1">
            {FONT_SIZES.map((f) => {
              const active = snap.font_size === f.px;
              return (
                <button
                  key={f.px}
                  onClick={() => onFontSize(f.px)}
                  className="border dd-border rounded-md px-3 py-1 text-sm cursor-pointer"
                  style={{
                    background: active ? "var(--accent-weak)" : undefined,
                    color: active ? "var(--accent)" : undefined,
                    fontWeight: active ? 600 : 400,
                  }}
                >
                  {f.label}
                </button>
              );
            })}
          </div>
        </div>

        <div className="flex justify-between items-center border-t dd-border pt-4 gap-4">
          <div className="min-w-0">
            <div className="font-medium">データの場所</div>
            <div className="text-sm dd-sub truncate" title={snap.db_path}>
              {snap.db_path}
            </div>
          </div>
          <button
            onClick={onRevealDb}
            className="text-sm border dd-border rounded-md px-3 py-1 shrink-0 cursor-pointer"
          >
            フォルダを開く
          </button>
        </div>

        <div className="flex justify-between items-center border-t dd-border pt-4">
          <div>
            <div className="font-medium">バックアップ</div>
            <div className="text-sm dd-sub">
              最終：{snap.last_backup ?? "未実行"} ／ 30世代保持
            </div>
          </div>
          <button
            onClick={onBackup}
            className="text-sm border dd-border rounded-md px-3 py-1 cursor-pointer"
          >
            今すぐ実行
          </button>
        </div>

        <div className="border-t dd-border pt-4 space-y-2">
          <div className="flex justify-between items-center gap-4">
            <div>
              <div className="font-medium">アップデート</div>
              <div className="text-sm dd-sub">
                現在のバージョン：v{snap.app_version}
                {updateState === "up_to_date" && "（最新です）"}
                {availableUpdate && ` → v${availableUpdate.version} が利用可能`}
              </div>
            </div>
            <div className="flex gap-2 shrink-0">
              {availableUpdate ? (
                <button
                  onClick={handleInstallUpdate}
                  disabled={updateState === "installing"}
                  className="text-sm font-medium text-white rounded-md px-3 py-1 cursor-pointer disabled:opacity-60"
                  style={{ background: "var(--accent)" }}
                >
                  {updateState === "installing"
                    ? "更新中…"
                    : "更新して再起動"}
                </button>
              ) : (
                <button
                  onClick={handleCheckUpdate}
                  disabled={updateState === "checking"}
                  className="text-sm border dd-border rounded-md px-3 py-1 cursor-pointer disabled:opacity-60"
                >
                  {updateState === "checking"
                    ? "確認中…"
                    : "アップデートを確認"}
                </button>
              )}
            </div>
          </div>
          {availableUpdate?.body && (
            <div className="dd-md text-xs dd-sub border dd-border rounded-md p-3 max-h-40 overflow-y-auto">
              <ReactMarkdown remarkPlugins={[remarkGfm]}>
                {availableUpdate.body}
              </ReactMarkdown>
            </div>
          )}
          {updateError && (
            <div className="text-xs" style={{ color: "var(--danger)" }}>
              {updateError}
            </div>
          )}
        </div>
      </div>

      <div className="dd-surface border dd-border rounded-lg p-5 dd-sub text-sm">
        フェーズ2：定期タスク／会社休日／通知 の設定がここに追加されます
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// 詳細パネル
// ---------------------------------------------------------------------------

function DetailPanel({
  task,
  onClose,
  onSave,
  onComplete,
  onHold,
  onResume,
  onReopenOrRestore,
  onDelete,
}: {
  task: Task;
  onClose: () => void;
  onSave: (
    patch: {
      title?: string;
      memo?: string;
      due_date?: string | null;
      urls?: string[];
    },
    label: string,
  ) => Promise<void>;
  onComplete: () => void;
  onHold: () => void;
  onResume: () => void;
  onReopenOrRestore: () => void;
  onDelete: () => void;
}) {
  const [title, setTitle] = useState(task.title);
  const [memo, setMemo] = useState(task.memo);
  const [editingMemo, setEditingMemo] = useState(false);
  const [newUrl, setNewUrl] = useState("");

  useEffect(() => {
    setTitle(task.title);
    setMemo(task.memo);
  }, [task.title, task.memo]);

  const commitTitle = () => {
    const trimmed = title.trim();
    if (trimmed && trimmed !== task.title) {
      onSave({ title: trimmed }, "タイトル");
    } else {
      setTitle(task.title);
    }
  };

  const commitMemo = () => {
    if (memo !== task.memo) {
      onSave({ memo }, "メモ");
    }
    setEditingMemo(false);
  };

  const addUrl = () => {
    const u = newUrl.trim();
    if (!u) return;
    setNewUrl("");
    if (!task.urls.includes(u)) {
      onSave({ urls: [...task.urls, u] }, "URL");
    }
  };

  const removeUrl = (idx: number) => {
    const next = task.urls.filter((_, i) => i !== idx);
    onSave({ urls: next }, "URL");
  };

  return (
    <aside className="w-[360px] border-l dd-border dd-surface overflow-y-auto p-5 shrink-0 select-text">
      <div className="flex justify-between items-center mb-3">
        <span className="text-xs dd-sub">#{task.id}</span>
        <button
          onClick={onClose}
          className="dd-sub text-lg leading-none cursor-pointer"
        >
          ×
        </button>
      </div>

      <input
        value={title}
        onChange={(e) => setTitle(e.target.value)}
        onBlur={commitTitle}
        onKeyDown={(e) => {
          if (e.key === "Enter") e.currentTarget.blur();
        }}
        className="w-full text-lg font-semibold border dd-border rounded-md px-2 py-1 mb-4 outline-none"
      />

      <div className="flex items-center justify-between">
        <label className="text-sm font-medium dd-sub">期限</label>
        {task.due_date && (
          <button
            onClick={() => onSave({ due_date: null }, "期限")}
            className="text-xs dd-sub underline cursor-pointer"
          >
            クリア
          </button>
        )}
      </div>
      <input
        type="date"
        value={task.due_date ?? ""}
        onChange={(e) =>
          onSave({ due_date: e.target.value || null }, "期限")
        }
        className="w-full border dd-border rounded-md px-2 py-1 mb-4 outline-none"
      />

      <label className="text-sm font-medium dd-sub">URL</label>
      <div className="space-y-1 mb-2 mt-1">
        {task.urls.map((u, i) => (
          <div key={u} className="flex items-center gap-1">
            <button
              onClick={() => api.openExternalUrl(u)}
              className="flex-1 text-left text-sm dd-accent truncate underline underline-offset-2 cursor-pointer"
              title={u}
            >
              {u}
            </button>
            <button
              onClick={() => removeUrl(i)}
              className="dd-sub text-sm px-1 cursor-pointer"
            >
              ×
            </button>
          </div>
        ))}
      </div>
      <input
        value={newUrl}
        onChange={(e) => setNewUrl(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") addUrl();
        }}
        placeholder="URLを貼り付けてEnter"
        className="w-full border dd-border rounded-md px-2 py-1 text-sm mb-4 outline-none"
      />

      <div className="flex justify-between items-center">
        <label className="text-sm font-medium dd-sub">メモ</label>
        <div className="text-xs border dd-border rounded-md overflow-hidden">
          <button
            onClick={() => setEditingMemo(true)}
            className="px-2 py-0.5 cursor-pointer"
            style={{
              background: editingMemo ? "var(--accent-weak)" : undefined,
            }}
          >
            編集
          </button>
          <button
            onClick={commitMemo}
            className="px-2 py-0.5 cursor-pointer"
            style={{
              background: !editingMemo ? "var(--accent-weak)" : undefined,
            }}
          >
            表示
          </button>
        </div>
      </div>

      {editingMemo ? (
        <textarea
          value={memo}
          onChange={(e) => setMemo(e.target.value)}
          onBlur={commitMemo}
          onKeyDown={(e) => {
            if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
              e.preventDefault();
              commitMemo();
            }
          }}
          rows={8}
          className="w-full border dd-border rounded-md px-2 py-1.5 text-sm mt-1 font-mono outline-none"
        />
      ) : (
        <div
          onClick={() => setEditingMemo(true)}
          className="dd-md text-sm border dd-border rounded-md px-3 py-2 mt-1 min-h-[120px] cursor-text"
        >
          {task.memo ? (
            <ReactMarkdown remarkPlugins={[remarkGfm]}>
              {task.memo}
            </ReactMarkdown>
          ) : (
            <span className="dd-sub">クリックしてメモを入力…</span>
          )}
        </div>
      )}

      <div className="text-xs dd-sub mt-4 space-y-0.5">
        <div>作成：{formatShortDateTime(task.created_at)}</div>
        {task.completed_at && (
          <div>完了：{formatShortDateTime(task.completed_at)}</div>
        )}
      </div>

      <div className="flex gap-2 mt-4 border-t dd-border pt-4">
        {task.status === "queued" && (
          <>
            <button
              onClick={onComplete}
              className="px-3 py-1.5 rounded-md text-sm font-medium text-white cursor-pointer"
              style={{ background: "var(--accent)" }}
            >
              完了
            </button>
            <button
              onClick={onHold}
              className="px-3 py-1.5 rounded-md text-sm border dd-border cursor-pointer"
            >
              保留
            </button>
          </>
        )}
        {task.status === "on_hold" && (
          <button
            onClick={onResume}
            className="px-3 py-1.5 rounded-md text-sm font-medium text-white cursor-pointer"
            style={{ background: "var(--accent)" }}
          >
            キューに戻す
          </button>
        )}
        {(task.status === "done" || task.status === "deleted") && (
          <button
            onClick={onReopenOrRestore}
            className="px-3 py-1.5 rounded-md text-sm font-medium text-white cursor-pointer"
            style={{ background: "var(--accent)" }}
          >
            {task.status === "done" ? "未完了に戻す" : "復元"}
          </button>
        )}
        {task.status !== "deleted" && (
          <button
            onClick={onDelete}
            className="ml-auto px-3 py-1.5 rounded-md text-sm cursor-pointer"
            style={{ color: "var(--danger)" }}
          >
            削除
          </button>
        )}
      </div>
    </aside>
  );
}
