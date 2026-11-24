// 网页版：浏览器打开 nife-remote-web 的端口，控制端服务跑在那台机器上（client/web/src/console.rs）。
//
// 会话和桌面版一样走 WebSocket（console.ts），连的是同一个端口的 `/ws`。网页版自己的：
// * 剪贴板在浏览器里同步（navigator.clipboard），规则与桌面版相同：只同步连接之后的变化，并抑制回声；
// * 没有被控端模式、托盘和窗口；"我的设备"不保存。
import type {
  AccountDevice,
  AccountInfo,
  AppInfo,
  Backend,
  Command,
  ConnectOptions,
  FsEntry,
  FsPlace,
  HostStatus,
  OpenSession,
  Problem,
  Profile,
  SessionEvent,
  SessionInfo,
} from "../api";
import { ConsoleLink, type MediaView } from "../console";
import { getLang } from "../i18n";
import type { SessionTarget } from "../session-window";

/** 一个会话的剪贴板同步 */
interface Clip {
  /** connect 返回之后才知道 */
  sid: number | null;
  /** 是否同步 */
  enabled: boolean;
  /** 双方已经一致的剪贴板文本；null 表示还没读到过本机剪贴板（第一次读到的内容不发送） */
  synced: string | null;
}

/** 与 nife_remote_proto::session::MAX_CLIPBOARD_TEXT 相同（按字符数粗略限制，服务端还会按字节检查） */
/** 网页版没有本地文件，账号也只在这次页面里有效 */
let webAccount: { info: AccountInfo; password: string } | null = null;

/** 网页版没有本地文件，这份偏好只放在内存里 */
let defaultProfile: Profile = { allowAssist: true, verify: "pairing", favorites: [], recent: [] };

const MAX_CLIPBOARD = 1 << 20;
const CLIPBOARD_POLL_MS = 1000;

/** 同一个地址（也可能在反向代理的子路径下）的 `ws` */
function consoleUrl(): string {
  const url = new URL("ws", location.href);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  url.search = "";
  url.hash = "";
  return url.href;
}

class WebConsole extends ConsoleLink implements Backend {
  readonly kind = "web" as const;
  /** 在浏览器的标签页里，没有自己的窗口 */
  readonly frame = null;

  private clips = new Set<Clip>();
  /** 最近一次读到 / 写入的本机剪贴板文本 */
  private clipSeen: string | null = null;
  /** 页面没有焦点时浏览器不让写剪贴板：先记下，回到页面时再写 */
  private clipToWrite: { text: string; revision: number } | null = null;
  private clipRevision = 0;
  private clipWriteTask: Promise<void> | null = null;
  private clipBusy = false;

  constructor() {
    super(async () => consoleUrl());
    setInterval(() => {
      if (document.hasFocus()) void this.syncClipboard(false);
    }, CLIPBOARD_POLL_MS);
    // 在别的程序里复制完切回来：马上同步
    window.addEventListener("focus", () => void this.syncClipboard(false));
  }

  override async appInfo(): Promise<AppInfo> {
    // 界面语言由浏览器决定
    return { ...(await super.appInfo()), lang: getLang() };
  }

  override async connect(
    code: string,
    opts: ConnectOptions,
    view: MediaView,
    onEvent: (ev: SessionEvent) => void,
    clipboard: boolean,
  ): Promise<SessionInfo> {
    const clip: Clip = { sid: null, enabled: clipboard, synced: this.clipSeen };
    this.clips.add(clip);
    const events = (ev: SessionEvent) => {
      if (ev.type === "clipboard") return this.remoteClipboard(clip, ev.text);
      if (ev.type === "closed") this.clips.delete(clip);
      onEvent(ev);
    };
    try {
      const info = await super.connect(code, opts, view, events, clipboard);
      clip.sid = info.sid;
      return info;
    } catch (e) {
      this.clips.delete(clip);
      throw e;
    }
  }

  /** 剪贴板同步的开关在浏览器里 */
  override command(c: Command): Promise<void> {
    if (c.cmd === "clipboardSync") return this.clipboardSync(c.sid, c.enabled);
    if (c.cmd === "disconnect") for (const clip of this.clips) if (clip.sid === c.sid) this.clips.delete(clip);
    return super.command(c);
  }

  // ───────────────────────── 剪贴板 ─────────────────────────

  /** `ask`：没有权限时请求权限（只在用户操作时这样做，浏览器会弹出询问） */
  private async readClipboard(ask: boolean): Promise<string | null> {
    if (!navigator.clipboard?.readText) return null;
    if (!ask) {
      try {
        const p = await navigator.permissions.query({ name: "clipboard-read" as PermissionName });
        if (p.state !== "granted") return null;
      } catch {
        // 不支持查询这个权限（例如 Firefox）：只在用户操作时读
        return null;
      }
    }
    try {
      return await navigator.clipboard.readText();
    } catch {
      return null;
    }
  }

  private writeClipboard(text: string): Promise<void> {
    this.clipToWrite = { text, revision: ++this.clipRevision };
    return this.flushClipboard();
  }

  /** 串行写入，只保留最新待执行值；旧调用的失败不能恢复已经被替换的重试。 */
  private flushClipboard(): Promise<void> {
    if (this.clipWriteTask) return this.clipWriteTask;
    const drain = async () => {
      while (this.clipToWrite !== null) {
        const update = this.clipToWrite;
        try {
          await navigator.clipboard.writeText(update.text);
        } catch {
          if (this.clipToWrite === update) break; // 等下次获得焦点再试
          continue; // 已有更新的值，继续写最新的
        }
        if (this.clipToWrite === update) {
          this.clipSeen = update.text;
          this.clipToWrite = null;
        }
      }
    };
    this.clipWriteTask = drain().finally(() => {
      this.clipWriteTask = null;
    });
    return this.clipWriteTask;
  }

  /** 被控端复制了文本 */
  private remoteClipboard(clip: Clip, text: string) {
    if (!clip.enabled) return;
    clip.synced = text;
    void this.writeClipboard(text);
  }

  /** 本机剪贴板有变化就发给打开了同步的会话 */
  private async syncClipboard(ask: boolean) {
    if (this.clipBusy) return;
    this.clipBusy = true;
    try {
      if (document.hasFocus()) await this.flushClipboard();
      if (this.clipToWrite !== null || this.clipWriteTask !== null) return;
      if (![...this.clips].some((c) => c.enabled && c.sid !== null)) return;
      const revision = this.clipRevision;
      const text = await this.readClipboard(ask);
      // 读取期间收到远端更新：这个读结果属于更早的状态，不能回传。
      if (revision !== this.clipRevision || text === null || text === "" || text.length > MAX_CLIPBOARD) return;
      this.clipSeen = text;
      for (const c of this.clips) {
        if (c.synced === null) {
          c.synced = text;
        } else if (c.sid !== null && c.synced !== text) {
          // 关闭同步的会话也要记下，否则重新打开时会把旧内容发过去
          c.synced = text;
          if (c.enabled) void super.command({ cmd: "clipboard", sid: c.sid, text }).catch(() => {});
        }
      }
    } finally {
      this.clipBusy = false;
    }
  }

  private async clipboardSync(sid: number, enabled: boolean) {
    const clip = [...this.clips].find((c) => c.sid === sid);
    if (!clip) return;
    clip.enabled = enabled;
    if (enabled) {
      // 用户刚打开同步：这时可以请求读剪贴板的权限。现有内容不发送，只同步之后的变化
      const revision = this.clipRevision;
      const text = await this.readClipboard(true);
      if (revision === this.clipRevision && this.clipToWrite === null && text !== null) {
        clip.synced = text;
        this.clipSeen = text;
      }
    }
  }

  async localClipboardText() {
    const text = await this.readClipboard(true);
    if (text === null) throw { kind: "clipboardBlocked" } satisfies Problem;
    return text;
  }

  // ───────────────────────── 远程窗口（浏览器的标签页） ─────────────────────────

  /** 已经打开的远程标签页（控制码 → 那个窗口）：再点同一台设备只是切过去，不重新连 */
  private sessionTabs = new Map<string, Window>();

  /** 远程画面在新标签页里：和桌面版一样，一台电脑一个窗口，显示器可以拖出来单独成窗口（地址里带上要连的那台） */
  async openSessionWindow(target: SessionTarget) {
    const name = `nife-session-${target.code}${target.display === null ? "" : `-d${target.display}`}`;
    const known = this.sessionTabs.get(name);
    if (known && !known.closed) {
      known.focus();
      return;
    }
    const url = new URL(location.href);
    url.hash = "";
    url.search = "";
    url.searchParams.set("session", JSON.stringify(target));
    const tab = window.open(url.href, name);
    if (!tab) throw { kind: "failed", detail: "the browser blocked the new tab" } satisfies Problem;
    this.sessionTabs.set(name, tab);
  }

  // 标签页归浏览器管，控制台不知道还有哪些开着（左栏的"会话中"只桌面版有）
  async sessionWindows(): Promise<OpenSession[] | null> {
    return null;
  }
  onSessionWindows() {
    return () => {};
  }
  async focusSession() {}
  async reportSession() {}

  // 网页版没有被控端、托盘和窗口。
  // 开发时加 ?preview=host 可以看桌面版才有的"本机被控中"卡片（只是看样式用，正式构建里没有）
  async hostStatus(): Promise<HostStatus | null> {
    // 开发模式由 main.tsx 打上标记（正式构建里没有这个标记，也就不可能被 URL 参数打开）
    const dev = (globalThis as { __NIFE_DEV__?: boolean }).__NIFE_DEV__ === true;
    if (dev && typeof location !== "undefined" && location.search.includes("preview=host")) {
      return {
        server: { state: "connected" },
        code: 260778,
        controllers: [{ sessionId: 1, peer: { name: "PC-01", ip: "10.0.0.8" }, safety: null }],
        pairing: "430316",
        pairingLocked: false,
        paired: 0,
      };
    }
    return null;
  }
  onHostStatus() {
    return () => {};
  }
  async setLanguage() {}
  onLangChanged() {
    return () => {};
  }
  async setWindowFullscreen() {}
  onWindowFullscreenExit() {
    return () => {};
  }
  // 网页版不保存"我的设备"（不写浏览器的存储）：只在本页面的内存里，刷新后清空。
  // 开发时加 ?preview=host 也给两台假的，方便看设备页
  async loadDevices() {
    const dev = (globalThis as { __NIFE_DEV__?: boolean }).__NIFE_DEV__ === true;
    if (dev && typeof location !== "undefined" && location.search.includes("preview=host")) {
      return [
        {
          code: "260778",
          alias: null,
          name: "MAO7",
          os: "Windows 11",
          resume: null,
          lastUsed: Date.now() - 60_000,
          quality: null,
          hostKey: null,
        },
        {
          code: "318890",
          alias: "公司电脑",
          name: "DESKTOP-78TN",
          os: "Windows 11",
          resume: null,
          lastUsed: Date.now() - 3_600_000,
          quality: null,
          hostKey: null,
        },
      ];
    }
    return null;
  }
  async saveDevices() {}
  // 网页版没有被控端，也就没有配对码
  async savePairing(): Promise<void> {
    throw new Error("the web console has no host, so there is no pairing code");
  }
  async kickAll(): Promise<void> {
    throw new Error("the web console has no host");
  }
  async kickSession(): Promise<void> {
    throw new Error("the web console has no host");
  }

  // 网页版没有本地文件：这份偏好放在内存里（刷新就回到默认值）
  async accountGet(): Promise<AccountInfo | null> {
    // 开发预览（?preview=host）：假装已登录，方便看主界面。__NIFE_DEV__ 只在 dev 里由 main.tsx 设上
    // （不能用 import.meta.env：这个文件也会被 node 的测试加载）
    if (!webAccount && (globalThis as { __NIFE_DEV__?: boolean }).__NIFE_DEV__ && new URLSearchParams(location.search).has("preview")) {
      return { id: "udev", name: "niferemote", createdAt: 0, devices: [], deviceKeys: {}, deviceOnline: {}, deviceInfo: {} };
    }
    return webAccount?.info ?? null;
  }
  async accountRefresh(): Promise<AccountInfo | null> {
    // 和 accountGet 一致（开发预览里也是那个假账号）
    return this.accountGet();
  }
  async accountRegister(name: string, password: string): Promise<AccountInfo> {
    webAccount = { info: { id: "uweb", name, createdAt: Math.floor(Date.now() / 1000), devices: [], deviceKeys: {}, deviceOnline: {}, deviceInfo: {} }, password };
    return webAccount.info;
  }
  async accountLogin(name: string, password: string): Promise<AccountInfo> {
    if (!webAccount || webAccount.info.name !== name || webAccount.password !== password) {
      throw new Error("no such account on this computer");
    }
    return webAccount.info;
  }
  async accountLogout(): Promise<void> {
    webAccount = null;
  }
  async accountChangePassword(oldPassword: string, newPassword: string): Promise<AccountInfo> {
    if (!webAccount || webAccount.password !== oldPassword) {
      throw new Error("the current password is wrong");
    }
    webAccount.password = newPassword;
    return webAccount.info;
  }
  async accountDevices(): Promise<AccountDevice[]> {
    return [];
  }
  async accountSaveDevices(): Promise<void> {}
  async accountHeartbeat(): Promise<void> {}

  // 网页版没有设备连接（没有本机被控端），列表来自它自己的登录态
  onAccountDevices(): () => void {
    return () => {};
  }

  // 网页版：文件传输开成一个浏览器标签页（左边那一栏看不到本机文件）
  async openFilesWindow({ code, name }: { code: string; name: string }): Promise<void> {
    const url = `${location.pathname}?files=${encodeURIComponent(JSON.stringify({ code, name }))}`;
    const w = window.open(url, `nife-files-${code}`);
    if (w) w.focus();
    else throw new Error("the browser blocked the new tab");
  }

  // 网页版没有"设备常连接"（见 src-tauri/src/presence.rs）：文件传输窗口不提供
  async peerFsOpen(): Promise<number> {
    throw { kind: "failed", detail: "网页版不支持文件传输" };
  }
  async peerFsSend(): Promise<void> {
    throw { kind: "failed", detail: "网页版不支持文件传输" };
  }
  peerFsClose() {}
  onFsPeer(): () => void {
    return () => {};
  }

  // 网页版看不见本机的文件系统（浏览器的沙箱）：文件传输窗口里左边那一栏不显示
  async localRoots(): Promise<{ places: FsPlace[]; home: string }> {
    return { places: [], home: "" };
  }
  async localList(): Promise<{ path: string; parent: string | null; entries: FsEntry[] }> {
    return { path: "", parent: null, entries: [] };
  }
  async localExists(): Promise<boolean> {
    return false;
  }
  async localOpen(): Promise<number> {
    throw new Error("the web console can't read local files");
  }
  async localRead(): Promise<{ data: string; eof: boolean }> {
    throw new Error("the web console can't read local files");
  }
  async localWrite(): Promise<void> {
    throw new Error("the web console can't write local files");
  }
  async localClose(): Promise<void> {}
  async controllerPublicKey(): Promise<string> {
    return "";
  }
  async trustControllers(): Promise<void> {}

  async profileGet(): Promise<Profile> {
    return { ...defaultProfile };
  }
  async profileSetAllow(allow: boolean): Promise<Profile> {
    return { ...defaultProfile, allowAssist: allow };
  }
  async profileSetVerify(verify: Profile["verify"]): Promise<Profile> {
    return { ...defaultProfile, verify };
  }
  async favoriteToggle(code: string): Promise<Profile> {
    const favorites = defaultProfile.favorites.includes(code)
      ? defaultProfile.favorites.filter((c) => c !== code)
      : [code, ...defaultProfile.favorites];
    defaultProfile = { ...defaultProfile, favorites };
    return { ...defaultProfile };
  }
  async recentPush(code: string, name: string): Promise<Profile> {
    const recent = [{ code, name, at: Math.floor(Date.now() / 1000) }, ...defaultProfile.recent.filter((r) => r.code !== code)].slice(0, 20);
    defaultProfile = { ...defaultProfile, recent };
    return { ...defaultProfile };
  }
  async recentDrop(code?: string): Promise<Profile> {
    const recent = code ? defaultProfile.recent.filter((r) => r.code !== code) : [];
    defaultProfile = { ...defaultProfile, recent };
    return { ...defaultProfile };
  }
  // 网页版跑在容器里，浏览器这边没有本地临时文件（界面偏好由界面自己清）
  async clearCache(): Promise<number> {
    return 0;
  }
  // 网页版的服务器地址和指纹是启动容器时的环境变量，界面里改不了（见 client/web/.env.example）
  async relayRefresh(): Promise<boolean> {
    return false;
  }
  async restartApp(): Promise<void> {
    throw new Error("the web console can't restart itself");
  }
}

export function webBackend(): Backend {
  return new WebConsole();
}
