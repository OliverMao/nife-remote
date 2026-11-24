// 全局状态：远程窗口里的会话、开着的远程窗口、我的设备、服务器状态等。
// "我的设备"保存在 config.toml 里（见 actions.ts 的 saveDevices）；画面增强、本地光标是这台电脑自己显示的偏好，
// 和对方无关，存在 WebView / 浏览器的 localStorage 里；其余只在窗口打开期间有效。
import { useCallback, useSyncExternalStore } from "react";
import type { AccountInfo, AppInfo, Codec, HostStatus, OpenSession, Problem, Profile, SavedDevice, ServerState } from "./api";
import { t } from "./i18n";
import { ENHANCE_LEVELS, type EnhanceLevel } from "./enhance";
import { DEFAULT_PRESET, PRESETS, type PresetId } from "./presets";
import type { RemoteSession } from "./session";

/** 管理界面显示哪一页："new" / "assist" = 远程协助，"host" = 被控中，"device" = 设备详情 */
export type View = string;

/** 新建连接页里正在输入的内容 */
export interface Draft {
  digits: string[];
  preset: PresetId;
  /** 配对码（对方要求配对时才显示这一栏，见 ConnectView） */
  pairing: string;
  busy: boolean;
  /** 保存类型而不是文字：切换语言后错误提示也跟着变 */
  error: Problem | null;
}

interface State {
  /** 远程窗口里的那一个会话（管理界面不连别人，它一直是空的，见 components/SessionWindow.tsx） */
  sessions: RemoteSession[];
  /**
   * 开着的远程窗口（桌面版：一个控制码一个窗口，见 src-tauri/src/viewer.rs）。
   * 远程画面不在这个窗口里，所以管理与被控窗口各一份状态；网页版是浏览器的标签页，一直是空的
   */
  open: OpenSession[];
  view: View;
  /** 新建连接页（显示发起协助 / 输入控制码的那一页） */
  draft: Draft | null;
  /** 设备页显示的是哪台（`view === "device"` 时有效，见 DeviceView） */
  device: string | null;
  /** 本机偏好和记录（profile.json，见 Rust 的 profile 模块） */
  profile: Profile;
  /** 现在登录的账号（账号服务里，没登录是 null） */
  account: AccountInfo | null;
  /** 账号状态读出来了没有（没读出来之前不画界面，避免闪一下登录页） */
  accountLoaded: boolean;
  /** 我的设备：连接过的电脑，最近连过的在前面 */
  devices: SavedDevice[];
  /** 我的设备能不能保存（桌面版：config.toml）；网页版只在本页面的内存里 */
  devicesSaved: boolean;
  /**
   * 我的设备里哪些现在能连（控制码 → 在线）：问服务器要的，见 lib/actions 的 refreshPresence。
   * null = 问不出来（没连上服务器、或者服务器是旧版本还不懂这个问题），界面就不显示在线状态
   */
  presence: Record<string, boolean> | null;
  server: ServerState;
  /**
   * 最近一次连不上服务器的原因。断开后会自动重试（状态在 disconnected 和 connecting 之间来回），
   * 重试期间也保留它，界面就不会一会儿显示错误、一会儿又消失；连上后清除
   */
  serverProblem: Problem | null;
  app: AppInfo | null;
  /** 本机连接码等（桌面版；网页版没有被控端，一直为 null） */
  host: HostStatus | null;
  decoders: Codec[];
  immersive: boolean;
  /** 设置里选的画质预设（新连接用；老连接在会话工具栏 / 设备页里单独改） */
  quality: PresetId;
  /** 画面增强（所有会话通用，见 enhance.ts） */
  enhance: EnhanceLevel;
  /**
   * 对方把光标画进了画面里时（拖动、缩放窗口等）也显示本机的光标：跟手，只是和画面里的那个一起有两个。
   * 关掉时只看画面里的。所有会话通用，默认打开（见 session.ts 的 applyCursor）
   */
  localCursor: boolean;
}

const ENHANCE_KEY = "nife-remote.enhance";
const QUALITY_KEY = "nife-remote.quality";
const LOCAL_CURSOR_KEY = "nife-remote.localCursor";

/** 上次选的画面增强（读不到就是关） */
export function loadEnhance(): EnhanceLevel {
  try {
    const v = globalThis.localStorage?.getItem(ENHANCE_KEY);
    // 默认"弱"：画面放大显示时更清楚，代价很小（用户可以在设置里关掉）
    return ENHANCE_LEVELS.find((l) => l === v) ?? "low";
  } catch {
    return "off";
  }
}

/** 本地光标的设置（读不到就是打开） */
export function loadLocalCursor(): boolean {
  try {
    return globalThis.localStorage?.getItem(LOCAL_CURSOR_KEY) !== "0";
  } catch {
    return true;
  }
}

/** 设置里的画质（读不到就是"均衡"） */
export function loadQuality(): PresetId {
  try {
    const v = globalThis.localStorage?.getItem(QUALITY_KEY);
    return PRESETS.find((p) => p.id === v)?.id ?? DEFAULT_PRESET;
  } catch {
    return DEFAULT_PRESET;
  }
}

/** 记下这台电脑的显示偏好 */
function saveLocal(key: string, value: string) {
  try {
    globalThis.localStorage?.setItem(key, value);
  } catch {
    // 存不了（例如浏览器禁用了存储）：只在这次有效
  }
}

export const emptyDraft = (): Draft => ({
  digits: Array(6).fill(""),
  preset: DEFAULT_PRESET,
  pairing: "",
  busy: false,
  error: null,
});

let state: State = {
  sessions: [],
  open: [],
  view: "new",
  draft: emptyDraft(),
  device: null,
  profile: { allowAssist: true, verify: "pairing", favorites: [], recent: [] },
  account: null,
  accountLoaded: false,
  devices: [],
  devicesSaved: false,
  presence: null,
  server: { state: "connecting" },
  serverProblem: null,
  app: null,
  host: null,
  decoders: ["h264"],
  immersive: false,
  quality: loadQuality(),
  enhance: loadEnhance(),
  localCursor: loadLocalCursor(),
};

const listeners = new Set<() => void>();

function set(patch: Partial<State>) {
  state = { ...state, ...patch };
  syncVisibility();
  for (const l of listeners) l();
}

/** 只有正在显示的会话保持全帧率；只有当前查看的会话播放声音（远程窗口里只有一个，见 components/SessionWindow.tsx） */
function syncVisibility() {
  for (const s of state.sessions) {
    s.setVisible(state.view === s.id);
    s.setForeground(state.view === s.id);
  }
}

/**
 * 给用户看的服务器状态：连不上之后在重试的，连上之前都算离线（不在「离线」和「连接中」之间来回跳）；
 * 只有第一次连接（还没出过问题）时是连接中
 */
export type ServerStatus = "connected" | "connecting" | "offline";
export const serverStatus = (st: State): ServerStatus =>
  st.server.state === "connected" ? "connected" : st.serverProblem ? "offline" : "connecting";

export const store = {
  get: () => state,
  subscribe: (fn: () => void) => {
    listeners.add(fn);
    return () => listeners.delete(fn);
  },
  setServer: (server: ServerState) => {
    const was = state.server.state;
    const serverProblem =
      server.state === "disconnected" ? server.reason : server.state === "connected" ? null : state.serverProblem;
    set({ server, serverProblem });
    if (server.state === "connected" && was !== "connected") for (const s of state.sessions) s.serverConnected();
  },
  /** 在等的被控端重新上线了（见 RemoteSession.hostOnline） */
  hostOnline: (code: number) => {
    for (const s of state.sessions) s.hostOnline(code);
  },
  setApp: (app: AppInfo) => set({ app }),
  /** 本机连接码变了（连上服务器、换了码、有人连上来、有人断开） */
  setHost: (host: HostStatus) => set({ host }),
  setDecoders: (decoders: Codec[]) => set({ decoders }),
  /** 开着的远程窗口变了（打开、关掉、里面的会话状态变了） */
  setOpen: (open: OpenSession[]) => set({ open }),
  setView: (view: View) => {
    if (view === "new" && !state.draft) return store.newTab();
    set({ view, immersive: false });
  },
  /** 打开一台设备的详情页（照 UU：点左栏的设备先进这个页面） */
  showDevice: (code: string) => set({ view: "device", device: code, immersive: false }),
  /** 打开发起协助的页面（左栏"开始协助"） */
  showAssist: () => set({ view: "assist", immersive: false }),
  /** 被控中那一页（左栏被控卡的放大按钮） */
  showHost: () => set({ view: "host", immersive: false }),
  setImmersive: (immersive: boolean) => set({ immersive }),
  /** 画质：下次连接用它（正在连的会话不跟着变，见 QualityPanel） */
  setQuality: (quality: PresetId) => {
    if (quality === state.quality) return;
    saveLocal(QUALITY_KEY, quality);
    set({ quality });
  },
  /** 画面增强：记下来，所有会话马上按它重画 */
  setEnhance: (enhance: EnhanceLevel) => {
    if (enhance === state.enhance) return;
    saveLocal(ENHANCE_KEY, enhance);
    set({ enhance });
    for (const s of state.sessions) s.refreshLayout();
  },
  /** 画面里有光标时显不显示本机的光标：记下来，所有会话马上换指针 */
  setLocalCursor: (localCursor: boolean) => {
    if (localCursor === state.localCursor) return;
    saveLocal(LOCAL_CURSOR_KEY, localCursor ? "1" : "0");
    set({ localCursor });
    for (const s of state.sessions) s.refreshCursor();
  },

  // ── 远程窗口里的会话 ──

  /** 新建连接页（显示发起协助 / 输入控制码的那一页） */
  newTab: () => set({ draft: state.draft ?? emptyDraft(), view: "new", immersive: false }),
  patchDraft: (patch: Partial<Draft>) => {
    if (state.draft) set({ draft: { ...state.draft, ...patch } });
  },
  /** 连上了（远程窗口里的那一个）：显示它 */
  add: (s: RemoteSession) => set({ sessions: [s], view: s.id }),
  /** 断开并移除（远程窗口关掉时用，见 actions.ts 的 closeTab） */
  remove: (id: string) => {
    const s = state.sessions.find((x) => x.id === id);
    if (!s) return;
    s.dispose();
    const sessions = state.sessions.filter((x) => x.id !== id);
    set({ sessions, view: sessions[0]?.id ?? "new", immersive: false });
  },
  setDevices: (devices: SavedDevice[], saved = state.devicesSaved) => set({ devices, devicesSaved: saved }),
  /** 我的设备里哪些现在能连（null = 不知道） */
  setPresence: (presence: Record<string, boolean> | null) => set({ presence }),
  /** 桌面端从 profile.json 读到的偏好（网页版是内存里的默认值） */
  setProfile: (profile: Profile) => set({ profile }),
  setAccount: (account: AccountInfo | null) => set({ account, accountLoaded: true }),

};

export function useStore<T>(select: (s: State) => T): T {
  return useSyncExternalStore(store.subscribe, () => select(state));
}

/** 会话显示的名字：在我的设备里起过名字就用它，否则用对方的设备名（界面上不显示控制码） */
export function useSessionTitle(s: RemoteSession): string {
  const alias = useStore((st) => st.devices.find((d) => d.code === s.code)?.alias ?? null);
  return alias || s.hostName || t("devices.unnamed");
}

/** 订阅单个会话的变化 */
export function useSession(s: RemoteSession | undefined): number {
  const sub = useCallback(
    (fn: () => void) => {
      if (!s) return () => {};
      const off = s.subscribe(fn);
      return () => void off();
    },
    [s],
  );
  const get = useCallback(() => (s ? s.getVersion() : 0), [s]);
  return useSyncExternalStore(sub, get);
}
