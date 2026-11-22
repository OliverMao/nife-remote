// 我的设备（左栏里那一列）：连接过的电脑，最近连过的在前面。点一下进它的详情页，
// 详情页里再点"显示远程窗口"才真的连（画质用上次的，见 DeviceView）。
import { useEffect, useState } from "react";
import type { SavedDevice } from "../lib/api";
import { fmtCode, forgetDevice, refreshPresence, renameDevice, restoreDevice, PRESENCE_EVERY_MS } from "../lib/actions";
import { deviceTitle, matchDevice, relativeTime } from "../lib/devices";
import { getLang, useT } from "../lib/i18n";
import { sessionStateText } from "../lib/messages";
import { matchPreset, presetName } from "../lib/presets";
import type { RemoteSession } from "../lib/session";
import { useSession, useStore } from "../lib/store";
import { IconClose, IconMonitor, IconPencil, IconSearch, IconWinMark } from "./Icons";

/** 超过这么多台时显示搜索框 */
const SEARCH_FROM = 7;

/** 我的设备：连接过的电脑，最近连过的在前面。点一下就连；可以重命名、移除（能撤销） */
export function MyDevices({ onOpen, disabled, compact = false }: { onOpen: (d: SavedDevice) => void; disabled: boolean; compact?: boolean }) {
  const t = useT();
  const devices = useStore((s) => s.devices);
  const saved = useStore((s) => s.devicesSaved);
  const sessions = useStore((s) => s.sessions);
  const presence = useStore((s) => s.presence);
  const [query, setQuery] = useState("");
  const [editing, setEditing] = useState<string | null>(null);
  const [removed, setRemoved] = useState<SavedDevice | null>(null);

  // 进来先问一次哪几台现在能连，之后隔一会儿再问一次（"我的设备"开着就在问）
  useEffect(() => {
    void refreshPresence();
    const timer = window.setInterval(() => void refreshPresence(true), PRESENCE_EVERY_MS);
    return () => window.clearInterval(timer);
  }, []);

  // 撤销的机会只保留一会儿
  useEffect(() => {
    if (!removed) return;
    const timer = window.setTimeout(() => setRemoved(null), 6000);
    return () => clearTimeout(timer);
  }, [removed]);

  if (devices.length === 0 && !removed) {
    // 第一次用：告诉用户连过的电脑会出现在这里（左栏里一直显示这句，不然那儿是空的）
    return saved || compact ? <p className="nt-empty muted small">{t("devices.empty")}</p> : null;
  }
  const open = new Map(sessions.map((s) => [s.code, s]));
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

      {removed && (
        <div className="undo-bar" role="status">
          <span>{t("devices.removed", { name: deviceTitle(removed) })}</span>
          <button
            className="text-btn"
            onClick={() => {
              restoreDevice(removed);
              setRemoved(null);
            }}
          >
            {t("devices.undo")}
          </button>
        </div>
      )}

      <div className="device-list">
        {shown.map((d) => (
          <DeviceItem
            key={d.code}
            d={d}
            session={open.get(d.code)}
            // 这台现在能不能连（服务器说的；不知道时是 undefined）
            online={presence && d.resume ? presence[d.code] === true : undefined}
            disabled={disabled}
            editing={editing === d.code}
            onOpen={() => onOpen(d)}
            onEdit={(on) => setEditing(on ? d.code : null)}
            onForget={() => {
              forgetDevice(d.code);
              setRemoved(d);
            }}
          />
        ))}
      </div>
      {shown.length === 0 && <p className="nt-empty muted small">{t("devices.noMatch")}</p>}
    </section>
  );
}

function DeviceItem({
  d,
  session,
  online,
  disabled,
  editing,
  onOpen,
  onEdit,
  onForget,
}: {
  d: SavedDevice;
  /** 已经打开的标签（点一下就切过去） */
  session: RemoteSession | undefined;
  /** 这台现在能不能连：true = 在线，false = 不在线，undefined = 不知道（没连过、服务器旧、还没问完） */
  online: boolean | undefined;
  disabled: boolean;
  editing: boolean;
  onOpen: () => void;
  onEdit: (on: boolean) => void;
  onForget: () => void;
}) {
  const t = useT();
  useSession(session);
  const title = deviceTitle(d);
  // 打开着的可以随时切过去；其余的要等服务器连上、没有正在进行的连接
  const off = disabled && !session;
  // 这行显示的是"这台现在能不能连"（服务器说的），不是"本机有没有连着它"；问不出来时退回原来的说法
  const status =
    online !== undefined
      ? online
        ? t("device.online")
        : t("device.offline")
      : session
        ? session.state === "live"
          ? t("devices.open")
          : sessionStateText(session)
        : relativeTime(d.lastUsed, Date.now(), getLang() === "zh" ? "zh-CN" : "en");
  const dot = online !== undefined ? `online-${online}` : session?.state;
  const p = d.quality ? matchPreset(d.quality) : null;
  const finish = (value: string | null) => {
    if (value !== null) renameDevice(d.code, value);
    onEdit(false);
  };

  return (
    <div
      role="button"
      tabIndex={0}
      className={`device-item ${session ? "open" : ""} ${off ? "disabled" : ""} ${editing ? "editing" : ""}`}
      title={[title !== d.name && d.name, d.os, fmtCode(d.code)].filter(Boolean).join(" · ")}
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
        <span className="device-sub">
          {dot && <span className={`dot ${dot}`} />}
          {fmtCode(d.code)}
          {status && ` · ${status}`}
          {!session && d.quality && ` · ${p ? presetName(p) : t("connect.customQuality")}`}
        </span>
      </span>
      {/* 左栏紧凑版没有副标题：用一个圆点表示这台现在能不能连 */}
      {online !== undefined && <span className={`device-reach ${online ? "on" : ""}`} />}
      {!editing && (
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
