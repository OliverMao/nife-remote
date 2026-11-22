// 标题栏右侧的齿轮按钮（SettingsButton）：网络状态、本机名称、语言、画质、快捷键说明都在这个弹层里。
//
// 弹层挂在按钮下面（以前挂在最左边的 Logo 上，和窗口按钮挤在一起）。
import { useCallback, useState, type ReactNode } from "react";
import { api } from "../lib/api";
import { focusActive } from "../lib/actions";
import { setLang, useLang, useT, type Lang } from "../lib/i18n";
import { problemText } from "../lib/messages";
import { serverStatus, store, useStore } from "../lib/store";
import { IconChevron, IconKeyboard, IconMenuLines, IconSettings, IconWarn } from "./Icons";
import { Logo } from "./Logo";
import { PRESETS, presetDesc, presetName } from "../lib/presets";
import { Popover } from "./Popover";

/** 弹出层的开关：关掉后键盘回到远程画面 */
function useMenu() {
  const [open, setOpen] = useState(false);
  const close = useCallback(() => {
    setOpen(false);
    focusActive();
  }, []);
  const toggle = useCallback(() => setOpen((o) => !o), []);
  return { open, close, toggle };
}

// ───────────────────────── 设置 ─────────────────────────

export function SettingsButton({ label, burger }: { label?: string; burger?: boolean }) {
  const t = useT();
  const { open, close, toggle } = useMenu();
  return (
    <div className="menu-anchor">
      {label ? (
        <button className={`sb-item ${open ? "on" : ""}`} aria-expanded={open} onClick={toggle}>
          <IconSettings size={17} />
          <span>{label}</span>
        </button>
      ) : (
        <button
          className={`ta-icon ${open ? "on" : ""}`}
          title={t("menu.title")}
          aria-label={t("menu.title")}
          aria-expanded={open}
          onClick={toggle}
        >
          {burger ? <IconMenuLines size={17} /> : <IconSettings size={17} />}
        </button>
      )}
      {open && <AppMenu onClose={close} />}
    </div>
  );
}

function AppMenu({ onClose }: { onClose: () => void }) {
  const t = useT();
  const app = useStore((s) => s.app);
  const web = api.kind === "web";
  const [keys, setKeys] = useState(false);

  return (
    <Popover className="app-menu" onClose={onClose}>
      <div className="am-head">
        <Logo size={38} />
        <div className="am-head-text">
          <div className="am-name">Nife Remote</div>
          <div className="muted small">{t(web ? "menu.versionWeb" : "menu.version", { version: app?.version ?? "—" })}</div>
        </div>
      </div>

      <div className="am-group">
        <ServerRow />
        <div className="am-row">
          <span className="am-label">{t("menu.name")}</span>
          <span className="am-value" title={web ? t("menu.nameHintWeb") : (app?.hostname ?? undefined)}>
            {app?.hostname || t("common.thisComputer")}
          </span>
        </div>
        <div className="am-row">
          <span className="am-label">{t("menu.language")}</span>
          <LangSwitch />
        </div>
        {/* 画质：新连接用它，正在连的会话在会话工具栏 / 设备页里单独改 */}
        <QualitySwitch />
      </div>

      {/* 网页版没有被控端，浏览器也会拦下这些快捷键 */}
      {!web && (
        <div className="am-actions">
          <button className={`am-item ${keys ? "open" : ""}`} aria-expanded={keys} onClick={() => setKeys((k) => !k)}>
            <span className="am-item-icon">
              <IconKeyboard size={16} />
            </span>
            <span className="am-item-text">
              {t("menu.shortcuts")}
              <span className="muted small block">{t("menu.shortcutsHint")}</span>
            </span>
            <IconChevron size={15} className="am-chevron" />
          </button>
          {keys && (
            <div className="am-keys">
              <Keys keys={["Ctrl", "T"]}>{t("shortcut.newTab")}</Keys>
              <Keys keys={["Ctrl", "W"]}>{t("shortcut.closeTab")}</Keys>
              <Keys keys={["Ctrl", "Tab"]}>{t("shortcut.nextTab")}</Keys>
              <Keys keys={["Ctrl", "1~9"]}>{t("shortcut.nthTab")}</Keys>
              <Keys keys={["Ctrl", "Shift", "T"]}>{t("shortcut.reopen")}</Keys>
            </div>
          )}
        </div>
      )}
    </Popover>
  );
}

/** 画质：省流 / 均衡 / 流畅 / 高清 / 原画（记在 localStorage 里，跟着这台电脑）。
 *  上面一行写的是**当前选中**那个预设的说明，选哪个就显示哪个。 */
function QualitySwitch() {
  const t = useT();
  const quality = useStore((s) => s.quality);
  return (
    <div className="am-row am-stack" title={t("menu.qualityHint")}>
      <div className="am-row-line">
        <span className="am-label">{t("menu.quality")}</span>
        <span className="muted small">{presetDesc(quality)}</span>
      </div>
      <div className="segmented">
        {PRESETS.map((p) => (
          <button
            key={p.id}
            className={quality === p.id ? "active" : ""}
            title={presetDesc(p.id)}
            onClick={() => store.setQuality(p.id)}
          >
            {presetName(p.id)}
          </button>
        ))}
      </div>
    </div>
  );
}

/** 菜单里的"网络状态"：在线 / 连接中 / 离线，点一下在**本菜单里**展开详情（不另开弹窗） */
function ServerRow() {
  const t = useT();
  const server = useStore((s) => s.server);
  const status = useStore(serverStatus);
  const [open, setOpen] = useState(false);
  return (
    <div className="am-net">
      <button className="am-row am-btn" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        <span className="am-label">{t("menu.network")}</span>
        <span className={`am-value srv ${status} ${server.state === "connecting" ? "trying" : ""}`}>
          <span className="dot" />
          {t(`server.short.${status}`)}
          <IconChevron size={13} className="am-chevron" />
        </span>
      </button>
      {open && <NetworkDetails />}
    </div>
  );
}

function Keys({ keys, children }: { keys: string[]; children: ReactNode }) {
  return (
    <>
      <span className="keys">
        {keys.map((k) => (
          <kbd key={k}>{k}</kbd>
        ))}
      </span>
      <span>{children}</span>
    </>
  );
}

/** 每个选项都用它自己的语言书写，看不懂当前语言的人也能找到 */
const LANGS: { id: Lang; label: string }[] = [
  { id: "zh", label: "中文" },
  { id: "en", label: "EN" },
];

export function LangSwitch() {
  const lang = useLang();
  const choose = (l: Lang) => {
    setLang(l);
    // 同步给 Rust 端：托盘菜单、窗口标题、之后产生的提示与错误
    void api.setLanguage(l);
  };
  return (
    <div className="segmented lang-switch" role="group" aria-label="Language / 语言" title="Language / 语言">
      {LANGS.map((l) => (
        <button key={l.id} className={lang === l.id ? "active" : ""} aria-pressed={lang === l.id} onClick={() => choose(l.id)}>
          {l.label}
        </button>
      ))}
    </div>
  );
}

// ───────────────────────── 网络状态（详情层） ─────────────────────────

/** 网络状态的详情：只有状态说明，不显示任何地址（服务器是打包时定好的） */
function NetworkDetails() {
  const t = useT();
  const status = useStore(serverStatus);
  const server = useStore((s) => s.server);
  const problem = useStore((s) => s.serverProblem);
  const app = useStore((s) => s.app);
  return (
    <div className="am-net-body">
      {problem && status !== "connected" && (
        <div className="error-box left" role="alert">
          <IconWarn size={15} />
          <span>
            {problemText(problem)}
            <span className="block sp-retry">{t(server.state === "connecting" ? "server.retrying" : "server.retryLater")}</span>
          </span>
        </div>
      )}

      {app && !app.configured && (
        <div className="error-box left" role="status">
          <IconWarn size={15} />
          <span>{t("server.notSet")}</span>
        </div>
      )}
    </div>
  );
}
