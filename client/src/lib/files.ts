// 文件传输窗口用的会话：和远程窗口一样是"到同一个控制码的另一路连接"（画面是一路流，
// 会话之间不共享），只是这路不看画面——把它接在一个离屏 canvas 上，画质压到最低。
//
// 读 / 写都是"问一块、回一块"：一次最多 MAX_CHUNK 字节，界面据此算进度、也不需要流控。
// 协议见 nife_remote_proto::session 的 Fs* 消息，命令/事件见 api.ts 的 Command / SessionEvent。
import {
  api,
  type Codec,
  type FsEntry,
  type FsPlace,
  type Problem,
  type QualitySettings,
  type SessionEvent,
} from "./api";
import { supportedDecoders } from "./codec";

/** 一次读 / 写最多这么多字节（和 Rust 的 MAX_FS_CHUNK 一致） */
export const MAX_CHUNK = 512 * 1024;

/** 传输用的画质：文件窗口不看画面，只要一路最省的流（有些被控端不许 fps=0） */
const SLIM: QualitySettings = { fps: 15, bitrateKbps: 500, maxShort: 360, adaptive: true, codec: "h264" };

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

/** 一块数据等多久算丢（和 console.ts 的命令超时一致） */
const REPLY_TIMEOUT_MS = 30_000;

/** 连接进度：等中转服务器、连对方电脑、连上了 */
export type FileStage = "server" | "connect" | "ready";

/** 一次请求的回答（`req` 用来对回请求） */
type Pending<T> = { resolve: (v: T) => void; reject: (e: Error) => void };

export class FileSession {
  readonly code: string;
  /** 被控端的名字（窗口标题、界面上显示） */
  name = "";
  sid: number | null = null;
  /** 连上了（可以发文件相关的命令） */
  live = false;
  /** 连不上的原因（界面显示） */
  problem: Problem | null = null;
  /** 被控端支持文件传输（对方的版本旧时没有这一位） */
  supported = false;

  private readonly canvas: HTMLCanvasElement;
  private nextReq = 1;
  private reqs = new Map<number, Pending<unknown>>();
  private files = new Map<number, Pending<number>>();
  private reads = new Map<number, Pending<{ data: string; eof: boolean }>>();
  private writes = new Map<number, Pending<void>>();
  private disposed = false;

  /** 现在卡在哪一步（界面据此显示"正在连接服务器…"这类提示） */
  stage: FileStage = "server";

  constructor(
    code: string,
    private readonly opts: {
      pairing?: string | null;
      onStage?: (stage: FileStage) => void;
      /** 会话断了（对方关了、版本不对……）：界面把原因显示出来 */
      onProblem?: (problem: Problem) => void;
    } = {},
  ) {
    this.code = code;
    this.canvas = document.createElement("canvas");
    this.canvas.width = 16;
    this.canvas.height = 16;
  }

  private setStage(stage: FileStage) {
    this.stage = stage;
    this.opts.onStage?.(stage);
  }

  /**
   * 连上（失败抛出问题，界面显示它）。
   *
   * 先等本机和中转服务器的连接建好：程序刚启动、网络刚恢复时它还在连，
   * 这时候直接发 `connect` 只会得到"尚未连接到服务器"——文件窗口等一会儿就好。
   */
  async open(): Promise<void> {
    this.setStage("server");
    if (!(await this.waitServer(60_000))) throw { kind: "notConnected" };
    // 不看画面，但控制端服务要求每个会话有一个画布（Worker 把画面画在这里，谁也看不见）
    const view = api.view(this.canvas, {
      size: () => {},
      cursor: () => {},
      undecodable: () => {},
      recovered: () => {},
      stats: () => {},
    });
    this.setStage("connect");
    const codecs = await supportedDecoders().catch(() => ["h264"] as Codec[]);
    const info = await api.connect(
      this.code,
      {
        settings: { ...SLIM, codec: codecs[0] ?? "h264" },
        audio: false,
        codecs,
        pairing: this.opts.pairing ?? null,
        // 文件窗口不看画面，也没有上次的重连凭证：每次都是一个新会话
        resume: null,
        hostKey: null,
      },
      view,
      (ev) => this.onEvent(ev),
      false,
    );
    this.sid = info.sid;
    this.name = info.hostName;
    this.live = true;
    this.supported = info.features.includes("files");
    this.setStage("ready");
  }

  /** 等到和中转服务器的连接建好（已经在连的时候最多等 `ms`） */
  private async waitServer(ms: number): Promise<boolean> {
    const now = await api.serverStatus().catch(() => null);
    if (now?.state === "connected") return true;
    return new Promise<boolean>((resolve) => {
      let off = () => {};
      const timer = window.setTimeout(() => {
        off();
        resolve(false);
      }, ms);
      off = api.onServerStatus((s) => {
        if (s.state !== "connected") return;
        window.clearTimeout(timer);
        off();
        resolve(true);
      });
    });
  }

  private onEvent(ev: SessionEvent) {
    switch (ev.type) {
      case "hello":
        this.name = ev.hostname;
        break;
      case "fsRoots":
        this.settle(this.reqs, ev.req, { places: ev.places, home: ev.home });
        break;
      case "fsList":
        this.settle(this.reqs, ev.req, {
          path: ev.path,
          parent: ev.parent,
          entries: ev.entries,
          error: ev.error,
        });
        break;
      case "fsExists":
        this.settle(this.reqs, ev.req, ev.exists);
        break;
      case "fsOpened":
        if (ev.error === null) this.settle(this.files, ev.id, ev.size);
        else this.settle(this.files, ev.id, new Error(ev.error));
        break;
      case "fsData":
        this.settle(this.reads, ev.id, { data: ev.data, eof: ev.eof });
        break;
      case "fsWritten":
        this.settle(this.writes, ev.id, undefined);
        break;
      case "fsError":
        this.settle(this.reads, ev.id, new Error(ev.error));
        this.settle(this.writes, ev.id, new Error(ev.error));
        break;
      case "closed":
        this.live = false;
        this.problem = ev.reason;
        this.opts.onProblem?.(ev.reason);
        this.failAll(new Error("the session is closed"));
        break;
      default:
        break;
    }
  }

  private settle<T>(map: Map<number, Pending<T>>, key: number, value: T | Error) {
    const p = map.get(key);
    if (!p) return;
    map.delete(key);
    if (value instanceof Error) p.reject(value);
    else p.resolve(value as T);
  }

  private failAll(e: Error) {
    for (const map of [this.reqs, this.files, this.reads, this.writes] as Map<number, Pending<unknown>>[]) {
      for (const p of map.values()) p.reject(e);
      map.clear();
    }
  }

  private ask<T>(map: Map<number, Pending<T>>, key: number, send: () => Promise<void>): Promise<T> {
    const p = new Promise<T>((resolve, reject) => {
      // 回答丢了不能一直等（界面会停在 0% 看不出来在干什么）：超时当失败，用户能看见
      const timer = window.setTimeout(() => {
        if (!map.delete(key)) return;
        reject(new Error("the other computer didn't reply"));
      }, REPLY_TIMEOUT_MS);
      map.set(key, {
        resolve: (v) => {
          window.clearTimeout(timer);
          resolve(v);
        },
        reject: (e) => {
          window.clearTimeout(timer);
          reject(e);
        },
      });
      send().catch((e: Error) => {
        if (!map.delete(key)) return;
        window.clearTimeout(timer);
        reject(e);
      });
    });
    // 会话断开时这里会被拒绝；调用方可能已经不等了（比如窗口正在关），
    // 不让它冒成"未处理的拒绝"（那会被误当成界面崩了）
    p.catch(() => {});
    return p;
  }

  private get sid_(): number {
    if (this.sid === null) throw new Error("not connected");
    return this.sid;
  }

  /** 被控端能直接跳过去的位置和默认目录 */
  roots(): Promise<FsRoots> {
    const req = this.nextReq++;
    return this.ask<FsRoots>(this.reqs, req, () => api.command({ cmd: "fsRoots", sid: this.sid_, req }));
  }

  /** 列一个目录（`path` 为空 = 默认目录） */
  list(path: string): Promise<FsListing> {
    const req = this.nextReq++;
    return this.ask<FsListing>(this.reqs, req, () => api.command({ cmd: "fsList", sid: this.sid_, req, path }));
  }

  /** 这个路径存不存在（发文件之前问：决定要不要弹"是否替换"） */
  exists(path: string): Promise<boolean> {
    const req = this.nextReq++;
    return this.ask<boolean>(this.reqs, req, () => api.command({ cmd: "fsExists", sid: this.sid_, req, path }));
  }

  /** 打开一个文件：`write` 为真时清空重写；返回要读的文件大小 */
  openFile(id: number, path: string, write: boolean): Promise<number> {
    return this.ask(this.files, id, () => api.command({ cmd: "fsOpen", sid: this.sid_, file: id, path, write }));
  }

  /** 读一块（`data` 是 base64，和本地写入的命令一样，省一次转换） */
  read(id: number, offset: number, len = MAX_CHUNK): Promise<{ data: string; eof: boolean }> {
    return this.ask<{ data: string; eof: boolean }>(this.reads, id, () =>
      api.command({ cmd: "fsRead", sid: this.sid_, file: id, offset, len }),
    );
  }

  /** 写一块（`data` 是 base64） */
  write(id: number, offset: number, data: string): Promise<void> {
    return this.ask<void>(this.writes, id, () => api.command({ cmd: "fsWrite", sid: this.sid_, file: id, offset, data }));
  }

  close(id: number) {
    if (this.sid === null) return;
    void api.command({ cmd: "fsClose", sid: this.sid, file: id }).catch(() => {});
  }

  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.failAll(new Error("closed"));
    if (this.sid !== null) void api.command({ cmd: "disconnect", sid: this.sid }).catch(() => {});
    this.sid = null;
    this.live = false;
  }
}

