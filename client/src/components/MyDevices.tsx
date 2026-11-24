// 我的设备（左栏里那一列）：**登录同一个账号的设备**（账号服务里存着的列表），不是本机的连接历史。
// 点一下进它的详情页，详情页里再点"显示远程窗口"才真的连（画质用上次的，见 DeviceView）。
// 本机也会在这个列表里（登录后自己登记的），标成"本机"、不能连自己。
import { useEffect, useState } from "react";
import type { OpenSession, SavedDevice } from "../lib/api";
import { forgetDevice, refreshPresence, registerSelfWithAccount, removeAccountDevice, renameDevice, PRESENCE_EVERY_MS } from "../lib/actions";
import { accountDevices, deviceReachable, deviceTitle, matchDevice, relativeTime } from "../lib/devices";
import { getLang, useT } from "../lib/i18n";
import { openStateText } from "../lib/messages";
import { matchPreset, presetName } from "../lib/presets";
import { useStore } from "../lib/store";
import { IconClose, IconMonitor, IconPencil, IconSearch, IconWinMark } from "./Icons";

/** 超过这么多台时显示搜索框 */
const SEARCH_FROM = 7;

/** 我的设备：登录同一个账号的设备（本机也在里面）。点一下进详情；可以重命名、移除（问一句） */
export function MyDevices({ onOpen, disabled, compact = false }: { onOpen: (d: SavedDevice) => void; disabled: boolean; compact?: boolean }) {
  const t = useT();
  const account = useStore((s) => s.account);
  const local = useStore((s) => s.devices);
  const saved = useStore((s) => s.devicesSaved);
  const host = useStore((s) => s.host);
  // 账号里的设备 + 本机记下的那些（名字、重连凭证、画质）；没连过的只有控制码
  const devices = accountDevices(account, local);
  const self = host?.code ? String(host.code) : null;
  const windows = useStore((s) => s.open);
  const presence = useStore((s) => s.presence);
  const [query, setQuery] = useState("");
  const [editing, setEditing] = useState<string | null>(null);
  // 等确认删除的那一台
  const [asking, setAsking] = useState<SavedDevice | null>(null);

  // 登录之后、本机控制码变了：把自己重新登记到账号的设备列表
  useEffect(() => {
    void registerSelfWithAccount();
  }, [account?.devices, host?.code]);

  // 进来先问一次哪几台现在能连，之后隔一会儿再问一次（"我的设备"开着就在问）
  useEffect(() => {
    void refreshPresence();
    const timer = window.setInterval(() => void refreshPresence(true), PRESENCE_EVERY_MS);
    return () => window.clearInterval(timer);
  }, []);

  if (!account) {
    // 没登录：这里显示的是"同一账号下的设备"，登录才有内容
    return saved || compact ? <p className="nt-empty muted small">{t("devices.signIn")}</p> : null;
  }
  if (devices.length === 0) {
    // 账号里一台都还没有（刚登录、还没登记完）
    return saved || compact ? <p className="nt-empty muted small">{t("devices.empty")}</p> : null;
  }
  const open = new Map(windows.map((w) => [w.code, w]));
  const shown = devices.filter((d) => matchDevice(d, query));

  return (
    <section className={`nt-devices ${compact ? "compact" : ""}`}>
      {!compact && <div className="section-title">
        <span>
          <IconMonitor size={14} />
          {t("devices.title")}
          <span className="count">{devices.length}</span>
        </span>
        {devices.length >= SEARCH_FROM ? (
          <label className="nt-search">
            <IconSearch size={14} />
            <input
              value={query}
              placeholder={t("devices.search")}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => e.key === "Escape" && setQuery("")}
            />
          </label>
        ) : (
          <span className="small">{t(saved ? "devices.hint" : "devices.hintWeb")}</span>
        )}
      </div>}

      <div className="device-list">
        {shown.map((d) => (
          <DeviceItem
            key={d.code}
            d={d}
            session={open.get(d.code)}
            // 这台现在能不能连（服务器说的；不知道时是 undefined）
            online={deviceReachable(d, presence, account.deviceOnline)}
            disabled={disabled}
            editing={editing === d.code}
            onOpen={() => onOpen(d)}
            onEdit={(on) => setEditing(on ? d.code : null)}
            self={self === d.code}
            onForget={() => setAsking(d)}
          />
        ))}
      </div>
      {shown.length === 0 && <p className="nt-empty muted small">{t("devices.noMatch")}</p>}

      {asking && (
        <div className="confirm-mask" role="presentation" onClick={() => setAsking(null)}>
          <div
            className="confirm-card"
            role="dialog"
            aria-modal="true"
            aria-label={t("devices.removeTitle")}
            onClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => e.key === "Escape" && setAsking(null)}
          >
            <div className="confirm-title">{t("devices.removeTitle")}</div>
            <p className="confirm-text">{t("devices.removeAsk", { name: deviceTitle(asking) })}</p>
            <div className="confirm-actions">
              <button className="confirm-btn" autoFocus onClick={() => setAsking(null)}>
                {t("common.cancel")}
              </button>
              <button
                className="confirm-btn danger"
                onClick={() => {
                  forgetDevice(asking.code);
                  void removeAccountDevice(asking.code);
                  setAsking(null);
                }}
              >
                {t("common.delete")}
              </button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}

function DeviceItem({
  d,
  session,
  online,
  self,
  disabled,
  editing,
  onOpen,
  onEdit,
  onForget,
}: {
  d: SavedDevice;
  /** 这台就是本机：不显示"能不能连"，也不能连自己 */
  self: boolean;
  /** 已经打开的远程窗口（点一下把它叫到前面） */
  session: OpenSession | undefined;
  /** 这台现在能不能连：true = 在线，false = 不在线，undefined = 不知道（没连过、服务器旧、还没问完） */
  online: boolean | undefined;
  disabled: boolean;
  editing: boolean;
  onOpen: () => void;
  onEdit: (on: boolean) => void;
  onForget: () => void;
}) {
  const t = useT();
  const title = self ? t("common.thisComputer") : deviceTitle(d);
  // 打开着的可以随时切过去；其余的要等服务器连上、没有正在进行的连接
  const off = self || (disabled && !session);
  // 这行显示的是"这台现在能不能连"（服务器说的），不是"本机有没有连着它"；问不出来时退回原来的说法
  const status = self
    ? t("devices.thisDevice")
    : online !== undefined
      ? online
        ? t("device.online")
        : t("device.offline")
      : session
        ? openStateText(session.state)
        : relativeTime(d.lastUsed, Date.now(), getLang() === "zh" ? "zh-CN" : "en");
  const dot = self ? undefined : online !== undefined ? `online-${online}` : session?.state;
  const p = d.quality ? matchPreset(d.quality) : null;
  const finish = (value: string | null) => {
    if (value !== null) renameDevice(d.code, value);
    onEdit(false);
  };

  return (
    <div
      role="button"
      tabIndex={0}
      className={`device-item ${session ? "open" : ""} ${off && !self ? "disabled" : ""} ${self ? "self" : ""} ${editing ? "editing" : ""}`}
      title={[title !== d.name && d.name, d.os].filter(Boolean).join(" · ")}
      onClick={() => !off && !editing && onOpen()}
      onKeyDown={(e) => e.key === "Enter" && !off && !editing && onOpen()}
    >
      <span className="device-mark">
        <IconWinMark size={16} />
      </span>
      <span className="device-meta">
        {editing ? (
          <input
            className="device-rename"
            autoFocus
            defaultValue={title}
            maxLength={60}
            onClick={(e) => e.stopPropagation()}
            onFocus={(e) => e.target.select()}
            onKeyDown={(e) => {
              e.stopPropagation();
              if (e.key === "Enter") finish(e.currentTarget.value);
              else if (e.key === "Escape") finish(null);
            }}
            onBlur={(e) => finish(e.currentTarget.value)}
          />
        ) : (
          <span className="device-name">{title}</span>
        )}
        {/* 副标题不显示控制码：同账号的设备靠名字认（控制码只在"远程协助"页输入） */}
        <span className="device-sub">
          {dot && <span className={`dot ${dot}`} />}
          {status}
          {!session && d.quality && `${status ? " · " : ""}${p ? presetName(p) : t("connect.customQuality")}`}
        </span>
      </span>
      {/* 左栏紧凑版没有副标题：用一个圆点表示这台现在能不能连 */}
      {!self && online !== undefined && <span className={`device-reach ${online ? "on" : ""}`} />}
      {!editing && !self && (
        <span className="device-actions">
          <button
            className="icon-btn subtle"
            title={t("devices.rename")}
            onClick={(e) => {
              e.stopPropagation();
              onEdit(true);
            }}
          >
            <IconPencil size={13} />
          </button>
          <button
            className="icon-btn subtle"
            title={t("devices.forget")}
            onClick={(e) => {
              e.stopPropagation();
              onForget();
            }}
          >
            <IconClose size={13} />
          </button>
        </span>
      )}
    </div>
  );
}
