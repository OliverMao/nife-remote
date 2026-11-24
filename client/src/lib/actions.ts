// 跨组件的操作：连接（打开远程窗口）、我的设备、关闭控制台（只藏界面）、全屏。
import { api, toProblem, type AccountDevice, type AccountInfo, type Problem, type QualitySettings, type SavedDevice } from "./api";
import * as devices from "./devices";
import { bestCodec } from "./codec";
import { DEFAULT_PRESET, presetSettings } from "./presets";
import { RemoteSession } from "./session";
import { sessionTarget } from "./session-window";
import { emptyDraft, store } from "./store";

export { fmtCode } from "./devices";

/**
 * 新连接用的画质：设置里选的预设（默认"均衡"：1080p / 30 fps / 4 Mbps）+ 默认编码格式（H.264）
 * + 默认按"文字清晰"缩放（高分屏缩小时文字更清楚，用户可以在画质面板里改）
 */
export function defaultQuality(): QualitySettings {
  const st = store.get();
  return { ...presetSettings(st.quality, bestCodec(st.decoders)), priority: "clarity" };
}

/** 窗口里就地输入配对码时用（对方要求配对时才输入） */
export interface OpenOptions {
  pairing?: string | null;
}

/**
 * 连一台电脑：**在它自己的窗口里**打开远程画面（网页版是新的浏览器标签页），
 * 管理界面只负责打开 / 切过去，远程画面不进管理界面（见 components/SessionWindow.tsx）。
 * 连接、配对、重连、出错都在那个窗口里；只有窗口本身打不开（例如浏览器拦了新标签页）时才返回问题。
 */
export async function openSession(
  code: string,
  settings: QualitySettings,
  opts: OpenOptions = {},
): Promise<Problem | null> {
  // 新建连接页（如果在看）显示一下正在打开
  if (store.get().draft) store.patchDraft({ digits: code.split(""), busy: true, error: null });
  try {
    await api.openSessionWindow({ code, settings, pairing: opts.pairing ?? null, display: null });
    if (store.get().draft) store.patchDraft({ busy: false, error: null });
    return null;
  } catch (e) {
    const problem = toProblem(e);
    if (store.get().draft) store.patchDraft({ busy: false, error: problem });
    return problem;
  }
}

/** 断开一路：远程窗口里 = 断开并关掉这个窗口（管理界面的页面没有"关掉"这一说） */
export function closeTab(view: string) {
  if (!sessionTarget()) return;
  if (view !== "new" && view !== "grid") store.remove(view);
  closeWindow();
}

/** 关掉这个窗口（网页版：关掉这个标签页） */
function closeWindow() {
  if (api.frame) void api.frame.destroy();
  else window.close();
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
}

/**
 * 把**本机**登记到账号的设备列表（没登录、还没拿到本机控制码时什么都不做）。
 *
 * "我的设备"= 登录同一个账号的设备：每台登录之后把自己登记上去，别的设备从这里就能看到它、直接连它。
 */
let registering = false;
export async function registerSelfWithAccount(): Promise<void> {
  const st = store.get();
  const account = st.account;
  const code = st.host?.code;
  if (registering || !account || !code) return;
  const self = String(code);
  registering = true;
  try {
    // 自己的控制台公钥也登记上去：同账号的其它设备凭它认本机，不用输配对码
    const key = await api.controllerPublicKey().catch(() => "");
    const list = await api.accountDevices();
    const before = list.find((d) => d.code === self)?.key ?? "";
    if (account.devices?.includes(self) && (!key || before === key)) return;
    const next: AccountDevice[] = list.some((d) => d.code === self)
      ? list.map((d) => (d.code === self && key ? { ...d, code: self, key } : d))
      : [...list, { code: self, key, online: true, name: "", os: "" }];
    await api.accountSaveDevices(next);
    store.setAccount(applyAccountDevices(account, next));
  } catch (e) {
    console.warn("can't register this computer on the account", e);
  } finally {
    registering = false;
  }
}

/** 本机报个到：账号里的其它设备据此看到我们在线（没登录、还没有控制码时什么都不做） */
export async function heartbeatToAccount(): Promise<void> {
  const st = store.get();
  const code = st.host?.code;
  if (!st.account || !code) return;
  try {
    await api.accountHeartbeat(String(code));
  } catch (e) {
    console.warn("can't reach the account service for the heartbeat", e);
  }
}

/** 账号设备列表 → store 里的 account（控制码给界面，公钥用来免配对） */
export function applyAccountDevices(account: AccountInfo, list: AccountDevice[]): AccountInfo {
  return {
    ...account,
    devices: list.map((d) => d.code),
    deviceKeys: Object.fromEntries(list.filter((d) => d.key).map((d) => [d.code, d.key])),
    // 在线状态和名字都由账号服务维护（设备连上账号服务就报到）：不依赖"本机有没有连过它"
    deviceOnline: Object.fromEntries(list.map((d) => [d.code, !!d.online])),
    deviceInfo: Object.fromEntries(list.map((d) => [d.code, { name: d.name ?? "", os: d.os ?? "" }])),
  };
}

/** 登录之后：把账号里的设备列表拉回来（"我的设备"显示的就是这个），再把自己登记上去 */
export async function refreshAccountDevices(): Promise<void> {
  const account = store.get().account;
  if (!account) return;
  try {
    const list = await api.accountDevices();
    store.setAccount(applyAccountDevices(account, list));
    void registerSelfWithAccount();
    void trustAccountSiblings();
  } catch (e) {
    console.warn("can't load the devices of the account", e);
  }
}

/**
 * 把登录同一个账号的其它设备的控制台公钥交给本机被控端：
 * 这些设备连本机时不用输配对码（同账号直接信任）
 */
export async function trustAccountSiblings(): Promise<void> {
  const st = store.get();
  const account = st.account;
  const self = st.host?.code ? String(st.host.code) : null;
  if (!account) return;
  const keys = Object.entries(account.deviceKeys ?? {})
    .filter(([code, key]) => code !== self && key)
    .map(([, key]) => key);
  try {
    await api.trustControllers(keys);
  } catch (e) {
    console.warn("can't pass the account devices to the host", e);
  }
}

/**
 * 把一台设备从账号的设备列表里移除（它下次登录还会自己登记回来）。
 * 本机的连接记录（重连凭证、画质）另外由 forgetDevice 清。
 */
export async function removeAccountDevice(code: string): Promise<void> {
  const account = store.get().account;
  if (!account) return;
  try {
    const list = (await api.accountDevices()).filter((d) => d.code !== code);
    store.setAccount(applyAccountDevices(account, list));
    await api.accountSaveDevices(list);
    void trustAccountSiblings();
  } catch (e) {
    console.warn("can't remove the device from the account", e);
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
export function rememberQuality(s: RemoteSession) {
  setDevices(devices.patchDevice(store.get().devices, s.code, { quality: s.settings }));
}

export const renameDevice = (code: string, name: string) =>
  setDevices(devices.renameDevice(store.get().devices, code, name));
export const forgetDevice = (code: string) => setDevices(devices.forgetDevice(store.get().devices, code));

/**
 * 清理本地缓存：临时文件（Rust 那边删 WebView 数据目录和残留）+ 这台电脑上的界面偏好
 * （画质、画面增强、本机光标）。控制码、"我的设备"、登录状态都不动。返回删掉的字节数。
 */
export async function clearCache(): Promise<number> {
  let freed = 0;
  try {
    freed = await api.clearCache();
  } catch {
    // 删不掉（文件被占用等）不算失败：界面偏好照样清
  }
  try {
    for (const key of Object.keys(globalThis.localStorage ?? {})) {
      if (key.startsWith("nife-remote.")) globalThis.localStorage.removeItem(key);
    }
  } catch {
    // 存不了 / 删不了：只在这次有效
  }
  store.setQuality(DEFAULT_PRESET);
  store.setEnhance("off");
  store.setLocalCursor(true);
  return freed;
}

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

