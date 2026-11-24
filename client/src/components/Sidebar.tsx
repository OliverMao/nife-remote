// 左栏（照 UU 远程的版式）：本机被控卡 / 我的设备 / 远程协助 / 设置。
//
// 以后这里还要放登录注册和"绑定自己的设备"，所以账号相关的区域留在最上面（现在先放本机被控卡）。
import type { SavedDevice } from "../lib/api";
import { api } from "../lib/api";
import { loadDevices } from "../lib/actions";
import { useT } from "../lib/i18n";
import { openStateText } from "../lib/messages";
import { store, useStore } from "../lib/store";
import { MyDevices } from "./MyDevices";
import { Logo } from "./Logo";
import { IconExpand, IconMonitor, IconRefresh, IconSwap } from "./Icons";

export function Sidebar() {
  const t = useT();
  return (
    <aside className="sidebar">
      <div className="sb-brand">
        <Logo size={22} />
        <span>Nife Remote</span>
      </div>
      <HostCard />
      <nav className="sb-nav">
        <div className="sb-section">
          {/* 标题在左、刷新在右（刷新原来在设置菜单里，离"我的设备"太远） */}
          <div className="sb-title between">
            <span className="sb-title-text">
              <IconMonitor size={16} />
              {t("devices.title")}
            </span>
            <button
              className="sb-act"
              title={t("devices.refresh")}
              aria-label={t("devices.refresh")}
              onClick={() => void loadDevices()}
            >
              <IconRefresh size={14} />
            </button>
          </div>
          <DeviceList />
        </div>
        <SessionList />
        <div className="sb-section">
          <div className="sb-title">
            <IconSwap size={16} />
            {t("sidebar.assist")}
          </div>
          <button className="sb-item" onClick={() => store.showAssist()}>
            <IconMonitor size={16} />
            <span>{t("sidebar.startAssist")}</span>
          </button>
        </div>
      </nav>
    </aside>
  );
}

/** 左栏顶部（照 UU）：被控状态卡片——只有一句状态 + 一句说明，右上角按钮进去看/踢掉控制端。
 *
 * **只有真的有人在控制本机时才出现**：没人连的时候这张卡整块不显示（包括正在连服务器、
 * 服务器离线这些情况），左栏直接从"我的设备"开始。状态/码都在"远程协助"页里能看到。 */
function HostCard() {
  const t = useT();
  const host = useStore((s) => s.host);

  // 网页版没有被控端（host 一直是 null），桌面版才有这张卡片
  const controllers = host?.controllers.length ?? 0;
  if (!host || controllers === 0) return null;

  return (
    <div className="host-card online">
      <button className="hc-open" title={t("host.expand")} aria-label={t("host.expand")} onClick={() => store.showHost()}>
        <IconExpand size={15} />
      </button>
      <div className="hc-title">{t("host.controlling")}</div>
      <div className="hc-line">{t("host.controllersLine", { n: controllers })}</div>
    </div>
  );
}

/** 会话中：现在开着的远程窗口（一台电脑一个，显示器拖出去的另算），点一下把它叫到前面 */
function SessionList() {
  const t = useT();
  const open = useStore((s) => s.open);
  if (open.length === 0) return null;

  /** 固定看某个显示器的窗口——名字后面写上是哪一块屏幕 */
  const name = (w: (typeof open)[number]) =>
    w.display === null
      ? w.name || t("session.remoteComputer")
      : `${w.name || t("session.remoteComputer")} · ${t("session.display", { n: w.display + 1 })}`;

  return (
    <div className="sb-section">
      <div className="sb-title">
        <IconMonitor size={16} />
        {t("sidebar.sessions")}
      </div>
      {open.map((w) => (
        <button
          key={`${w.code}-${w.display ?? "x"}`}
          className="sb-item"
          title={`${name(w)} · ${openStateText(w.state)}`}
          onClick={() => void api.focusSession(w.code, w.display)}
        >
          <span className={`sb-live ${w.state}`} />
          <span>{name(w)}</span>
        </button>
      ))}
    </div>
  );
}

function DeviceList() {

  // 照 UU：点左栏的设备先进它的详情页，那里再点"显示远程窗口"才真的连
  const open = (d: SavedDevice) => store.showDevice(d.code);
  return <MyDevices onOpen={open} disabled={false} compact />;
}
