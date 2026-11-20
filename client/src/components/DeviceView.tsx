// 设备页（照 UU 远程的设备详情页）：点左栏里的一台设备进来，先看概览再进桌面。
//
// 这一版是界面稿：中间的预览区、下面那排工具（文件传输 / 观看模式 / 终端 / 端口映射）都是占位的，
// 只有「显示远程窗口」是真的去连（开的还是原来的会话标签）。
import { useMemo, useState } from "react";
import type { SavedDevice } from "../lib/api";
import { fmtCode, forgetDevice, loadDevices, openSession } from "../lib/actions";
import { deviceTitle } from "../lib/devices";
import { useT } from "../lib/i18n";
import { bestCodec } from "../lib/codec";
import { DEFAULT_PRESET, presetSettings } from "../lib/presets";
import { store, useStore } from "../lib/store";
import { IconGrid, IconKebab, IconMonitor, IconRefresh, IconSliders, IconTrash, IconVolume } from "./Icons";

/** 工具条上的那几项：都还没有功能，先按 UU 的样子摆上 */
const TOOLS = [
  { icon: <IconMonitor size={16} />, key: "device.tool.files" },
  { icon: <IconVolume size={16} />, key: "device.tool.watch" },
  { icon: <IconSliders size={16} />, key: "device.tool.terminal" },
  { icon: <IconGrid size={16} />, key: "device.tool.ports" },
] as const;

export function DeviceView() {
  const t = useT();
  const code = useStore((s) => s.device);
  const devices = useStore((s) => s.devices);
  const sessions = useStore((s) => s.sessions);
  const decoders = useStore((s) => s.decoders);
  const [menu, setMenu] = useState(false);

  const device: SavedDevice | undefined = useMemo(
    () => devices.find((d) => d.code === code),
    [devices, code],
  );
  const session = sessions.find((s) => s.code === code);

  if (!code) return null;
  const name = device ? deviceTitle(device) : fmtCode(code);
  const quality = device?.quality ?? presetSettings(DEFAULT_PRESET, bestCodec(decoders));
  const live = session?.state === "live";

  const enter = () => void openSession(code, quality);

  return (
    <div className="device-page">
      {/* 照 UU：一行就够——状态 + 名字 + 更多菜单，操作都收进菜单里 */}
      <div className="dv-head">
        <span className={`dv-pill ${live ? "online" : ""}`}>
          <span className="dot" />
          {live ? t("device.online") : t("device.offline")}
        </span>
        <h1>{name}</h1>
        <span className="dv-spacer" />
        <button
          className="dv-kebab"
          title={t("device.more")}
          onClick={() => setMenu((v) => !v)}
          aria-expanded={menu}
        >
          <IconKebab size={16} />
        </button>
        {menu && (
          <div className="dv-menu" onMouseLeave={() => setMenu(false)}>
            <button onClick={() => (setMenu(false), void loadDevices())}>
              <IconRefresh size={15} />
              {t("device.refresh")}
            </button>
            <button className="danger" onClick={() => (setMenu(false), forgetDevice(code), store.setView("new"))}>
              <IconTrash size={15} />
              {t("device.delete")}
            </button>
          </div>
        )}
      </div>

      <div className="dv-card">
        {/* 预览区：真的连着的时候这里就是远程画面，没连时是占位 */}
        <div className="dv-stage">
          <IconMonitor size={40} />
          <div className="dv-stage-title">{live ? t("device.liveTitle") : t("device.idleTitle")}</div>
          <button className="primary-btn" onClick={enter}>
            {live ? t("device.show") : t("device.connect")}
          </button>
        </div>
        <div className="dv-tools">
          {TOOLS.map((tool) => (
            <button key={tool.key} className="dv-tool" disabled title={t("device.soon")}>
              {tool.icon}
              {t(tool.key)}
            </button>
          ))}
          <button className="dv-tool icon" disabled title={t("device.soon")}>
            <IconGrid size={16} />
          </button>
        </div>
      </div>

    </div>
  );
}
