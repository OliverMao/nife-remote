// 被控中页（左栏被控卡右上角的放大按钮进来，照 UU 的"本机被控中"大图）：
// 上面一条通栏横幅写着被控了多久、有几台在控，下面是正在控制本机的设备列表。
//
// "断开所有远控"是真的（走被控端的 KickAll）；单个设备的"断开远控"还没有按会话断开的接口，先禁用。
import { useEffect, useState } from "react";
import { api, type Problem } from "../lib/api";
import { useT } from "../lib/i18n";
import { problemText } from "../lib/messages";
import { store, useStore } from "../lib/store";
import { accountDevices, deviceTitle } from "../lib/devices";
import { IconMonitor, IconShrink, IconWinMark } from "./Icons";

/** 秒数 → 06:28:56 */
function hms(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(Math.floor(s / 3600))}:${p(Math.floor((s % 3600) / 60))}:${p(s % 60)}`;
}

export function HostView() {
  const t = useT();
  const host = useStore((s) => s.host);
  const [since, setSince] = useState<Record<number, number>>({});
  const [now, setNow] = useState(Date.now());
  const [error, setError] = useState<Problem | null>(null);

  // 每一路控制端是什么时候连上来的（状态里没有时间，本地记第一次看到它的时刻）
  const devices = accountDevices(useStore((s) => s.account), useStore((s) => s.devices));
  const controllers = host?.controllers ?? [];
  useEffect(() => {
    setSince((prev) => {
      const next: Record<number, number> = {};
      for (const c of controllers) next[c.sessionId] = prev[c.sessionId] ?? Date.now();
      return next;
    });
  }, [controllers.map((c) => c.sessionId).join(",")]); // eslint-disable-line react-hooks/exhaustive-deps

  // 秒表：只有被控着才需要每秒重画
  useEffect(() => {
    if (controllers.length === 0) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [controllers.length]);

  const starts = controllers.map((c) => since[c.sessionId] ?? now);
  const total = starts.length > 0 ? (now - Math.min(...starts)) / 1000 : 0;

  const kickAll = () => {
    setError(null);
    void api.kickAll().catch((e) => setError(e as Problem));
  };

  return (
    <div className="host-page">
      <div className="hv-banner">
        <button className="hv-shrink" title={t("host.backHome")} aria-label={t("host.backHome")} onClick={() => store.setView("new")}>
          <IconShrink size={16} />
        </button>
        <div className="hv-banner-text">
          <div className="hv-banner-title">
            {controllers.length > 0 ? t("host.controlling") : t("host.title")}
            {controllers.length > 0 && <span className="hv-timer">{hms(total)}</span>}
          </div>
          <div className="hv-banner-sub">
            {controllers.length > 0 ? t("host.controllersLine", { n: controllers.length }) : t("host.waiting")}
          </div>
        </div>
        <div className="hv-banner-mark" aria-hidden>
          <IconMonitor size={92} />
        </div>
        <button className="hv-kick" onClick={kickAll} disabled={controllers.length === 0}>
          {t("host.kickAll")}
        </button>
      </div>

      {error && <div className="error-box left">{problemText(error)}</div>}

      <div className="hv-list">
        <div className="hv-list-title">{t("devices.title")}</div>
        {devices.length === 0 ? (
          <div className="hv-empty">{t("devices.empty")}</div>
        ) : (
          devices.map((d) => (
            <button className="hv-item" key={d.code} onClick={() => store.showDevice(d.code)}>
              <span className="hv-item-icon soft">
                <IconWinMark size={20} />
              </span>
              <div className="hv-item-text">
                <div className="hv-item-name">{deviceTitle(d)}</div>
                <div className="hv-item-meta">
                  <span className="hv-tag">{d.quality ? t("device.connectedOnce") : t("devices.unknown")}</span>
                </div>
              </div>
              <span className="hv-item-btn">{t("device.open")}</span>
            </button>
          ))
        )}
      </div>

      <div className="hv-list">
        <div className="hv-list-title">{t("host.controllersTitle")}</div>
        {controllers.length === 0 ? (
          <div className="hv-empty">{t("host.noControllers")}</div>
        ) : (
          controllers.map((c) => (
            <div className="hv-item" key={c.sessionId}>
              <span className="hv-item-icon">
                <IconWinMark size={20} />
              </span>
              <div className="hv-item-text">
                <div className="hv-item-name">{c.peer.name || t("devices.unknown")}</div>
                <div className="hv-item-meta">
                  <span className="hv-tag">{t("host.tagRemote")}</span>
                  <span className="muted small">
                    {t("host.duration")} {hms((now - (since[c.sessionId] ?? now)) / 1000)}
                  </span>
                </div>
              </div>
              <button
                className="hv-item-btn"
                onClick={() =>
                  void api.kickSession(c.sessionId).catch((e) => setError(e as Problem))
                }
              >
                {t("host.kickOne")}
              </button>
            </div>
          ))
        )}
      </div>
    </div>
  );
}
