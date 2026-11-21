// 左栏（照 UU 远程的版式）：本机被控卡 / 我的设备 / 远程协助 / 设置。
//
// 以后这里还要放登录注册和"绑定自己的设备"，所以账号相关的区域留在最上面（现在先放本机被控卡）。
import type { SavedDevice } from "../lib/api";
import { fmtCode } from "../lib/actions";
import { useT } from "../lib/i18n";
import { store, useStore } from "../lib/store";
import { MyDevices } from "./MyDevices";
import { Logo } from "./Logo";
import { IconExpand, IconMonitor, IconSwap } from "./Icons";

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
          <div className="sb-title">
            <IconMonitor size={16} />
            {t("devices.title")}
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

/** 左栏里的"我的设备"：点一下直接连（画质用上次的） */
/** 会话中（原来的顶部标签条）：没有标签页之后，多个远程桌面在这里切换 */
function SessionList() {
  const t = useT();
  const sessions = useStore((s) => s.sessions);
  const view = useStore((s) => s.view);
  if (sessions.length === 0) return null;
  return (
    <div className="sb-section">
      <div className="sb-title">
        <IconMonitor size={16} />
        {t("sidebar.sessions")}
      </div>
      {sessions.map((s) => (
        <button
          key={s.id}
          className={`sb-item ${view === s.id ? "on" : ""}`}
          title={`${s.hostName} · ${fmtCode(s.code)}`}
          onClick={() => store.setView(s.id)}
        >
          <span className={`sb-live ${s.state}`} />
          <span>{s.hostName || fmtCode(s.code)}</span>
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
