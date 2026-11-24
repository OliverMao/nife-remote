// 桌面版：控制台是 Tauri 窗口，控制端服务在同一个进程里（client/src-tauri）。
//
// 会话和网页版一样走 WebSocket（console.ts）：连的是本程序只监听 127.0.0.1 的服务，地址由 `console_url` 命令给出，
// 带着随机令牌（见 src-tauri/src/console.rs）。窗口、语言、我的设备这些还是 Tauri 的命令和事件。
// 剪贴板由 Rust 同步这台电脑的。
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type {
  AccountDevice,
  AccountInfo,
  Backend,
  FsEntry,
  FsPeerMessage,
  FsPlace,
  HostStatus,
  OpenSession,
  OpenSessionState,
  Profile,
  SavedDevice,
  WindowFrame,
} from "../api";
import { ConsoleLink } from "../console";
import type { Lang } from "../i18n";
import type { SessionTarget } from "../session-window";

/** 把异步注册的监听变成同步返回的取消函数 */
function unlistenLater(p: Promise<UnlistenFn>): () => void {
  return () => void p.then((f) => f());
}

function windowFrame(): WindowFrame {
  const w = getCurrentWindow();
  return {
    minimize: () => w.minimize(),
    toggleMaximize: () => w.toggleMaximize(),
    isMaximized: () => w.isMaximized(),
    onResized: (cb) => unlistenLater(w.onResized(() => cb())),
    close: () => w.close(),
    destroy: () => w.destroy(),
    hide: () => w.hide(),
    setTitle: (title) => w.setTitle(title),
    onCloseRequested: (cb) =>
      unlistenLater(
        w.onCloseRequested((e) => {
          // 关窗口 = 只藏界面（见 actions.ts 的 hideOnClose）：这里拦住真正的关闭，别的都不做
          if (cb()) return;
          e.preventDefault();
        }),
      ),
  };
}

class DesktopConsole extends ConsoleLink implements Backend {
  readonly kind = "desktop" as const;
  readonly frame = windowFrame();

  constructor() {
    super(() => invoke<string>("console_url"));
  }

  localClipboardText() {
    return invoke<string>("clipboard_text");
  }

  hostStatus() {
    return invoke<HostStatus>("host_status");
  }

  onHostStatus(cb: (s: HostStatus) => void) {
    return unlistenLater(listen<HostStatus>("host-status", (e) => cb(e.payload)));
  }

  loadDevices() {
    return invoke<SavedDevice[]>("devices_load");
  }

  saveDevices(devices: SavedDevice[]) {
    return invoke<void>("devices_save", { devices });
  }

  // 远程画面在它自己的窗口里（见 src-tauri/src/viewer.rs）
  openSessionWindow(target: SessionTarget) {
    return invoke<void>("session_open", {
      code: target.code,
      settings: target.settings,
      pairing: target.pairing,
      display: target.display,
    });
  }

  sessionWindows() {
    return invoke<OpenSession[]>("session_windows");
  }

  onSessionWindows(cb: (list: OpenSession[]) => void) {
    return unlistenLater(listen<OpenSession[]>("sessions-changed", (e) => cb(e.payload)));
  }

  focusSession(code: string, display: number | null) {
    return invoke<void>("session_focus", { code, display });
  }

  reportSession(code: string, display: number | null, name: string, state: OpenSessionState) {
    return invoke<void>("session_report", { code, display, name, state });
  }

  clearCache() {
    return invoke<number>("clear_cache");
  }
  relayRefresh() {
    return invoke<boolean>("relay_refresh");
  }

  restartApp() {
    return invoke<void>("restart_app");
  }

  savePairing(code: string) {
    return invoke<void>("pairing_save", { code });
  }

  kickAll() {
    return invoke<void>("kick_all");
  }

  kickSession(sessionId: number) {
    return invoke<void>("kick_session", { sessionId });
  }

  accountGet() {
    return invoke<AccountInfo | null>("account_get");
  }
  accountRefresh() {
    return invoke<AccountInfo | null>("account_refresh");
  }
  accountRegister(name: string, password: string) {
    return invoke<AccountInfo>("account_register", { name, password });
  }
  accountLogin(name: string, password: string) {
    return invoke<AccountInfo>("account_login", { name, password });
  }
  accountLogout() {
    return invoke<void>("account_logout");
  }
  accountChangePassword(oldPassword: string, newPassword: string) {
    return invoke<AccountInfo>("account_change_password", { oldPassword, newPassword });
  }
  accountDevices() {
    return invoke<AccountDevice[]>("account_devices");
  }
  accountSaveDevices(devices: AccountDevice[]) {
    return invoke<void>("account_save_devices", { devices });
  }
  accountHeartbeat(code: string) {
    return invoke<void>("account_heartbeat", { code });
  }

  onAccountDevices(cb: (devices: AccountDevice[]) => void) {
    return unlistenLater(listen<{ devices: AccountDevice[] }>("account-devices", (e) => cb(e.payload.devices ?? [])));
  }

  openFilesWindow(target: { code: string; name: string }) {
    return invoke<void>("files_open", { code: target.code, name: target.name });
  }

  // 和同账号的另一台设备传文件：经账号服务的 WebSocket 转发（见 src-tauri/src/peer.rs）
  peerFsOpen(code: string) {
    return invoke<number>("peer_fs_open", { code });
  }
  peerFsSend(tid: number, data: Record<string, unknown>) {
    return invoke<void>("peer_fs_send", { tid, data });
  }
  peerFsClose(tid: number) {
    void invoke<void>("peer_fs_close", { tid }).catch(() => {});
  }
  onFsPeer(cb: (m: FsPeerMessage) => void) {
    return unlistenLater(listen<FsPeerMessage>("fs-peer", (e) => cb(e.payload)));
  }

  localRoots() {
    return invoke<{ places: FsPlace[]; home: string }>("local_fs_roots");
  }
  localList(path: string) {
    return invoke<{ path: string; parent: string | null; entries: FsEntry[] }>("local_fs_list", { path });
  }
  localExists(path: string) {
    return invoke<boolean>("local_fs_exists", { path });
  }
  localOpen(id: number, path: string, write: boolean) {
    return invoke<number>("local_fs_open", { id, path, write });
  }
  localRead(id: number, offset: number, len: number) {
    return invoke<{ data: string; eof: boolean }>("local_fs_read", { id, offset, len });
  }
  localWrite(id: number, offset: number, data: string) {
    return invoke<void>("local_fs_write", { id, offset, data });
  }
  localClose(id: number) {
    return invoke<void>("local_fs_close", { id });
  }
  controllerPublicKey() {
    return invoke<string>("controller_public_key");
  }
  trustControllers(keys: string[]) {
    return invoke<void>("trust_controllers", { keys });
  }

  profileGet() {
    return invoke<Profile>("profile_get");
  }

  profileSetAllow(allow: boolean) {
    return invoke<Profile>("profile_set_allow", { allow });
  }

  profileSetVerify(verify: "pairing" | "approval" | "open") {
    return invoke<Profile>("profile_set_verify", { verify });
  }

  favoriteToggle(code: string) {
    return invoke<Profile>("favorite_toggle", { code });
  }

  recentPush(code: string, name: string) {
    return invoke<Profile>("recent_push", { code, name });
  }

  recentDrop(code?: string) {
    return invoke<Profile>("recent_drop", { code: code ?? null });
  }

  setLanguage(lang: Lang) {
    return invoke<void>("set_language", { lang });
  }

  onLangChanged(cb: (lang: Lang) => void) {
    return unlistenLater(listen<Lang>("lang-changed", (e) => cb(e.payload)));
  }

  // 不直接用 setFullscreen：无边框窗口全屏时还要停用拖动边缘改大小（见 src-tauri/src/fullscreen.rs）
  setWindowFullscreen(on: boolean) {
    return invoke<void>("set_fullscreen", { on });
  }

  // 不用 isFullscreen：窗口全屏和网页全屏要分开处理（见 src-tauri/src/fullscreen.rs），它不知道，进全屏时窗口一变大就会被当成退出了
  onWindowFullscreenExit(cb: () => void) {
    return unlistenLater(
      getCurrentWindow().onResized(async () => {
        if (!(await invoke<boolean>("window_fullscreen"))) cb();
      }),
    );
  }
}

export function tauriBackend(): Backend {
  return new DesktopConsole();
}
