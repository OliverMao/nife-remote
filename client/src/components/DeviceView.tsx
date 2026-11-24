// 设备页（照 UU 远程的设备详情页）：点左栏里的一台设备进来，先看概览再进桌面。
//
// 这一版是界面稿：中间的预览区、下面那排工具（文件传输 / 观看模式 / 终端 / 端口映射）都是占位的，
// 只有「显示远程窗口」是真的：点一下会打开它自己的远程窗口（配对码、连接进度、错误都在那个窗口里）。
import { useEffect, useMemo, useState } from "react";
import { toProblem, type Problem, type SavedDevice } from "../lib/api";
import {
  defaultQuality,
  forgetDevice,
  loadDevices,
  openSession,
  PRESENCE_EVERY_MS,
  refreshPresence,
  removeAccountDevice,
} from "../lib/actions";
import { accountDevices, deviceReachable, deviceTitle } from "../lib/devices";
import { useT } from "../lib/i18n";
import { problemText } from "../lib/messages";
import { store, useStore } from "../lib/store";
import { openFileTransfer } from "./FileTransfer";
import { IconGrid, IconKebab, IconMonitor, IconRefresh, IconSliders, IconTrash, IconVolume } from "./Icons";

/** 工具条上的那几项：都还没有功能，先按 UU 的样子摆上 */
/** 还没做的功能：按钮占位，鼠标悬停说明"即将推出" */
const TOOLS = [
  { icon: <IconVolume size={16} />, key: "device.tool.watch" },
  { icon: <IconSliders size={16} />, key: "device.tool.terminal" },
  { icon: <IconGrid size={16} />, key: "device.tool.ports" },
] as const;

export function DeviceView() {
  const t = useT();
  const code = useStore((s) => s.device);
  const devices = useStore((s) => s.devices);
  const windows = useStore((s) => s.open);
  const presence = useStore((s) => s.presence);
  const [menu, setMenu] = useState(false);
  // 远程窗口打不开时的原因（连接本身的问题在那个窗口里显示）
  const [error, setError] = useState<Problem | null>(null);
  const [busy, setBusy] = useState(false);

  // 这台现在能不能连（进来问一次，之后隔一会儿再问）
  useEffect(() => {
    void refreshPresence();
    const timer = window.setInterval(() => void refreshPresence(true), PRESENCE_EVERY_MS);
    return () => window.clearInterval(timer);
  }, []);

  const account = useStore((s) => s.account);
  const device: SavedDevice | undefined = useMemo(
    () => accountDevices(account, devices).find((d) => d.code === code) ?? devices.find((d) => d.code === code),
    [account, devices, code],
  );
  const win = windows.find((w) => w.code === code);

  if (!code) return null;
  // 名字就是设备名（控制码不在界面上显示，见"远程协助"页）
  const name = device ? deviceTitle(device) : "";
  const quality = device?.quality ?? defaultQuality();
  const live = win?.state === "live";
  // 和左栏用的是同一个判断：true 在线、false 不在线、undefined 不知道（没连过这台、服务器旧、还没问完）
  const online = device ? deviceReachable(device, presence, account?.deviceOnline) : undefined;
  const reachable = online ?? live;
  const state = reachable ? "online" : online === false ? "offline" : "unknown";

  // 远程画面在它自己的窗口里：已经开着就只是把它叫到前面（见 src-tauri/src/viewer.rs）
  const enter = async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    setError(await openSession(code, quality));
    setBusy(false);
  };

  return (
    <div className="device-page">
      {/* 照 UU：一行就够——状态 + 名字 + 更多菜单，操作都收进菜单里 */}
      <div className="dv-head">
        <span className={`dv-pill ${state === "online" ? "online" : ""}`}>
          <span className="dot" />
          {t(state === "online" ? "device.online" : state === "offline" ? "device.offline" : "device.unknownState")}
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
            <button
              onClick={() => {
                setMenu(false);
                void loadDevices();
                void refreshPresence(true);
              }}
            >
              <IconRefresh size={15} />
              {t("device.refresh")}
            </button>
            <button
              className="danger"
              onClick={() => {
                setMenu(false);
                forgetDevice(code);
                void removeAccountDevice(code);
                store.setView("new");
              }}
            >
              <IconTrash size={15} />
              {t("device.delete")}
            </button>
          </div>
        )}
      </div>

      <div className="dv-card">
        {/* 预览区：远程画面在它自己的窗口里，这里是占位 */}
        <div className="dv-stage">
          <IconMonitor size={40} />
          <div className="dv-stage-title">{live ? t("device.liveTitle") : t("device.idleTitle")}</div>
          {error && (
            <div className="error-box left" role="alert">
              <span>{problemText(error)}</span>
            </div>
          )}
          <button className="primary-btn" onClick={() => void enter()} disabled={busy}>
            {live ? t("device.show") : t("device.connect")}
          </button>
        </div>
        <div className="dv-tools">
          {/* 文件传输：另开一个窗口（左边本机、右边对方），和远程窗口的控制中心里那一项是同一件事 */}
          <button
            className="dv-tool"
            title={t("files.title")}
            onClick={() =>
              void openFileTransfer(code, device ? deviceTitle(device) : "").catch((e: unknown) =>
                setError(toProblem(e)),
              )
            }
          >
            <IconMonitor size={16} />
            {t("device.tool.files")}
          </button>
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
