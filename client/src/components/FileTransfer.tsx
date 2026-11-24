// 文件传输窗口：左边本机、右边被控端，两个目录面板 + 底部传输列表。
//
// 位置栏（路径那一行）点一下是下拉菜单：此电脑、各个盘、桌面 / 下载这些系统文件夹、
// 最近去过的目录——和资源管理器一样，不用一层层往上退。
//
// 只做**单个文件**（不像文件夹那样递归）：可以多选，一个接一个传。
// 没有断点续传、不限速；同名冲突按图里的三选一：跳过 / 替换 / 保留两者（`名字 (1).扩展名`）。
//
// 本机这一半走 Tauri 命令（lib/api.ts 的 local*），被控端那一半走这个窗口自己的会话（lib/files.ts）。
import { useCallback, useEffect, useRef, useState } from "react";
import { api, toProblem, type FsEntry, type FsPlace } from "../lib/api";
import { FileSession, MAX_CHUNK, type FileStage, type FsListing } from "../lib/files";
import type { FilesTarget } from "../lib/files-window";
import { setLang, t, useT } from "../lib/i18n";
import { problemText } from "../lib/messages";

/** 一个目录面板的内容（`path` 为空 = "此电脑"，列的是各个盘和系统文件夹） */
interface Dir {
  path: string;
  parent: string | null;
  entries: FsEntry[];
}

const EMPTY_DIR: Dir = { path: "", parent: null, entries: [] };

/** 冲突时用户选的（见图里的三个按钮） */
type Conflict = "skip" | "replace" | "keepBoth";

/** 传输列表里的一行 */
interface Job {
  id: number;
  name: string;
  /** up = 本机 → 对方，down = 对方 → 本机 */
  kind: "up" | "down";
  from: string;
  to: string;
  size: number;
  done: number;
  status: JobStatus;
  error?: string;
}

type JobStatus = "running" | "paused" | "done" | "failed" | "skipped" | "cancelled";

/** 一个目录最多画多少行（再看就进去找，不然一次渲染几千行会卡） */
const MAX_ROWS = 2000;

/** 每边记几个最近去过的目录 */
const MAX_RECENT = 6;

/** 用户点了取消：从传输循环里扔出来 */
const CANCELLED = new Error("cancelled");

/** 等用户点冲突对话框 */
interface Asking {
  name: string;
  resolve: (c: Conflict) => void;
}

/** 路径拼接（Windows 用反斜杠，其它用正斜杠） */
function join(dir: string, name: string): string {
  if (!dir) return name;
  const sep = dir.includes("\\") ? "\\" : "/";
  return dir.endsWith(sep) ? dir + name : dir + sep + name;
}

/** base64 里有多少字节（进度用；`=` 是补位，不算） */
function bytesOf(b64: string): number {
  const pad = b64.endsWith("==") ? 2 : b64.endsWith("=") ? 1 : 0;
  return (b64.length / 4) * 3 - pad;
}

/** 文件名里的扩展名（列表的"类型"那一列） */
function kindOf(name: string, dir: boolean): string {
  if (dir) return t("files.folder");
  const i = name.lastIndexOf(".");
  return i > 0 ? name.slice(i + 1) : "";
}

/** 文件大小：KB / MB / GB */
function sizeOf(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(2)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

/** 修改时间：`2026/09/16 17:18` */
function timeOf(unix: number): string {
  if (!unix) return "";
  const d = new Date(unix * 1000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}/${p(d.getMonth() + 1)}/${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** 系统文件夹的文案键（`kind` 是 Rust 给的那几种） */
const PLACE_TEXT: Record<string, "files.thisPc" | "files.desktop" | "files.documents" | "files.downloads" | "files.pictures" | "files.videos" | "files.music"> = {
  this: "files.thisPc",
  desktop: "files.desktop",
  documents: "files.documents",
  downloads: "files.downloads",
  pictures: "files.pictures",
  videos: "files.videos",
  music: "files.music",
};

/** 位置的名字：盘显示卷标 + 盘符（`Data (D:)`），系统文件夹交给文案 */
function placeLabel(p: FsPlace): string {
  if (p.kind === "drive") {
    const letter = p.path.length >= 2 ? p.path.slice(0, 2) : p.path;
    return p.name ? `${p.name} (${letter})` : letter;
  }
  return t(PLACE_TEXT[p.kind] ?? "files.thisPc");
}

/** 位置的图标（和列表里一样用文字符号，不引图标库） */
function placeIcon(p: FsPlace): string {
  if (p.kind === "drive") return "▤";
  if (p.kind === "this") return "▣";
  return "▸";
}

export function FileTransfer({ target }: { target: FilesTarget }) {
  useT();
  const [session, setSession] = useState<FileSession | null>(null);
  const sessionRef = useRef<FileSession | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  /** 正在做什么（"正在连接服务器…"），不是错误 */
  const [stage, setStage] = useState<FileStage>("server");
  /** 出错：本地错误是 Error，服务端给的是 Problem 对象，都转成同一句话显示 */
  const fail = (e: unknown) => setProblem(problemText(toProblem(e)));

  const [hostName, setHostName] = useState("");
  const [local, setLocal] = useState<Dir>(EMPTY_DIR);
  const [remote, setRemote] = useState<Dir>(EMPTY_DIR);
  const [localPlaces, setLocalPlaces] = useState<FsPlace[]>([]);
  const [remotePlaces, setRemotePlaces] = useState<FsPlace[]>([]);
  const [localRecent, setLocalRecent] = useState<string[]>([]);
  const [remoteRecent, setRemoteRecent] = useState<string[]>([]);
  const [picked, setPicked] = useState<{ side: "local" | "remote"; names: string[] } | null>(null);
  const [jobs, setJobs] = useState<Job[]>([]);
  const asking = useRef<Asking | null>(null);
  const [dialog, setDialog] = useState<Asking | null>(null);
  const nextId = useRef(1);
  /** 被暂停 / 被取消的任务（传输循环在每个块之前看一眼） */
  const paused = useRef(new Set<number>());
  const cancelled = useRef(new Set<number>());
  /** 传输排队：一次只传一个文件（块级请求-应答，多路并发没有收益） */
  const chain = useRef<Promise<void>>(Promise.resolve());

  // ── 连接 ──
  // 这个窗口是浅色的（见 styles.css 的 .ft）：页面声明的是深色 color-scheme，改回来滚动条才是浅的
  useEffect(() => {
    document.documentElement.style.colorScheme = "light";
  }, []);

  // 语言切换（标题栏、按钮文字跟着变）
  useEffect(() => api.onLangChanged(setLang), []);

  useEffect(() => {
    void api.appInfo().then((i) => setHostName(i.hostname), () => {});
  }, []);

  useEffect(() => {
    const s = new FileSession(target.code, {
      onStage: setStage,
      onProblem: (p) => setProblem(problemText(p)),
    });
    sessionRef.current = s;
    setSession(s);
    void (async () => {
      try {
        await s.open();
        if (!s.supported) {
          setProblem(t("files.unsupported"));
          return;
        }
        const roots = await s.roots();
        setRemotePlaces([
          { name: "", path: "", kind: "this" },
          ...roots.places.filter((p) => p.kind === "drive"),
          ...roots.places.filter((p) => p.kind !== "drive"),
        ]);
        await openRemote(roots.home);
      } catch (e) {
        fail(e);
      }
    })();
    return () => {
      sessionRef.current = null;
      s.dispose();
    };
  }, [target.code]);

  // ── 目录 ──
  const toDir = (r: FsListing | { path: string; parent: string | null; entries: FsEntry[] }): Dir => ({
    path: r.path,
    parent: r.parent,
    entries: r.entries,
  });

  const remember = (list: string[], path: string) =>
    path ? [path, ...list.filter((p) => p !== path)].slice(0, MAX_RECENT) : list;

  const openLocal = useCallback(async (path: string) => {
    try {
      // "此电脑"：不列目录，直接画各个盘和系统文件夹
      if (!path) {
        setLocal({ path: "", parent: null, entries: [] });
        return;
      }
      setLocal(toDir(await api.localList(path)));
      setLocalRecent((r) => remember(r, path));
    } catch (e) {
      fail(e);
    }
  }, []);

  const openRemote = async (path: string) => {
    const s = sessionRef.current;
    if (!s) return;
    if (!path) {
      setRemote({ path: "", parent: null, entries: [] });
      return;
    }
    const r = await s.list(path);
    if (r.error) {
      setProblem(r.error);
      return;
    }
    setRemote(toDir(r));
    setRemoteRecent((prev) => remember(prev, r.path));
  };

  // ── 本机的默认目录 ──
  useEffect(() => {
    void (async () => {
      try {
        const { places, home } = await api.localRoots();
        setLocalPlaces([{ name: "", path: "", kind: "this" }, ...places]);
        if (home) await openLocal(home);
      } catch {
        // 网页版没有本机文件：左边那一栏会是空的
      }
    })();
  }, [openLocal]);

  // ── 传输 ──
  const patch = (id: number, p: Partial<Job>) =>
    setJobs((list) => list.map((j) => (j.id === id ? { ...j, ...p } : j)));

  const askConflict = (name: string): Promise<Conflict> =>
    new Promise((resolve) => {
      const a: Asking = { name, resolve };
      asking.current = a;
      setDialog(a);
    });

  const answer = (c: Conflict) => {
    setDialog(null);
    const a = asking.current;
    asking.current = null;
    a?.resolve(c);
  };

  /** 目标已经存在时问用户要怎么办；返回最终要写的路径（null = 不传了） */
  const settle = async (dirPath: string, name: string, exists: (p: string) => Promise<boolean>) => {
    let path = join(dirPath, name);
    if (!(await exists(path))) return path;
    const choice = await askConflict(name);
    if (choice === "skip") return null;
    if (choice === "replace") return path;
    // 保留两者：name (1).ext、name (2).ext……
    const dot = name.lastIndexOf(".");
    const base = dot > 0 ? name.slice(0, dot) : name;
    const ext = dot > 0 ? name.slice(dot) : "";
    for (let n = 1; n < 1000; n++) {
      const candidate = join(dirPath, `${base} (${n})${ext}`);
      if (!(await exists(candidate))) return candidate;
    }
    return path;
  };

  /** 暂停 / 取消就在每个块之前等着：不用改动块本身的收发（也不需要对方配合） */
  const gate = async (id: number) => {
    for (;;) {
      if (cancelled.current.has(id)) throw CANCELLED;
      if (!paused.current.has(id)) return;
      await new Promise((r) => setTimeout(r, 150));
    }
  };

  const transfer = async (side: "local" | "remote", name: string, fromDir: string, toDir: string) => {
    const s = sessionRef.current;
    if (!s) return;
    const up = side === "local";
    const from = join(fromDir, name);
    const id = nextId.current++;
    setJobs((list) => [
      ...list,
      { id, name, kind: up ? "up" : "down", from, to: "", size: 0, done: 0, status: "running" },
    ]);

    // 本机这一半和会话这一半各用一个 id（不撞）
    const handle = 1000 + id;
    try {
      const target = await settle(toDir, name, (p) => (up ? s.exists(p) : api.localExists(p)));
      if (!target) {
        patch(id, { status: "skipped" });
        return;
      }
      patch(id, { to: target });
      let size: number;
      if (up) {
        size = await api.localOpen(handle, from, false);
        patch(id, { size });
        await s.openFile(id, target, true);
      } else {
        size = await s.openFile(id, from, false);
        patch(id, { size });
        await api.localOpen(handle, target, true);
      }
      let offset = 0;
      while (offset < size) {
        await gate(id);
        const chunk =
          side === "local"
            ? await api.localRead(handle, offset, MAX_CHUNK)
            : await s.read(id, offset, MAX_CHUNK);
        if (!chunk.data) break;
        if (up) await s.write(id, offset, chunk.data);
        else await api.localWrite(handle, offset, chunk.data);
        offset += bytesOf(chunk.data);
        patch(id, { done: Math.min(offset, size) });
        if (chunk.eof) break;
      }
      patch(id, { status: "done", done: size });
    } catch (e) {
      if (e === CANCELLED) patch(id, { status: "cancelled" });
      else patch(id, { status: "failed", error: problemText(toProblem(e)) });
    } finally {
      s.close(id);
      void api.localClose(handle).catch(() => {});
      paused.current.delete(id);
      cancelled.current.delete(id);
    }
  };

  /** 选中文件发送：一个接一个传（传完刷新目标那一栏） */
  const send = (side: "local" | "remote") => {
    if (!picked || picked.side !== side || picked.names.length === 0) return;
    const names = [...picked.names];
    const fromDir = side === "local" ? local.path : remote.path;
    const toDir = side === "local" ? remote.path : local.path;
    setPicked({ side, names: [] });
    chain.current = chain.current.then(async () => {
      for (const name of names) await transfer(side, name, fromDir, toDir);
      if (side === "local") await openRemote(toDir);
      else await openLocal(toDir);
    });
  };

  const toggle = (side: "local" | "remote", name: string) =>
    setPicked((prev) => {
      const names = prev?.side === side ? prev.names : [];
      const next = names.includes(name) ? names.filter((n) => n !== name) : [...names, name];
      return { side, names: next };
    });

  const selectAll = (side: "local" | "remote", dir: Dir) => {
    const files = dir.entries.filter((e) => !e.dir).map((e) => e.name);
    setPicked((prev) =>
      prev?.side === side && prev.names.length === files.length ? { side, names: [] } : { side, names: files },
    );
  };

  const cancel = (id: number) => {
    cancelled.current.add(id);
    patch(id, { status: "cancelled" });
  };

  const runCancelAll = () => {
    for (const j of jobs) {
      if (j.status === "running" || j.status === "paused") cancel(j.id);
    }
  };

  const setPause = (ids: number[], on: boolean) => {
    for (const id of ids) {
      if (on) paused.current.add(id);
      else paused.current.delete(id);
    }
    setJobs((list) =>
      list.map((j) =>
        ids.includes(j.id) && (j.status === "running" || j.status === "paused")
          ? { ...j, status: on ? "paused" : "running" }
          : j,
      ),
    );
  };

  const sideNames = (side: "local" | "remote") => (picked?.side === side ? picked.names : []);
  const canSend = (side: "local" | "remote") => sideNames(side).length > 0;
  const sentCount = jobs.filter((j) => j.status === "done").length;
  const active = jobs.filter((j) => j.status === "running" || j.status === "paused");
  const finished = jobs.filter((j) => j.status !== "running" && j.status !== "paused");

  return (
    <div className="app ft">
      <div className="ft-body">
        {problem ? (
          <div className="ft-problem" role="alert">
            {problem}
          </div>
        ) : (
          stage !== "ready" && (
            <div className="ft-note" role="status">
              {t(stage === "server" ? "files.waitServer" : "files.waitHost")}
            </div>
          )
        )}

        <div className="ft-panes">
          <Pane
            title={hostName || t("files.local")}
            badge={t("files.local")}
            dir={local}
            places={localPlaces}
            recent={localRecent}
            picked={sideNames("local")}
            onToggle={(name) => toggle("local", name)}
            onSelectAll={() => selectAll("local", local)}
            onOpen={(path) => void openLocal(path)}
            side="left"
            canSend={canSend("local")}
            onSend={() => send("local")}
          />

          <Pane
            title={session?.name || target.name}
            badge={t("files.remote")}
            dir={remote}
            places={remotePlaces}
            recent={remoteRecent}
            picked={sideNames("remote")}
            onToggle={(name) => toggle("remote", name)}
            onSelectAll={() => selectAll("remote", remote)}
            onOpen={(path) => void openRemote(path)}
            side="right"
            canSend={canSend("remote")}
            onSend={() => send("remote")}
          />
        </div>

        <section className="ft-jobs">
          <header className="ft-jobs-head">
            <span className="ft-jobs-title">{t("files.jobs")}</span>
            <span className="muted small">{t("files.jobsCount", { n: sentCount })}</span>
            <span className="spacer" />
            <button
              className="ft-link"
              disabled={!active.some((j) => j.status === "running")}
              onClick={() => setPause(active.filter((j) => j.status === "running").map((j) => j.id), true)}
            >
              {t("files.pauseAll")}
            </button>
            <button
              className="ft-link"
              disabled={!active.some((j) => j.status === "paused")}
              onClick={() => setPause(active.filter((j) => j.status === "paused").map((j) => j.id), false)}
            >
              {t("files.startAll")}
            </button>
            <button className="ft-link" disabled={active.length === 0} onClick={runCancelAll}>
              {t("files.cancelAll")}
            </button>
            <button
              className="ft-link"
              disabled={finished.length === 0}
              onClick={() => setJobs((list) => list.filter((j) => j.status === "running" || j.status === "paused"))}
            >
              {t("files.clearDone")}
            </button>
          </header>
          <div className="ft-jobs-cols">
            <span>{t("files.name")}</span>
            <span>{t("files.status")}</span>
            <span>{t("files.size")}</span>
            <span>{t("files.from")}</span>
            <span>{t("files.to")}</span>
            <span />
          </div>
          <div className="ft-jobs-list">
            {jobs.map((j) => (
              <div className="ft-job" key={j.id}>
                <span className="ft-job-name" title={j.name}>
                  {j.name}
                </span>
                <span className={`ft-job-state ${j.status}`}>
                  {j.status === "running" || j.status === "paused"
                    ? `${j.status === "paused" ? t("files.state.paused") + " · " : ""}${Math.round(
                        (j.done / Math.max(1, j.size)) * 100,
                      )}%`
                    : t(`files.state.${j.status}` as "files.state.done")}
                  {j.error ? <em title={j.error}> · {j.error}</em> : null}
                </span>
                <span className="muted">{j.size ? sizeOf(j.size) : ""}</span>
                <span className="muted ft-path" title={j.from}>
                  {j.from}
                </span>
                <span className="muted ft-path" title={j.to}>
                  {j.to}
                </span>
                <button
                  className="ft-op"
                  title={j.status === "running" || j.status === "paused" ? t("files.cancel") : t("files.remove")}
                  onClick={() =>
                    j.status === "running" || j.status === "paused"
                      ? cancel(j.id)
                      : setJobs((list) => list.filter((x) => x.id !== j.id))
                  }
                >
                  ✕
                </button>
              </div>
            ))}
          </div>
        </section>
      </div>

      {dialog && (
        <div className="confirm-mask" role="presentation">
          <div className="confirm-card" role="dialog" aria-modal="true" aria-label={t("files.conflictTitle")}>
            <div className="confirm-title">{t("files.conflictTitle")}</div>
            <p className="confirm-text">{t("files.conflictAsk", { name: dialog.name })}</p>
            <div className="confirm-actions">
              <button className="confirm-btn" onClick={() => answer("skip")}>
                {t("files.skip")}
              </button>
              <button className="confirm-btn" onClick={() => answer("keepBoth")}>
                {t("files.keepBoth")}
              </button>
              <button className="confirm-btn danger" autoFocus onClick={() => answer("replace")}>
                {t("files.replace")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

/** 一个目录面板 */
function Pane({
  title,
  badge,
  dir,
  places,
  recent,
  picked,
  onToggle,
  onSelectAll,
  onOpen,
  side,
  canSend,
  onSend,
}: {
  title: string;
  badge: string;
  dir: Dir;
  places: FsPlace[];
  recent: string[];
  picked: string[];
  onToggle: (name: string) => void;
  onSelectAll: () => void;
  onOpen: (path: string) => void;
  side: "left" | "right";
  canSend: boolean;
  onSend: () => void;
}) {
  /** 路径栏的下拉菜单开着没有 */
  const [open, setOpen] = useState(false);
  const wrap = useRef<HTMLDivElement | null>(null);

  // 点别处就收起
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!wrap.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  const go = (path: string) => {
    setOpen(false);
    onOpen(path);
  };

  const files = dir.entries.filter((e) => !e.dir);
  const allPicked = files.length > 0 && files.every((e) => picked.includes(e.name));

  return (
    <section className="ft-pane">
      <header className="ft-pane-head">
        {side === "left" ? (
          <>
            <strong className="ft-pane-name">{title}</strong>
            <span className="ft-badge">{badge}</span>
            <span className="spacer" />
            <button className="ft-send" disabled={!canSend} onClick={onSend}>
              {t("files.send")} <span aria-hidden>→</span>
            </button>
          </>
        ) : (
          <>
            <button className="ft-send" disabled={!canSend} onClick={onSend}>
              <span aria-hidden>←</span> {t("files.send")}
            </button>
            <span className="spacer" />
            <span className="ft-badge">{badge}</span>
            <strong className="ft-pane-name">{title}</strong>
          </>
        )}
      </header>

      <div className="ft-tools">
        <button
          className="ft-tool"
          disabled={!dir.parent && !dir.path}
          title={t("files.up")}
          onClick={() => onOpen(dir.path ? (dir.parent ?? "") : "")}
        >
          ←
        </button>
        <button className="ft-tool" title={t("files.thisPc")} onClick={() => go("")}>
          ▣
        </button>
        <button className="ft-tool" title={t("files.refresh")} onClick={() => onOpen(dir.path)}>
          ⟳
        </button>
        <div className="ft-path-wrap" ref={wrap}>
          <button
            className="ft-path-bar"
            title={dir.path || t("files.thisPc")}
            onClick={() => setOpen((v) => !v)}
            aria-expanded={open}
          >
            <span className="ft-path-text">{dir.path || t("files.thisPc")}</span>
            <span className="ft-caret" aria-hidden>
              ⌄
            </span>
          </button>
          {open && <PlaceList places={places} recent={recent} onGo={go} />}
        </div>
      </div>

      <div className="ft-cols">
        <span
          className={`ft-check ${allPicked ? "on" : ""}`}
          role="checkbox"
          aria-checked={allPicked}
          aria-label={t("files.select")}
          onClick={onSelectAll}
        />
        <span>{t("files.name")}</span>
        <span>{t("files.modified")}</span>
        <span>{t("files.type")}</span>
        <span>{t("files.size")}</span>
      </div>
      <div className="ft-list">
        {/* 目录很大时只画前这么多条：一次渲染几千行会让窗口卡住 */}
        {dir.entries.slice(0, MAX_ROWS).map((e) => {
          const on = picked.includes(e.name);
          return (
            <div
              key={e.name}
              className={`ft-row ${on ? "picked" : ""}`}
              onClick={() => (e.dir ? onOpen(join(dir.path, e.name)) : onToggle(e.name))}
              title={e.name}
            >
              <span className={`ft-check ${on ? "on" : ""}`} aria-hidden>
                {on ? "✓" : ""}
              </span>
              <span className="ft-cell name">
                <span className={`ft-icon ${e.dir ? "dir" : ""}`} aria-hidden>
                  {e.dir ? "▸" : "≡"}
                </span>
                {e.name}
              </span>
              <span className="ft-cell muted">{timeOf(e.modified)}</span>
              <span className="ft-cell muted">{kindOf(e.name, e.dir)}</span>
              <span className="ft-cell muted">{e.dir ? "" : sizeOf(e.size)}</span>
            </div>
          );
        })}

        {/* "此电脑"：列各个盘和系统文件夹 */}
        {dir.path === "" &&
          places.map((p) => (
            <div key={p.path || p.kind} className="ft-row" onClick={() => onOpen(p.path)} title={p.path}>
              <span className="ft-check" aria-hidden />
              <span className="ft-cell name">
                <span className="ft-icon dir" aria-hidden>
                  {placeIcon(p)}
                </span>
                {placeLabel(p)}
              </span>
              <span className="ft-cell muted" />
              <span className="ft-cell muted">{t("files.folder")}</span>
              <span className="ft-cell muted" />
            </div>
          ))}

        {dir.path !== "" && dir.entries.length === 0 && (
          <p className="ft-empty muted small">{t("files.empty")}</p>
        )}
        {dir.entries.length > MAX_ROWS && (
          <p className="ft-empty muted small">{t("files.tooMany", { n: dir.entries.length - MAX_ROWS })}</p>
        )}
      </div>
    </section>
  );
}

/** 路径栏的下拉：搜索框 + 此电脑 / 各个盘 / 系统文件夹 / 最近去过 */
function PlaceList({ places, recent, onGo }: { places: FsPlace[]; recent: string[]; onGo: (path: string) => void }) {
  const [q, setQ] = useState("");
  const text = q.trim().toLowerCase();
  const match = (label: string, path: string) =>
    !text || label.toLowerCase().includes(text) || path.toLowerCase().includes(text);
  const hit = places.filter((p) => match(placeLabel(p), p.path));
  const recents = recent.filter((p) => match(p, p));

  return (
    <div className="ft-places">
      <input
        className="ft-place-search"
        autoFocus
        value={q}
        placeholder={t("files.goPath")}
        onChange={(e) => setQ(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") setQ("");
          if (e.key === "Enter" && q.trim()) onGo(q.trim());
        }}
      />
      <div className="ft-place-list">
        {hit.length === 0 && recents.length === 0 && <p className="ft-empty muted small">{t("files.empty")}</p>}
        {hit.map((p, i) => (
          <div key={p.path || "this"} className="ft-place">
            <button className="ft-place-row" onClick={() => onGo(p.path)} title={p.path}>
              <span className="ft-icon dir" aria-hidden>
                {placeIcon(p)}
              </span>
              <span className="ft-place-name">{placeLabel(p)}</span>
            </button>
            {/* 盘和系统文件夹之间画一条线 */}
            {i + 1 < hit.length && (hit[i].kind === "drive") !== (hit[i + 1].kind === "drive") && (
              <div className="ft-place-sep" />
            )}
          </div>
        ))}
        {recents.length > 0 && <div className="ft-place-cap">{t("files.recent")}</div>}
        {recents.map((p) => (
          <button key={p} className="ft-place-row" onClick={() => onGo(p)} title={p}>
            <span className="ft-icon" aria-hidden>
              ◷
            </span>
            <span className="ft-place-name">{p}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

/** 远程窗口里打开文件传输（见 components/ControlCenter.tsx） */
export function openFileTransfer(code: string, name: string): Promise<void> {
  return api.openFilesWindow({ code, name });
}
