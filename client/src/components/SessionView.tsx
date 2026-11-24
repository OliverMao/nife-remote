import { type RefObject, useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { closeTab, toggleFullscreen } from "../lib/actions";
import { api } from "../lib/api";
import { t, useT } from "../lib/i18n";
import { errorText, noticeText, problemText, sessionStateText } from "../lib/messages";
import { CODEC_LABEL } from "../lib/presets";
import type { RemoteSession } from "../lib/session";
import { useSession, useSessionTitle, useStore } from "../lib/store";
import {
  IconMonitor,
  IconRefresh,
  IconSettings,
  IconWarn,
} from "./Icons";
import { ControlCenter } from "./ControlCenter";
import { Logo } from "./Logo";
import { fmtKbps } from "./QualityPanel";
import { WindowButtons } from "./WindowButtons";

/** 把会话的 canvas 挂到当前位置（同一时刻只会挂在一个地方） */
export function CanvasHost({ s, interactive }: { s: RemoteSession; interactive: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const el = ref.current!;
    s.attach(el);
    s.setInteractive(interactive);
    if (interactive) s.focus();
    return () => {
      s.setInteractive(false);
      s.detach();
    };
  }, [s, interactive]);
  return <div ref={ref} className={`canvas-host ${interactive && s.scaleMode === "native" ? "native" : ""}`} />;
}

/** 顶端热区的高度（和 .hot-edge 一致） */
const HOT_EDGE = 6;
/** 鼠标从这个高度以内移出网页，也算移到了顶端（网页顶端上面还有浏览器的界面时） */
const LEAVE_TOP = 48;
/**
 * 工具栏周围这一圈内不收起：跨过热区和工具栏之间的空隙。网页版全屏时鼠标碰到顶端，
 * 浏览器的工具栏可能盖住工具条，鼠标往下移出它时还在这一圈里
 */
const KEEP_NEAR = 32;

/**
 * 全屏时悬浮工具栏的显示 / 收起。不用 CSS :hover：系统界面盖住顶端时网页就不算悬停了，
 * 工具栏会马上收起。这里只在鼠标明确离开（在网页里移到了工具栏附近以外）时才收起
 */
function useToolbarReveal(active: boolean, toolbar: RefObject<HTMLElement | null>) {
  // 全屏中切换到别的连接时会重新挂载，这时鼠标还在工具栏上：先显示
  const [revealed, setRevealed] = useState(active);
  const shown = useRef(active);
  useEffect(() => {
    const set = (v: boolean) => {
      shown.current = v;
      setRevealed(v);
    };
    if (!active) {
      set(false);
      return;
    }
    const near = (x: number, y: number) => {
      const r = toolbar.current?.getBoundingClientRect();
      return !!r && x >= r.left - KEEP_NEAR && x <= r.right + KEEP_NEAR && y <= r.bottom + KEEP_NEAR;
    };
    const onPointer = (e: PointerEvent) => {
      // 按着键拖动时（例如把远程的窗口拖到顶端）不弹出；触摸屏点一下顶端也能弹出
      if (e.clientY <= HOT_EDGE && (e.buttons === 0 || e.type === "pointerdown")) set(true);
      else if (shown.current && !near(e.clientX, e.clientY)) set(false);
    };
    const onLeave = (e: MouseEvent) => {
      if (e.relatedTarget) return; // 还在网页里
      set(e.clientY <= LEAVE_TOP && e.buttons === 0);
    };
    window.addEventListener("pointermove", onPointer);
    window.addEventListener("pointerdown", onPointer);
    document.addEventListener("mouseout", onLeave);
    return () => {
      window.removeEventListener("pointermove", onPointer);
      window.removeEventListener("pointerdown", onPointer);
      document.removeEventListener("mouseout", onLeave);
    };
  }, [active, toolbar]);
  return revealed;
}

export function SessionView({ s, pinnedDisplay = null }: { s: RemoteSession; pinnedDisplay?: number | null }) {
  // 订阅语言切换（文案直接用导入的 t）
  useT();
  useSession(s);
  const immersive = useStore((st) => st.immersive);
  const title = useSessionTitle(s);
  // 控制中心（画质、声音、剪贴板、鼠标、全屏、断开……）开着没有
  const [center, setCenter] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  // 对方要求配对码、或者对方的身份变了（要重新配对）时输入的配对码
  const [pairing, setPairing] = useState("");
  const needsPairing =
    s.state === "closed" &&
    (s.closedReason?.kind === "pairingRequired" ||
      s.closedReason?.kind === "pairingFailed" ||
      s.closedReason?.kind === "pairingUnavailable" ||
      s.closedReason?.kind === "hostKeyChanged");
  const toolbarRef = useRef<HTMLElement>(null);
  const revealed = useToolbarReveal(immersive, toolbarRef);
  // 控制中心关掉后键盘回到远程画面（例如按 Esc 关掉后接着打字）
  const closeCenter = useCallback(() => {
    setCenter(false);
    s.focus();
  }, [s]);

  // 显示器标签：拖到窗口外 = 那块屏幕单独开一个窗口（见 popOut）
  const pull = useRef<{ index: number; x: number; y: number; moved: boolean } | null>(null);
  const skipClick = useRef(false);
  const [pulled, setPulled] = useState<{ index: number; x: number; y: number } | null>(null);
  /** 指针是不是已经在工具栏（标题栏）外面了 */
  const outside = (x: number, y: number) => {
    const r = toolbarRef.current?.getBoundingClientRect();
    return !!r && (y < r.top - 2 || y > r.bottom + 2 || x < r.left - 2 || x > r.right + 2);
  };

  /** 这一块屏幕已经在另一个窗口里看（控制器左栏"会话中"那份列表） */
  const openWindows = useStore((st) => st.open);
  const openedElsewhere = (index: number) => openWindows.some((w) => w.code === s.code && w.display === index);

  /**
   * 这个窗口还要列出哪几块屏：一块屏只属于一个窗口。
   * 已经单独开出去的不在这里重复出现（否则拖出去以后两个窗口都列着它）；
   * 这个窗口自己固定看某块屏时，那就是它的全部，只剩一个也就没有标签栏了。
   */
  const tabs =
    pinnedDisplay !== null ? s.displays.filter((d) => d.index === pinnedDisplay) : s.displays.filter((d) => !openedElsewhere(d.index));

  /** 把一块屏幕拖成一个新窗口：画面是一路流，所以那是另一个会话（同一个控制码，固定看这个显示器） */
  const popOut = (index: number) => {
    void api
      .openSessionWindow({ code: s.code, display: index, settings: s.settings, pairing: null })
      .catch((e) => setToast(errorText(e)));
    // 同一个画面不连两次：这个窗口原本就在看它，就换到另一块屏幕
    if (s.currentDisplay === index) {
      const other = s.displays.find((d) => d.index !== index);
      if (other) s.selectDisplay(other.index);
    }
  };

  useEffect(() => {
    if (!toast) return;
    const t = window.setTimeout(() => setToast(null), 2400);
    return () => clearTimeout(t);
  }, [toast]);

  // 第一次锁定鼠标时（包括被控端隐藏光标时自动锁定、按快捷键）提示怎么解除：
  // 全屏时 Esc 发给了对方，指针也碰不到顶端叫出工具栏
  const pointerLocked = s.pointerLocked;
  const lockHinted = useRef(false);
  useEffect(() => {
    if (!pointerLocked || lockHinted.current) return;
    lockHinted.current = true;
    setToast(t("session.mouseLocked"));
  }, [pointerLocked]);

  const fullscreen = async () => {
    setCenter(false);
    await toggleFullscreen();
    s.focus();
  };

  const st = s.stats;
  const live = s.state === "live";
  const rtt = st.rttMs != null ? Math.round(st.rttMs) : null;

  /**
   * 连上了但一直没有画面（对方一个帧都没编出来）：多半是被控端采不到画面（锁屏、没有显示器、
   * 采集失败）。黑屏不给任何说明的话，只能靠猜是网络还是对方，所以这里主动说出来。
   */
  const [noPicture, setNoPicture] = useState(false);
  useEffect(() => {
    if (!live || st.width > 0) {
      setNoPicture(false);
      return;
    }
    const timer = window.setTimeout(() => setNoPicture(true), 4000);
    return () => window.clearTimeout(timer);
  }, [live, st.width]);
  const rttClass = rtt == null ? "" : rtt < 60 ? "good" : rtt < 150 ? "ok" : "bad";

  return (
    <div className={`session-view ${immersive ? "immersive" : ""}`}>
      {immersive && <div className="hot-edge" />}
      {/* 这一条就是窗口的标题栏（无边框，拖动、双击最大化由 Tauri 按 data-tauri-drag-region 处理） */}
      <header ref={toolbarRef} className={`toolbar ${revealed ? "revealed" : ""}`} data-tauri-drag-region="deep">
        {/* 左边：图标 + 对方名称 + 控制码（和图示一样，这些都在标题栏上） */}
        <div className="tb-brand">
          <Logo size={18} />
          <div className="addr-title">
            <span className={`dot ${s.state}`} />
            <span
              className="host"
              title={s.safety ? `${title} · ${t("session.safety", { code: s.safety })}` : title}
            >
              {title}
            </span>
          </div>
          {/* 界面上不显示控制码（只在"远程协助"页输入）；安全码挪到设备名上悬停看 */}

        </div>

        {/* 显示器：像标签一样切换；拖到窗口外松手，这块屏幕就单独开一个窗口（见 popOut） */}
        {tabs.length > 1 && (
          <div className="disp-tabs" role="tablist" aria-label={t("session.switchDisplay")}>
            {tabs.map((d) => (
              <button
                key={d.index}
                role="tab"
                aria-selected={d.index === s.currentDisplay}
                className={`disp-tab ${d.index === s.currentDisplay ? "active" : ""}`}
                title={[d.name, `${d.width}×${d.height}`, d.primary ? t("session.primary") : ""]
                  .filter(Boolean)
                  .join(" · ")}
                onPointerDown={(e) => {
                  if (e.button !== 0) return;
                  pull.current = { index: d.index, x: e.clientX, y: e.clientY, moved: false };
                  e.currentTarget.setPointerCapture(e.pointerId);
                }}
                onPointerMove={(e) => {
                  const p = pull.current;
                  if (!p || p.index !== d.index) return;
                  if (!p.moved && Math.hypot(e.clientX - p.x, e.clientY - p.y) > 6) p.moved = true;
                  const out = p.moved && outside(e.clientX, e.clientY);
                  setPulled(out ? { index: d.index, x: e.clientX, y: e.clientY } : null);
                }}
                onPointerUp={(e) => {
                  const p = pull.current;
                  pull.current = null;
                  setPulled(null);
                  if (!p || p.index !== d.index) return;
                  if (!p.moved) return; // 没拖动：交给 onClick 切换
                  // 拖动之后也会有一次 click：这一下不算切换（下一轮再恢复）
                  skipClick.current = true;
                  window.setTimeout(() => (skipClick.current = false), 0);
                  if (outside(e.clientX, e.clientY)) popOut(d.index);
                  else {
                    // 拖回栏内松手：当作点了一下
                    s.selectDisplay(d.index);
                    s.focus();
                  }
                }}
                onPointerCancel={() => {
                  pull.current = null;
                  setPulled(null);
                }}
                onClick={() => {
                  // 拖出去之后松手也会有一次 click：那一下不算切换
                  if (skipClick.current) {
                    skipClick.current = false;
                    return;
                  }
                  s.selectDisplay(d.index);
                  s.focus();
                }}
              >
                <IconMonitor size={14} />
                <span>{t("session.display", { n: d.index + 1 })}</span>
                {d.name && <span className="disp-name">({d.name})</span>}
              </button>
            ))}
          </div>
        )}

        <div className="addr-stats">
            {live ? (
              <>
                <span>{Math.round(st.fps)} fps</span>
                <span>{fmtKbps(st.kbps)}</span>
                <span className={`rtt ${rttClass}`}>{rtt != null ? `${rtt} ms` : "— ms"}</span>
                {st.width > 0 && (
                  <span className="muted" title={t("session.pathTitle")}>
                    {st.direct ? t("session.direct") : t("session.relayed")}
                  </span>
                )}
                {s.syncActive && s.syncStats.videoDelayMs >= 5 && (
                  <span className="muted" title={t("session.syncTitle")}>
                    {t("session.sync", { ms: Math.round(s.syncStats.videoDelayMs) })}
                  </span>
                )}
                {st.width > 0 && (
                  <span className="muted stat-res">
                    {st.width}×{st.height}
                  </span>
                )}
                {s.encoder && (
                  <span className="muted stat-codec" title={s.encoder.name}>
                    {CODEC_LABEL[s.encoder.codec]} ·{" "}
                    {s.encoder.hardware ? t("session.hwEncoding") : t("session.swEncoding")}
                  </span>
                )}
              </>
            ) : (
              <span className="muted">{sessionStateText(s)}</span>
            )}
          </div>

        {/* 右边只有"控制中心"：麦克风、声音、画质这些都在里面，标题栏不摆开关 */}
        <div className="tb-right">
          <div className="menu-anchor">
            <button
              className={`tb-btn ${center ? "on" : ""}`}
              title={t("session.controlCenter")}
              aria-expanded={center}
              onClick={() => setCenter((v) => !v)}
            >
              <IconSettings size={17} />
              <span>{t("session.controlCenter")}</span>
            </button>
            {center && <ControlCenter s={s} onClose={closeCenter} onToast={setToast} onFullscreen={fullscreen} />}
          </div>
        </div>
        {/* 无边框窗口：最小化 / 最大化 / 关闭由界面画（全屏时浮动的工具栏里不摆窗口按钮） */}
        {!immersive && <WindowButtons onClose={() => closeTab(s.id)} />}
      </header>

      <div className="stage">
        <CanvasHost s={s} interactive={s.state === "live"} />

        {/* 连上了但对方没送画面：说清楚，别让人对着黑屏猜 */}
        {noPicture && (
          <div className="stage-note" role="status">
            <IconWarn size={16} />
            <div>
              <div>{t("session.noPicture")}</div>
              <div className="muted small">{t("session.noPictureHint")}</div>
            </div>
          </div>
        )}

        {/* 把显示器标签拖出窗口时跟着鼠标的提示 */}
        {pulled && (
          <div className="pull-hint" style={{ left: pulled.x, top: pulled.y }}>
            <IconMonitor size={15} />
            {t("session.pullOutHint", { n: pulled.index + 1 })}
          </div>
        )}

        {s.state !== "live" && (
          <div className="stage-overlay">
            {s.state === "closed" ? (
              <div className="overlay-card">
                <IconWarn size={28} />
                {/* 第一次连要配对码：只摆一个"配对码"输入框（表单式的表达，不写状态说明） */}
                {s.closedReason?.kind !== "pairingRequired" && (
                  <>
                    <div className="overlay-title">
                      {s.retries > 0 ? t("session.reconnectFailed") : t("session.lost")}
                    </div>
                    <div className="muted">{s.closedReason ? problemText(s.closedReason) : t("session.ended")}</div>
                  </>
                )}
                {s.closedReason?.kind === "pairingRequired" && (
                  <div className="overlay-title">{t("connect.pairing")}</div>
                )}
                {needsPairing && (
                  <input
                    className="pairing-input"
                    value={pairing}
                    inputMode="numeric"
                    autoComplete="off"
                    placeholder="000 000"
                    aria-label={t("connect.pairing")}
                    autoFocus
                    onChange={(e) => setPairing(e.target.value.replace(/\D/g, "").slice(0, 6))}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" && pairing.length === 6) s.pairWith(pairing);
                    }}
                  />
                )}
                <div className="overlay-actions">
                  {needsPairing ? (
                    <button className="primary-btn small" disabled={pairing.length !== 6} onClick={() => s.pairWith(pairing)}>
                      <IconRefresh size={16} />
                      {t("connect.go")}
                    </button>
                  ) : (
                    <button className="primary-btn small" onClick={() => s.reconnect()}>
                      <IconRefresh size={16} />
                      {t("session.reconnect")}
                    </button>
                  )}
                  <button className="ghost-btn" onClick={() => closeTab(s.id)}>
                    {t("session.close")}
                  </button>
                </div>
              </div>
            ) : s.state === "approving" ? (
              <div className="overlay-card">
                <span className="spinner large" />
                <div className="overlay-title">{t("session.approving", { name: title })}</div>
                <div className="muted small">{t("session.approvingHint")}</div>
                {s.safety && (
                  <div className="muted small" title={t("session.safetyHint")}>
                    {t("session.safety", { code: s.safety })}
                  </div>
                )}
                <div className="overlay-actions">
                  <button className="ghost-btn" onClick={() => closeTab(s.id)}>
                    {t("session.close")}
                  </button>
                </div>
              </div>
            ) : s.state === "waiting" ? (
              <div className="overlay-card">
                <span className="spinner large" />
                <div className="overlay-title">{t("session.waiting")}</div>
                <div className="muted small">{t("session.waitingHint")}</div>
                <div className="overlay-actions">
                  <button className="ghost-btn" onClick={() => closeTab(s.id)}>
                    {t("session.close")}
                  </button>
                </div>
              </div>
            ) : (
              <div className="overlay-card">
                <span className="spinner large" />
                <div className="overlay-title">
                  {s.state === "reconnecting" ? t("session.reconnecting") : t("session.connecting")}
                </div>
                {s.closedReason && <div className="muted small">{problemText(s.closedReason)}</div>}
                {s.retries > 0 && <div className="muted small">{t("session.attempt", { n: s.retries })}</div>}
              </div>
            )}
          </div>
        )}

        {s.notice && (
          <div className="notice" onClick={() => s.dismissNotice()}>
            <IconWarn size={16} />
            <span>{noticeText(s.notice)}</span>
          </div>
        )}
        {toast && <div className="toast">{toast}</div>}
      </div>
    </div>
  );
}

