// 标签栏右侧的两个按钮：
// * 服务器状态（ServerIndicator）：一直写着在线 / 连接中 / 离线，点开是服务器的详情，弹出层就在它下面；
// * 设置（SettingsButton）：本机名称、语言、默认画质、快捷键说明。
//
// 主菜单在最右边（以前在最左边的 Logo 上，和窗口按钮挤在一起，点服务器状态也会在最左边弹出菜单。
import { useCallback, useEffect, useRef, useState, type FormEvent, type ReactNode } from "react";
import { api, toProblem, type Problem } from "../lib/api";
import { focusActive, loadDevices } from "../lib/actions";
import { setLang, useLang, useT, type Lang } from "../lib/i18n";
import { problemText } from "../lib/messages";
import { serverStatus, store, useStore } from "../lib/store";
import { IconChevron, IconKeyboard, IconMenuLines, IconRefresh, IconServer, IconSettings, IconWarn } from "./Icons";
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
        {/* 默认画质：新连接用它，正在连的会话在会话工具栏 / 设备页里单独改 */}
        <div className="am-row am-stack" title={t("menu.qualityHint")}>
          <div className="am-row-line">
            <span className="am-label">{t("menu.quality")}</span>
            <span className="muted small">{presetDesc(store.get().quality)}</span>
          </div>
          <QualitySwitch />
        </div>
      </div>

      <div className="am-actions">
        <button className="am-item" onClick={() => void loadDevices()}>
          <span className="am-item-icon">
            <IconRefresh size={16} />
          </span>
          <span className="am-item-text">{t("menu.refreshDevices")}</span>
        </button>
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

/** 默认画质：省流 / 均衡 / 流畅 / 高清 / 原画（记在 localStorage 里，跟着这台电脑） */
function QualitySwitch() {
  const quality = useStore((s) => s.quality);
  return (
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
  );
}

/** 菜单里的服务器一行：状态 + 改地址/指纹（原来顶栏那个胶囊搬进来） */
function ServerRow() {
  const t = useT();
  const server = useStore((s) => s.server);
  const status = useStore(serverStatus);
  const [open, setOpen] = useState(false);
  return (
    <>
      <button className="am-row am-btn" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        <span className="am-label">{t("server.title")}</span>
        <span className={`am-value srv ${status} ${server.state === "connecting" ? "trying" : ""}`}>
          <span className="dot" />
          {t(`server.${server.state}`)}
          <IconChevron size={13} className="am-chevron" />
        </span>
      </button>
      {open && <ServerPanel onClose={() => setOpen(false)} />}
    </>
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

// ───────────────────────── 服务器状态 ─────────────────────────

/** 标签栏右侧的服务器状态：一直写着是什么状态；点一下在它下面打开服务器的详情 */
export function ServerIndicator() {
  const t = useT();
  const status = useStore(serverStatus);
  const server = useStore((s) => s.server);
  const app = useStore((s) => s.app);
  const { open, close, toggle } = useMenu();
  const label = t(`server.${server.state}`);
  return (
    <div className="menu-anchor">
      <button
        className={`server-ind ${status} ${server.state === "connecting" ? "trying" : ""} ${open ? "on" : ""}`}
        title={`${t("server.title")} · ${label}${app && server.state === "connected" ? ` · ${app.server}` : ""}`}
        aria-expanded={open}
        onClick={toggle}
      >
        <span className="dot" />
        <span className="server-ind-text">{t(`server.short.${status}`)}</span>
      </button>
      {open && <ServerPanel onClose={close} />}
    </div>
  );
}

function ServerPanel({ onClose }: { onClose: () => void }) {
  const t = useT();
  const status = useStore(serverStatus);
  const server = useStore((s) => s.server);
  const problem = useStore((s) => s.serverProblem);
  const app = useStore((s) => s.app);
  // 服务器地址和指纹可以装机后再填（桌面版）；网页版是容器启动时的环境变量，改不了。
  // 还没填过（`app.configured` 为 false）时直接进入编辑：不填就没法用
  const [editing, setEditing] = useState(false);
  const [auth, setAuth] = useState("");
  const [busy, setBusy] = useState(false);
  const [saveError, setSaveError] = useState<Problem | null>(null);
  const autoEdit = useRef(false);

  const startEdit = () => {
    setAuth(app?.auth ?? "");
    setSaveError(null);
    setEditing(true);
  };

  useEffect(() => {
    if (autoEdit.current || api.kind !== "desktop" || !app || app.auth) return;
    autoEdit.current = true;
    startEdit();
  }, [app]);

  // 服务器地址在启动时读一次，所以保存完重启程序让它生效
  const save = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setSaveError(null);
    try {
      const saved = await api.saveAuth(auth);
      // 账号服务地址立刻生效；中转服务器地址下次启动才用于连接
      if (app) store.setApp({ ...app, auth: saved });
      setEditing(false);
      setBusy(false);
    } catch (err) {
      setSaveError(toProblem(err));
      setBusy(false);
    }
  };
  return (
    <Popover className="server-panel" onClose={onClose}>
      <div className="sp-head">
        <span className={`sp-icon ${status}`}>
          <IconServer size={18} />
        </span>
        <div className="sp-head-text">
          <div className="pop-title">{t("server.title")}</div>
          <div className={`sp-state ${status}`}>
            <span className="dot" />
            {t(`server.${server.state}`)}
          </div>
        </div>
      </div>

      {problem && status !== "connected" && (
        <div className="error-box left" role="alert">
          <IconWarn size={15} />
          <span>
            {problemText(problem)}
            <span className="block sp-retry">{t(server.state === "connecting" ? "server.retrying" : "server.retryLater")}</span>
          </span>
        </div>
      )}

      {app && !app.configured && !editing && (
        <div className="error-box left" role="status">
          <IconWarn size={15} />
          <span>{t("server.notSet")}</span>
        </div>
      )}

      {app && !editing && (
        <div className="sp-field">
          <div className="am-row-line">
            <span className="am-label">{t("server.auth")}</span>
            {api.kind === "desktop" && (
              <button className="sp-edit" onClick={startEdit}>
                {t("server.edit")}
              </button>
            )}
          </div>
          <div className="sp-addr mono">{app.auth || t("server.authNotSet")}</div>
          {app.configured && (
            <>
              <span className="am-label sp-relay-label">{t("server.relay")}</span>
              <div className="sp-addr mono">{app.server}</div>
            </>
          )}
        </div>
      )}

      {app && editing && (
        <form className="sp-field" onSubmit={save}>
          <label className="am-label" htmlFor="sp-auth">
            {t("server.auth")}
          </label>
          <input
            id="sp-auth"
            className="sp-input mono"
            value={auth}
            onChange={(e) => setAuth(e.target.value)}
            placeholder={t("server.authHint")}
            autoFocus
            spellCheck={false}
          />
          <p className="sp-hint">{t("server.authAssigned")}</p>
          {saveError && (
            <div className="error-box left" role="alert">
              <IconWarn size={15} />
              <span>{problemText(saveError)}</span>
            </div>
          )}
          <div className="sp-actions">
            <button type="submit" className="primary" disabled={busy}>
              {t("server.saveAndRestart")}
            </button>
            <button type="button" onClick={() => setEditing(false)} disabled={busy}>
              {t("server.cancel")}
            </button>
          </div>
        </form>
      )}

      <p className="sp-hint">{t("server.hint")}</p>
    </Popover>
  );
}
