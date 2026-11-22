// 跨组件的操作：连接、我的设备、重新打开关闭的会话、关闭控制台（先确认）、全屏。
import { api, toProblem, type QualitySettings, type SavedDevice } from "./api";
import * as devices from "./devices";
import { bestCodec } from "./codec";
import { presetSettings } from "./presets";
import { RemoteSession } from "./session";
import { emptyDraft, store } from "./store";

export { fmtCode } from "./devices";

/** 新连接用的画质：设置里选的预设（默认"均衡"）+ 本机能解码的格式 */
export function defaultQuality(): QualitySettings {
  const st = store.get();
  return presetSettings(st.quality, bestCodec(st.decoders));
}

/** 新建连接标签里正在连的会话（关掉这个标签就放弃） */
let pending: RemoteSession | null = null;

/**
 * 怎么连：配对码（对方要求配对时；对方的身份变了、用户确认是重装之后也是重新配对，
 * 配对码认的就是对方，记着的旧公钥不作数，见 nife_remote_proto::handshake 的"身份"）
 */
export interface OpenOptions {
  pairing?: string | null;
}

/**
 * 连接控制码为 `code` 的电脑，在新建连接标签里显示进度和错误，连上后这个标签变成会话标签。
 * 已经有这台电脑的标签：切过去（断开了就重新连接）。
 */
export async function openSession(code: string, settings: QualitySettings, opts: OpenOptions = {}): Promise<void> {
  const st = store.get();
  const dup = st.sessions.find((s) => s.code === code);
  if (dup) {
    if (st.view === "new") store.closeDraft();
    store.setView(dup.id);
    if (dup.state === "closed") {
      if (opts.pairing) dup.pairWith(opts.pairing);
      else dup.reconnect();
    }
    return;
  }
  if (!st.draft) store.newTab();
  if (store.get().draft?.busy) return;
  store.patchDraft({ digits: code.split(""), busy: true, error: null });
  // 我的设备里有：带着上次的重连凭证连（对方不在线时等它上线，而不是报错），连上之前就显示它的名字；
  // 只认记着的对方公钥（带着配对码时认配对码，连上之后记下对方现在的公钥）
  const saved = st.devices.find((d) => d.code === code);
  const s = new RemoteSession(code, settings, saved, opts.pairing);
  pending = s;
  try {
    await s.open();
    if (pending !== s) {
      // 等待期间关掉了新建连接标签
      s.dispose();
      return;
    }
    store.add(s);
    // 连上了就记进我的设备（在等对方上线的，等连上时再记）；之后每次重连、改画质也更新
    if (s.sid !== null) rememberSession(s);
    s.onConnected = rememberSession;
    s.onSettingsChange = rememberQuality;
  } catch (e) {
    s.dispose();
    if (pending === s) store.patchDraft({ busy: false, error: toProblem(e) });
  } finally {
    if (pending === s) pending = null;
  }
}

/** 关闭新建连接标签（正在连接就放弃） */
export function closeDraft() {
  const st = store.get();
  if (!st.draft || st.sessions.length === 0) return;
  pending = null;
  store.closeDraft();
}

/** 关闭标签：会话标签 = 断开连接，新建连接标签 = 放弃输入 */
export function closeTab(view: string) {
  if (view === "new") closeDraft();
  else if (view !== "grid") store.remove(view);
}

/** 重新打开最近关闭的会话（Ctrl+Shift+T） */
export function reopenLast() {
  const r = store.get().recent[0];
  if (!r) return;
  store.newTab();
  void openSession(r.code, r.settings);
}

// ── 我的设备 ──

/** 读出保存的设备（打开控制台时） */
export async function loadDevices() {
  const list = await api.loadDevices().catch((e) => {
    console.warn("can't load the saved devices", e);
    return null;
  });
  // 读的时候控制台里可能已经记下了新连上的（很少见）：合在一起
  const now = store.get().devices;
  const merged = list ? devices.sortDevices([...now, ...list.filter((d) => !now.some((x) => x.code === d.code))]) : now;
  store.setDevices(merged.slice(0, devices.MAX_DEVICES), list !== null);
  if (list && now.length > 0) saveDevices();
}

// ── 在线状态（设备现在能不能连，见 nife_remote_proto::signal 的"在线状态"） ──

/** 多久问一次（只在"我的设备"和设备的详情页开着时问） */
export const PRESENCE_EVERY_MS = 15_000;

/** 上一次问的时间：切来切去、好几个地方同时问时不用重复问 */
let presenceAt = 0;

/**
 * 问服务器"这些设备现在能不能连"，更新 store 里的在线状态。
 *
 * 只问有重连凭证的（服务器只回答以前连上过的控制码，不然这个功能就能用来枚举控制码）；没登录、没连上服务器、
 * 或者服务器还是旧版本时结果是 null，界面就不显示在线状态。`force`：用户手动刷新
 */
export async function refreshPresence(force = false): Promise<void> {
  const now = Date.now();
  if (!force && now - presenceAt < PRESENCE_EVERY_MS) return;
  presenceAt = now;
  const targets = store
    .get()
    .devices.filter((d) => d.resume)
    .map((d) => ({ code: d.code, resume: d.resume! }));
  if (targets.length === 0) {
    store.setPresence(null);
    return;
  }
  try {
    const online = await api.presence(targets);
    store.setPresence(online === null ? null : Object.fromEntries(online.map((c) => [c, true])));
  } catch (e) {
    console.warn("can't ask which devices are online", e);
    store.setPresence(null);
  }
}

/** 保存：一次只存一份，存的时候取最新的列表（连续几次变化只存最后一次，也不会旧的盖掉新的） */
let saving: Promise<void> = Promise.resolve();
let dirty = false;
/** 还没存完的次数 */
let unsaved = 0;

function saveDevices() {
  if (!store.get().devicesSaved) return;
  dirty = true;
  unsaved++;
  saving = saving.then(async () => {
    try {
      if (!dirty) return;
      dirty = false;
      await api.saveDevices(store.get().devices).catch((e) => console.warn("can't save the devices", e));
    } finally {
      unsaved--;
    }
  });
}

function setDevices(list: SavedDevice[]) {
  if (list === store.get().devices) return;
  store.setDevices(list);
  saveDevices();
  syncDevicesToAccount();
}

/** 设备列表同步到账号（没登录就什么都不做）：账号服务里存着"本账号设备"的 ID */
let syncing = false;
export function syncDevicesToAccount() {
  if (syncing || !store.get().account) return;
  const codes = store.get().devices.map((d) => d.code);
  syncing = true;
  void api.accountSaveDevices(codes).catch((e) => console.warn("can't sync the devices to the account", e)).finally(() => {
    syncing = false;
  });
}

/** 登录之后：把账号里的设备拉回来（本机没有的补进"我的设备"，用设置里的画质），再把本机的补上去 */
export async function syncDevicesFromAccount(): Promise<void> {
  if (!store.get().account) return;
  try {
    const codes = await api.accountDevices();
    const now = store.get().devices;
    const missing = codes.filter((c) => !now.some((d) => d.code === c));
    if (missing.length > 0) {
      const preset = defaultQuality();
      // 从账号拉回来的只记 ID：名字先当控制码、画质用默认，连一次之后会自动补全
      let merged = now;
      for (const code of missing) {
        const seen = { code, name: devices.fmtCode(code), os: "", resume: null, quality: preset, hostKey: null };
        merged = devices.remember(merged, seen, Date.now());
      }
      merged = devices.sortDevices(merged);
      store.setDevices(merged.slice(0, devices.MAX_DEVICES), store.get().devicesSaved);
      saveDevices();
    }
    syncDevicesToAccount();
  } catch (e) {
    console.warn("can't load the devices from the account", e);
  }
}

/** 连上了：记下这台电脑（名字、系统、重连凭证、画质、时间） */
export function rememberSession(s: RemoteSession) {
  // 最近连接记在 profile.json 里（远程协助页显示）
  void api.recentPush(s.code, s.hostName).then((p) => store.setProfile(p), () => {});
  const seen = {
    code: s.code,
    name: s.hostName,
    os: s.hostOs,
    resume: s.resumeToken,
    quality: s.settings,
    hostKey: s.hostKey,
  };
  setDevices(devices.remember(store.get().devices, seen, Date.now()));
}

/** 画质改了：下次用同样的画质连（只更新已经记下的） */
function rememberQuality(s: RemoteSession) {
  setDevices(devices.patchDevice(store.get().devices, s.code, { quality: s.settings }));
}

export const renameDevice = (code: string, name: string) =>
  setDevices(devices.renameDevice(store.get().devices, code, name));
export const forgetDevice = (code: string) => setDevices(devices.forgetDevice(store.get().devices, code));
export const restoreDevice = (d: SavedDevice) => setDevices(devices.restoreDevice(store.get().devices, d));

/** 等正在保存的存完（关闭控制台之前） */
const devicesSaved = () => Promise.race([saving, new Promise((r) => setTimeout(r, 1500))]);

/** 键盘回到当前会话的远程画面（关掉主菜单之后等） */
export function focusActive() {
  const st = store.get();
  st.sessions.find((s) => s.id === st.view)?.focus();
}

/** 新建连接标签：清空输入 */
export function resetDraft() {
  store.patchDraft(emptyDraft());
}

// ── 关闭控制台窗口 ──

/**
 * 窗口要关闭了（标题栏的关闭按钮、任务栏、Alt+F4，见 App 里的 onCloseRequested）：
 * **只把界面藏起来**，不问也不退出 —— 被控端照常跑，托盘里还在，点托盘菜单"打开控制台"又出来。
 * 真正退出程序走托盘右键菜单的"退出"。
 */
export function hideOnClose(): boolean {
  // 我的设备还没存完：先把它存完（不等，别拖慢关窗口）
  if (unsaved > 0) void devicesSaved();
  void api.frame?.hide();
  return false;
}

// ── 全屏 ──

// Keyboard Lock API：全屏时捕获 Win、Alt+Tab 等系统快捷键并发送给远端
type KeyboardLock = { lock?: (codes?: string[]) => Promise<void>; unlock?: () => void };
const keyboardApi = () => (navigator as Navigator & { keyboard?: KeyboardLock }).keyboard;

// 桌面版和网页版都先进入网页全屏（Keyboard Lock 的前提），再让窗口全屏
const WEB_FULLSCREEN = true;

/** 进入 / 退出全屏（网页全屏 + 键盘锁定 + 窗口全屏）。可以重复调用 */
export async function applyFullscreen(on: boolean) {
  if (WEB_FULLSCREEN && on) {
    // 先进入网页全屏（Keyboard Lock 的前提），再让窗口全屏
    if (!document.fullscreenElement) await document.documentElement.requestFullscreen?.().catch(() => {});
    await keyboardApi()?.lock?.().catch(() => {});
  } else if (WEB_FULLSCREEN) {
    keyboardApi()?.unlock?.();
    if (document.fullscreenElement) await document.exitFullscreen().catch(() => {});
  }
  await api.setWindowFullscreen(on).catch(() => {});
}

export async function toggleFullscreen() {
  const next = !store.get().immersive;
  store.setImmersive(next);
  await applyFullscreen(next);
}

