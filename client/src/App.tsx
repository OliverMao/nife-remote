import { useEffect, useRef } from "react";
import { TitleBar } from "./components/TitleBar";
import { WebUnsupported, webUnsupported } from "./components/WebUnsupported";
import { applyAccountDevices, applyFullscreen, heartbeatToAccount, hideOnClose, loadDevices, refreshAccountDevices, registerSelfWithAccount } from "./lib/actions";
import { api } from "./lib/api";
import { supportedDecoders } from "./lib/codec";
import { setLang, useLang } from "./lib/i18n";
import { markPlatform } from "./lib/platform";
import { filesTarget } from "./lib/files-window";
import { sessionTarget } from "./lib/session-window";
import { store, useStore } from "./lib/store";
import { Sidebar } from "./components/Sidebar";
import { DeviceView } from "./components/DeviceView";
import { AssistView } from "./components/AssistView";
import { HostView } from "./components/HostView";
import { LoginView } from "./components/LoginView";
import { SessionWindow } from "./components/SessionWindow";
import { FileTransfer } from "./components/FileTransfer";

markPlatform();

/** 这个窗口是不是一台电脑的远程窗口（见 lib/session-window.ts） */
const REMOTE = sessionTarget();
const FILES = filesTarget();

export function App() {
  // 语言切换时整个界面重新渲染
  useLang();
  // 网页版：不是安全页面（HTTPS / localhost）或浏览器太旧，没有视频解码可用
  const unsupported = webUnsupported();
  if (unsupported) return <WebUnsupported reason={unsupported} />;
  // 远程窗口：整个窗口就是远程画面（没有左栏、没有标题栏）
  if (FILES) return <FileTransfer target={FILES} />;
  if (REMOTE) return <SessionWindow target={REMOTE} />;
  return <Console />;
}

function Console() {
  const view = useStore((s) => s.view);
  const immersive = useStore((s) => s.immersive);
  const account = useStore((s) => s.account);
  const accountLoaded = useStore((s) => s.accountLoaded);

  /** 本机的连接记录 + 账号里的设备（"我的设备"显示的是后者） */
  const refreshDevices = () => {
    void loadDevices();
    // 先报到再拉列表：这样"我的设备"里本机（以及其它刚启动的设备）的在线状态也是最新的
    void heartbeatToAccount().then(refreshAccountDevices);
  };

  useEffect(() => {
    const unLang = api.onLangChanged(setLang);
    void api.appInfo().then((info) => {
      store.setApp(info);
      setLang(info.lang);
    });
    void api.serverStatus().then(store.setServer);
    // 首页的本机连接码：先取一次，之后跟着变化（连上服务器、换码、有人连上）
    void api.hostStatus().then((h) => {
      if (!h) return;
      store.setHost(h);
      void registerSelfWithAccount();
    });
    // 网页版没有被控端，这两个都是空的
    const unHost = api.onHostStatus((h) => {
      store.setHost(h);
      void registerSelfWithAccount();
    });
    void loadDevices();
    // 本机偏好（允许协助 / 验证方式 / 收藏 / 最近连接）
    void api.profileGet().then((p) => store.setProfile(p), () => {});
    void api.accountGet().then((a) => {
      store.setAccount(a);
      // 后台再问一下账号服务（凭证过期就自动变成没登录）
      if (a) {
        void api.accountRefresh().then((fresh) => store.setAccount(fresh), () => {});
        void refreshAccountDevices();
      }
    }, () => {});
    // 账号服务的设备连接推来的列表（名字、系统、上下线）：直接换掉"我的设备"里那份
    const unDevices = api.onAccountDevices((list) => {
      const account = store.get().account;
      if (account) store.setAccount(applyAccountDevices(account, list));
    });
    // 我的设备：每 30 秒自动刷新一次，窗口切回前台也刷新（别的地方改了配置能跟上）
    const devicesTimer = window.setInterval(() => void refreshDevices(), 30_000);
    const onFocus = () => void refreshDevices();
    window.addEventListener("focus", onFocus);
    void supportedDecoders().then(store.setDecoders);
    // 连不上中转服务器时自动再问一次账号服务：分配变了（换了地址 / 线路）就重启用新的
    let asking = false;
    const unStatus = api.onServerStatus((state) => {
      store.setServer(state);
      if (asking || state.state === "connected" || state.state === "connecting") return;
      // 只有"连不上 / 断了"这类问题才值得再问一次账号服务（配置错、证书不符问了也没用）
      if (state.state === "disconnected" && !["serverUnreachable", "serverLost"].includes(state.reason.kind)) {
        return;
      }
      if (!store.get().app) return;
      asking = true;
      void api
        .relayRefresh()
        .then((changed) => (changed ? api.restartApp() : undefined))
        .catch(() => {})
        .finally(() => {
          asking = false;
        });
    });
    const unOnline = api.onHostOnline(store.hostOnline);
    // 开着的远程窗口（左栏"会话中"、设备列表里的"已打开"）：一个控制码一个窗口，见 src-tauri/src/viewer.rs
    void api.sessionWindows().then((list) => list && store.setOpen(list), () => {});
    const unOpen = api.onSessionWindows(store.setOpen);
    // 关闭窗口（标题栏的关闭按钮、任务栏、Alt+F4）：有会话时先确认
    // 关窗口 = 只藏界面（不退出、不问），真正退出在托盘右键菜单里
    const unClose = api.frame?.onCloseRequested(hideOnClose);
    const onKey = (e: KeyboardEvent) => {
      // 屏蔽 WebView 自带的刷新/查找/打印等快捷键，避免误触
      if (e.key === "F5" || ((e.ctrlKey || e.metaKey) && ["r", "p", "f", "g", "j", "u"].includes(e.key.toLowerCase()))) {
        e.preventDefault();
      }
    };
    window.addEventListener("keydown", onKey);
    const noMenu = (e: MouseEvent) => {
      if (!(e.target instanceof HTMLInputElement)) e.preventDefault();
    };
    window.addEventListener("contextmenu", noMenu);
    // 全屏被系统退出（长按 Esc、系统快捷键……）时同步状态
    const exitImmersive = () => {
      if (store.get().immersive) store.setImmersive(false);
    };
    const onFullscreenChange = () => {
      if (!document.fullscreenElement) exitImmersive();
    };
    document.addEventListener("fullscreenchange", onFullscreenChange);
    const unWinFs = api.onWindowFullscreenExit(exitImmersive);
    return () => {
      unStatus();
      unDevices();
      window.clearInterval(devicesTimer);
      window.removeEventListener("focus", onFocus);
      unHost();
      unOnline();
      unOpen();
      unLang();
      unClose?.();
      unWinFs();
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("contextmenu", noMenu);
      document.removeEventListener("fullscreenchange", onFullscreenChange);
    };
  }, []);

  // 退出沉浸模式（包括关掉了全屏中的会话、切到了别的页面）时，窗口也退出全屏
  const wasImmersive = useRef(immersive);
  useEffect(() => {
    if (wasImmersive.current && !immersive) void applyFullscreen(false);
    wasImmersive.current = immersive;
  }, [immersive]);

  // 没登录：整个窗口就是登录页（标题栏留着，窗口还要能拖动 / 关闭）；账号状态还没读出来时什么都不画
  if (!accountLoaded) return <div className="app login-shell" />;
  if (!account) {
    return (
      <div className="app login-shell">
        <div className="workspace">
          <TitleBar />
          <LoginView />
        </div>
      </div>
    );
  }

  return (
    <div className={`app ${immersive ? "immersive" : ""}`}>
      {!immersive && <Sidebar />}
      <div className="workspace">
        {!immersive && <TitleBar />}
        <main className="main">
          {view === "host" ? <HostView /> : view === "device" ? <DeviceView /> : <AssistView />}
        </main>
      </div>
    </div>
  );
}
