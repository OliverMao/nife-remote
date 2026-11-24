// 控制中心：会话工具栏上那一堆按钮（画质、声音、剪贴板、麦克风、鼠标、画面大小、全屏、断开……）
// 都收进这一个弹层里，标题栏上只留麦克风和它自己（参考常见的远程桌面：`控制中心` + 窗口按钮）。
import { useState } from "react";
import { api } from "../lib/api";
import { closeTab } from "../lib/actions";
import { t, useT, type MsgKey } from "../lib/i18n";
import { errorText } from "../lib/messages";
import { matchPreset, presetName } from "../lib/presets";
import type { RemoteSession } from "../lib/session";
import { useSession, useStore } from "../lib/store";
import { IconChevron, IconExpand, IconPower, IconShrink } from "./Icons";
import { Popover } from "./Popover";
import { QualityPanelBody } from "./QualityPanel";
import { openFileTransfer } from "./FileTransfer";

/** 浏览器 / 网页版里发不了的组合键（系统操作在手机上有单独的一组，见 s.hostActions） */
type Combo = { label: MsgKey; keys: string[] };

const WINDOWS_COMBOS: Combo[] = [
  { label: "combo.winStart", keys: ["MetaLeft"] },
  { label: "combo.altTab", keys: ["AltLeft", "Tab"] },
  { label: "combo.winD", keys: ["MetaLeft", "KeyD"] },
  { label: "combo.winE", keys: ["MetaLeft", "KeyE"] },
  { label: "combo.winR", keys: ["MetaLeft", "KeyR"] },
  { label: "combo.taskManager", keys: ["ControlLeft", "ShiftLeft", "Escape"] },
  { label: "combo.altF4", keys: ["AltLeft", "F4"] },
  { label: "combo.printScreen", keys: ["PrintScreen"] },
];

type Page = "root" | "quality" | "keys";

/**
 * 控制中心。`onClose`：收起弹层；`onToast`：在画面上方的提示条里说一句话；
 * `onFullscreen`：进出全屏（需要会话工具栏那边配合：切完还要把键盘交回画面）。
 */
export function ControlCenter({
  s,
  onClose,
  onToast,
  onFullscreen,
}: {
  s: RemoteSession;
  onClose: () => void;
  onToast: (msg: string) => void;
  onFullscreen: () => void;
}) {
  const t = useT();
  useSession(s);
  const immersive = useStore((st) => st.immersive);
  const [page, setPage] = useState<Page>("root");

  /** 把本机剪贴板当键盘输入打到对方（登录框之类不让粘贴的地方） */
  const typeClipboard = async () => {
    try {
      const text = await api.localClipboardText();
      if (!text) return onToast(t("session.noClipboard"));
      s.sendText(text);
      onToast(t("session.typed", { n: text.length }));
    } catch (e) {
      onToast(errorText(e));
    }
    s.focus();
  };

  const pick = (run: () => void) => {
    onClose();
    run();
  };

  const preset = matchPreset(s.settings);
  /** 对方也支持文件传输（双方都有这一位能力）：老版本被控端没有 */
  const hasFiles = s.features.includes("files");

  return (
    <Popover className={`control-center ${page === "root" ? "" : "wide"}`} onClose={onClose}>
      {page === "quality" ? (
        <QualityPanelBody s={s} onBack={() => setPage("root")} />
      ) : page === "keys" ? (
        <>
          <div className="cc-head pop-title">
            <button className="pop-back" onClick={() => setPage("root")} aria-label={t("common.back")} title={t("common.back")}>
              <IconChevron size={15} />
            </button>
            {t("session.shortcuts")}
          </div>
          <div className="cc-list">
            {s.hostActions.length > 0
              ? // 手机：系统操作（返回、主页……），对方列出了哪些就显示哪些
                s.hostActions.map((a) => (
                  <button key={a} className="cc-row" onClick={() => pick(() => s.sendSystem(a))}>
                    {t(`action.${a}` as const)}
                  </button>
                ))
              : WINDOWS_COMBOS.map((c) => (
                  <button key={c.label} className="cc-row" onClick={() => pick(() => s.sendCombo(c.keys))}>
                    {t(c.label)}
                  </button>
                ))}
          </div>
        </>
      ) : (
        <>
          <div className="cc-head pop-title">{t("session.controlCenter")}</div>
          <div className="cc-list">
            {/* 画质：另一页，放下面板里的全部设置 */}
            <button className="cc-row" onClick={() => setPage("quality")}>
              <span className="cc-label">{t("session.quality")}</span>
              <span className="cc-value">
                {preset ? presetName(preset) : t("connect.customQuality")}
                <IconChevron size={14} />
              </span>
            </button>

            <label className="cc-row">
              <span className="cc-label">
                {t("session.playSound")}
                {audioHint(s) && <span className="muted small block">{audioHint(s)}</span>}
              </span>
              <input
                type="checkbox"
                className="switch"
                checked={s.audioEnabled}
                onChange={(e) => s.setAudioEnabled(e.target.checked)}
              />
            </label>
            <label className={`cc-row ${s.audioEnabled ? "" : "disabled"}`}>
              <span className="cc-label">
                {t("session.avSync")}
                {/* 只有真的在测量时才有话说：没开声音时不用解释"为什么要开声音" */}
                {s.audioEnabled && syncHint(s) && <span className="muted small block">{syncHint(s)}</span>}
              </span>
              <input
                type="checkbox"
                className="switch"
                checked={s.avSync}
                disabled={!s.audioEnabled}
                onChange={(e) => s.setAvSync(e.target.checked)}
              />
            </label>

            <label className="cc-row">
              <span className="cc-label">{t("session.clipSync")}</span>
              <input
                type="checkbox"
                className="switch"
                checked={s.clipboardSync}
                onChange={(e) => s.setClipboardSync(e.target.checked)}
              />
            </label>
            <button className="cc-row" onClick={() => pick(() => void typeClipboard())}>
              <span className="cc-label">
                {t("session.clipType")}
                <span className="muted small block">{t("session.clipTypeHint")}</span>
              </span>
            </button>

            {/* 麦克风：对方能接（有虚拟麦克风）时才有 */}
            {s.micState !== null && s.micState !== "unsupported" && (
              <label className="cc-row" title={s.micState === "noDevice" ? t("mic.noDeviceTip") : undefined}>
                <span className="cc-label">
                  {t("mic.send")}
                  {micHint(s) && <span className="muted small block">{micHint(s)}</span>}
                </span>
                <input
                  type="checkbox"
                  className="switch"
                  checked={s.micEnabled}
                  onChange={(e) => s.setMicEnabled(e.target.checked)}
                />
              </label>
            )}

            <label className="cc-row">
              <span className="cc-label">
                {s.mouseLocked ? t("session.mouseUnlock") : t("session.mouseLock")}
                <span className="muted small block">{t("session.mouseLockHint")}</span>
              </span>
              <input
                type="checkbox"
                className="switch"
                checked={s.mouseLocked}
                onChange={() => {
                  s.toggleMouseLock();
                  if (s.mouseLocked) onToast(t("session.mouseLocked"));
                }}
              />
            </label>

            {/* 画面怎么显示：适应窗口 / 原始大小（1:1） */}
            <div className="cc-row">
              <span className="cc-label">{t("session.scale")}</span>
              <div className="segmented">
                <button className={s.scaleMode === "fit" ? "active" : ""} onClick={() => s.setScaleMode("fit")}>
                  {t("session.fit")}
                </button>
                <button className={s.scaleMode === "native" ? "active" : ""} onClick={() => s.setScaleMode("native")}>
                  {t("session.native")}
                </button>
              </div>
            </div>

            {/* 文件传输：另开一个窗口（左边本机、右边对方），见 components/FileTransfer.tsx。
                对方版本不支持时这一行照样显示，只是不能点（省得"为什么别人有这个按钮"） */}
            <button
              className={`cc-row ${hasFiles ? "" : "disabled"}`}
              disabled={!hasFiles}
              title={hasFiles ? undefined : t("files.unsupported")}
              onClick={() =>
                pick(() => void openFileTransfer(s.code, s.hostName).catch((e: unknown) => onToast(errorText(e))))
              }
            >
              <span className="cc-label">
                {t("files.title")}
                {!hasFiles && <span className="muted small block">{t("files.unsupported")}</span>}
              </span>
              <span className="cc-value">
                <IconChevron size={14} />
              </span>
            </button>

            <button className="cc-row" onClick={() => setPage("keys")}>
              <span className="cc-label">{t("session.shortcuts")}</span>
              <span className="cc-value">
                <IconChevron size={14} />
              </span>
            </button>

            <button className="cc-row" onClick={() => pick(onFullscreen)}>
              <span className="cc-label">{immersive ? t("session.exitFullscreen") : t("session.fullscreen")}</span>
              <span className="cc-value">{immersive ? <IconShrink size={16} /> : <IconExpand size={16} />}</span>
            </button>
          </div>

          <div className="cc-foot">
            <button className="cc-row danger" onClick={() => closeTab(s.id)}>
              <IconPower size={16} />
              {t("session.disconnect")}
            </button>
          </div>
        </>
      )}
    </Popover>
  );
}

// ───────────────────────── 说明文字 ─────────────────────────

function audioHint(s: RemoteSession): string {
  // 关着就什么都不写：一行"播放远程声音"已经够说明了
  if (!s.audioEnabled) return "";
  if (s.audioError !== null) return t("notice.audioFailed", { detail: s.audioError });
  const st = s.audio.stats;
  return st ? t("audio.jitter", { ms: Math.round(st.targetMs) }) : t("audio.remote");
}

/** 麦克风打开了、但声音没发过去（本机采集不了，或者对方现在接不了） */
function micHint(s: RemoteSession): string {
  if (s.micError !== null) return t(`mic.local.${s.micError}` as const);
  switch (s.micState) {
    case "noDevice":
      return t("mic.noDevice");
    case "busy":
      return t("mic.busy");
    case "failed":
      return t("mic.failed", { detail: s.micDetail ?? "—" });
    default:
      // 没打开时不用解释怎么用：打开之后才有意义
      if (!s.micEnabled) return "";
      return s.micLive ? t("mic.live") : t("mic.ready");
  }
}

function syncHint(s: RemoteSession): string {
  if (!s.audioEnabled) return "";
  const { skewMs, videoDelayMs, extraMs } = s.syncStats;
  if (skewMs === null) return t("sync.waiting");
  const skew = Math.round(skewMs);
  if (!s.avSync) return skew >= 0 ? t("sync.audioLate", { ms: skew }) : t("sync.videoLate", { ms: -skew });
  const before = skew >= 0 ? t("sync.wasAudioLate", { ms: skew }) : t("sync.wasVideoLate", { ms: -skew });
  if (extraMs > 0) return t("sync.audioDelayed", { ms: Math.round(extraMs), before });
  return t("sync.videoDelayed", { ms: Math.round(videoDelayMs), before });
}
