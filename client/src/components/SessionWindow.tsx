// 一台电脑的远程窗口：整个窗口就是远程画面 + 会话工具栏（没有左栏、没有管理界面的标签）。
//
// 会话就在这个窗口里建立（走 console.ts 的 WebSocket）：关掉窗口 = 断开这一路。
// 管理界面（控制台）只负责打开 / 切过去，见 lib/actions.ts 的 openSession。
import { useEffect, useRef, useState } from "react";
import { api, type Codec, type OpenSessionState, type Problem } from "../lib/api";
import { applyFullscreen, rememberQuality, rememberSession } from "../lib/actions";
import { supportedDecoders } from "../lib/codec";
import { setLang, t, useLang } from "../lib/i18n";
import { RemoteSession } from "../lib/session";
import type { SessionTarget } from "../lib/session-window";
import { store, useStore } from "../lib/store";
import { SessionView } from "./SessionView";

export function SessionWindow({ target }: { target: SessionTarget }) {
  const [session, setSession] = useState<RemoteSession | null>(null);
  const immersive = useStore((s) => s.immersive);
  // 语言切换时重新渲染（窗口标题里的“显示 N”跟着变）
  const lang = useLang();

  useEffect(() => {
    let live = true;
    let created: RemoteSession | null = null;
    let off: (() => void) | null = null;
    const unLang = api.onLangChanged(setLang);
    // 中转服务器断了 / 对方重新上线：会话据此重连（见 session.ts 的 serverConnected / hostOnline）
    void api.serverStatus().then(store.setServer, () => {});
    const unStatus = api.onServerStatus(store.setServer);
    const unOnline = api.onHostOnline(store.hostOnline);
    // 开着的远程窗口：显示器标签上标出哪个显示器已经在另一个窗口里了
    void api.sessionWindows().then((list) => list && store.setOpen(list), () => {});
    const unOpen = api.onSessionWindows(store.setOpen);

    void (async () => {
      // 语言、能解码的格式、"我的设备"（对方的名字、重连凭证、对方的公钥）
      const [info, decoders, devices] = await Promise.all([
        api.appInfo().catch(() => null),
        supportedDecoders().catch(() => ["h264"] as Codec[]),
        api.loadDevices().catch(() => null),
      ]);
      if (info) {
        store.setApp(info);
        setLang(info.lang);
      }
      store.setDecoders(decoders);
      store.setDevices(devices ?? [], devices !== null);
      if (!live) return;
      const saved = devices?.find((d) => d.code === target.code);
      const s = new RemoteSession(target.code, target.settings, saved, target.pairing);
      created = s;
      // 从显示器标签拖出来的窗口：连上之后切到这个显示器
      s.pinDisplay(target.display);
      // 连上之后记进"我的设备"（名字、重连凭证、画质）；之后每次重连、改画质也更新
      s.onConnected = rememberSession;
      s.onSettingsChange = rememberQuality;
      store.add(s);
      setSession(s);
      // 报给控制台：左栏"会话中"那一行的名字和圆点。状态每秒都在变（统计），只在样子变了才发
      let last = "";
      const report = () => {
        const now = `${s.state}\u0000${s.hostName}`;
        if (now === last) return;
        last = now;
        void api.reportSession(target.code, target.display, s.hostName, s.state as OpenSessionState).catch(() => {});
      };
      off = s.subscribe(report);
      report();
      // 连不上（对方要配对码、控制码不对……）：会话自己在画面上说明原因，并等用户重连 / 输配对码
      await s.open().catch((e) => s.fail(e as Problem));
    })();

    return () => {
      live = false;
      off?.();
      unLang();
      unStatus();
      unOnline();
      unOpen();
      created?.dispose();
    };
  }, [target]);

  // 窗口标题（任务栏里看得到）：对方的设备名，固定看某个显示器时带上显示器号
  useEffect(() => {
    if (!session) return;
    const update = () => {
      const name = session.hostName || t("session.remoteComputer");
      const title = target.display === null ? name : `${name} · ${t("session.display", { n: target.display + 1 })}`;
      document.title = title;
      void api.frame?.setTitle(title).catch(() => {});
    };
    update();
    const off = session.subscribe(update);
    return () => void off();
  }, [session, target.display, lang]);

  // 右键菜单由远程画面对付（发给对方），不弹 WebView 自己的
  useEffect(() => {
    const noMenu = (e: MouseEvent) => {
      if (!(e.target instanceof HTMLInputElement)) e.preventDefault();
    };
    window.addEventListener("contextmenu", noMenu);
    return () => window.removeEventListener("contextmenu", noMenu);
  }, []);

  // 全屏被系统退出（长按 Esc、系统快捷键……）时跟着退出（同 App.tsx 的管理界面）
  useEffect(() => {
    const exit = () => {
      if (store.get().immersive) store.setImmersive(false);
    };
    const onFullscreenChange = () => {
      if (!document.fullscreenElement) exit();
    };
    document.addEventListener("fullscreenchange", onFullscreenChange);
    const unWinFs = api.onWindowFullscreenExit(exit);
    return () => {
      unWinFs();
      document.removeEventListener("fullscreenchange", onFullscreenChange);
    };
  }, []);

  // 退出沉浸模式时窗口也退出全屏（见 actions.ts 的 applyFullscreen）
  const wasImmersive = useRef(immersive);
  useEffect(() => {
    if (wasImmersive.current && !immersive) void applyFullscreen(false);
    wasImmersive.current = immersive;
  }, [immersive]);

  return (
    <div className={`app session-window ${immersive ? "immersive" : ""}`}>
      <div className="workspace">
        <main className="main">
          {session ? (
            <SessionView key={session.id} s={session} pinnedDisplay={target.display} />
          ) : (
            <div className="session-loading">
              <span className="spinner large" />
            </div>
          )}
        </main>
      </div>
    </div>
  );
}
