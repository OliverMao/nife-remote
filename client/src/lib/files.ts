// 文件传输：走**账号服务的 WebSocket** 转发（见 src-tauri/src/peer.rs 和 server/auth/app/presence.py），
// 不再经过远程会话——所以不需要有人正在看对方画面，也不需要配对码，只要对方在线、而且在同一个账号里。
//
// 一问一答：每次请求带一个 `req` 号，对面的回答原样带回来（`fs-peer` 事件），这里据此 resolve。
// 一次最多 MAX_CHUNK 字节，界面据此算进度，也不需要额外的流控。
import { api, type FsEntry, type FsPlace, type Problem } from "./api";

/** 一次读 / 写最多这么多字节（Rust 那边也按这个上限截断） */
export const MAX_CHUNK = 512 * 1024;

/** 能直接跳过去的位置（盘符、桌面、下载……）和默认目录 */
export interface FsRoots {
  places: FsPlace[];
  home: string;
}

/** 一个目录的内容（`error` 有值就是打不开，界面显示它） */
export interface FsListing {
  path: string;
  parent: string | null;
  entries: FsEntry[];
  error: string | null;
}

/** 一块数据等多久算丢 */
const REPLY_TIMEOUT_MS = 30_000;

/** 连接进度：连账号服务里的那台设备、连上了 */
export type FileStage = "connect" | "ready";

/** 一次请求的回答 */
type Pending = { resolve: (v: unknown) => void; reject: (e: unknown) => void; timer: number };

/** 对方给的一句话（英文技术信息）：界面显示成"操作失败：…" */
function failed(detail: string): Problem {
  return { kind: "failed", detail };
}

export class FileSession {
  readonly code: string;
  /** 对方的名字（窗口标题、界面上显示；由外面传进来，这里不再从会话里问） */
  name = "";
  /** 隧道号（`null` = 还没连上） */
  private tid: number | null = null;
  private nextReq = 1;
  private pending = new Map<number, Pending>();
  private disposed = false;
  private readonly stopEvent: () => void;

  stage: FileStage = "connect";

  constructor(
    code: string,
    name: string,
    private readonly opts: {
      onStage?: (stage: FileStage) => void;
      /** 出错 / 对面断开了：（界面显示原因，连上后传 null 撤下） */
      onProblem?: (problem: Problem | null) => void;
    } = {},
  ) {
    this.code = code;
    this.name = name;
    this.stopEvent = api.onFsPeer((m) => this.onMessage(m));
  }

  private setStage(stage: FileStage) {
    this.stage = stage;
    this.opts.onStage?.(stage);
  }

  /** 连上对方（开一条隧道）。对方不在线 / 不在这个账号里就抛出原因 */
  async open(): Promise<void> {
    this.setStage("connect");
    this.tid = await api.peerFsOpen(this.code);
    this.opts.onProblem?.(null);
    this.setStage("ready");
  }

  private onMessage(m: { tid: number; data?: { req?: number; ok?: unknown; err?: string }; gone?: string }) {
    if (m.tid !== this.tid) return;
    if (m.gone) {
      // 隧道断了（对方关了窗口、掉线）：在等的请求全部失败，界面把原因显示出来
      const problem =
        m.gone === "peerGone" ? failed("对方断开了连接") : failed("文件传输已结束");
      this.failAll(problem);
      this.opts.onProblem?.(problem);
      return;
    }
    const req = m.data?.req;
    if (typeof req !== "number") return;
    const p = this.pending.get(req);
    if (!p) return;
    this.pending.delete(req);
    window.clearTimeout(p.timer);
    if (m.data?.err !== undefined) p.reject(failed(m.data.err));
    else p.resolve(m.data?.ok);
  }

  private failAll(problem: Problem) {
    for (const p of this.pending.values()) {
      window.clearTimeout(p.timer);
      p.reject(problem);
    }
    this.pending.clear();
  }

  /** 发一条请求，等对面的回答 */
  private ask<T>(data: Record<string, unknown>): Promise<T> {
    if (this.tid === null) return Promise.reject(failed("还没连上对方"));
    const req = this.nextReq++;
    const p = new Promise<T>((resolve, reject) => {
      const timer = window.setTimeout(() => {
        if (!this.pending.delete(req)) return;
        reject(failed("对方没有回应"));
      }, REPLY_TIMEOUT_MS);
      this.pending.set(req, { resolve: resolve as (v: unknown) => void, reject, timer });
      api.peerFsSend(this.tid as number, { req, ...data }).catch((e: unknown) => {
        if (!this.pending.delete(req)) return;
        window.clearTimeout(timer);
        reject(e);
      });
    });
    // 隧道断开时这里会被拒绝；调用方可能已经不等了，别冒成"未处理的拒绝"
    p.catch(() => {});
    return p;
  }

  /** 对方能直接跳过去的位置和默认目录 */
  roots(): Promise<FsRoots> {
    return this.ask<FsRoots>({ op: "roots" });
  }

  /** 列一个目录（`path` 为空 = 默认目录） */
  list(path: string): Promise<FsListing> {
    return this.ask<FsListing>({ op: "list", path });
  }

  /** 这个路径存不存在（发文件之前问：决定要不要弹"是否替换"） */
  exists(path: string): Promise<boolean> {
    return this.ask<boolean>({ op: "exists", path });
  }

  /** 打开一个文件：`write` 为真时清空重写；返回要读的文件大小 */
  openFile(id: number, path: string, write: boolean): Promise<number> {
    return this.ask<number>({ op: "open", id, path, write });
  }

  /** 读一块（`data` 是 base64，和本地写入的命令一样，省一次转换） */
  read(id: number, offset: number, len = MAX_CHUNK): Promise<{ data: string; eof: boolean }> {
    return this.ask<{ data: string; eof: boolean }>({ op: "read", id, offset, len });
  }

  /** 写一块（`data` 是 base64） */
  write(id: number, offset: number, data: string): Promise<void> {
    return this.ask<void>({ op: "write", id, offset, data });
  }

  close(id: number) {
    void this.ask<void>({ op: "close", id }).catch(() => {});
  }

  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.stopEvent();
    this.failAll(failed("文件传输已结束"));
    if (this.tid !== null) api.peerFsClose(this.tid);
    this.tid = null;
  }
}
