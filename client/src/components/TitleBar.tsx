// 标题栏（无边框窗口的顶栏，照 UU 远程）：左边 Logo + 名字，右边状态 + 通知/账号/菜单 + 窗口按钮。
//
// 不再有标签条：一次只显示一个页面，多个远程会话靠左栏的"会话中"列表切换。
//
// macOS 上最左边是系统的红绿灯按钮（没有 Logo），右边没有窗口按钮。
import { useEffect, useState } from "react";
import { api } from "../lib/api";
import { useT } from "../lib/i18n";
import { NATIVE_WINDOW_BUTTONS } from "../lib/platform";
import { AccountButton } from "./AccountMenu";
import { SettingsButton } from "./AppMenu";
import {
  IconWinClose,
  IconWinMaximize,
  IconWinMinimize,
  IconWinRestore,
} from "./Icons";

export function TitleBar() {
  return (
    <header className={`titlebar ${NATIVE_WINDOW_BUTTONS ? "native-buttons" : ""}`} data-tauri-drag-region="deep">
      <div className="title-spacer" data-tauri-drag-region />
      <div className="title-actions">
        <AccountButton />
        <SettingsButton burger />
      </div>
      {/* 分隔线只在真有窗口按钮（桌面版）时才画：网页版右边是浏览器自己的按钮 */}
      {api.frame && !NATIVE_WINDOW_BUTTONS && (
        <>
          <span className="ta-sep" aria-hidden />
          <WindowButtons />
        </>
      )}
    </header>
  );
}

/** 点窗口按钮时别让焦点跑到别处（关掉后会停在一个不存在的元素上） */
function keepFocus(e: React.MouseEvent) {
  e.preventDefault();
}

// ───────────────────────── 窗口按钮 ─────────────────────────

function WindowButtons() {
  const t = useT();
  const frame = api.frame!;
  const [maximized, setMaximized] = useState(false);
  useEffect(() => {
    const update = () =>
      void frame.isMaximized().then((m) => {
        setMaximized(m);
        // 没有最大化时 Linux 上画一圈细边框（系统不给无边框窗口画阴影）
        document.documentElement.classList.toggle("maximized", m);
      });
    update();
    return frame.onResized(update);
  }, [frame]);
  return (
    <div className="win-buttons">
      <button className="win-btn" title={t("window.minimize")} onMouseDown={keepFocus} onClick={() => void frame.minimize()}>
        <IconWinMinimize />
      </button>
      <button
        className="win-btn"
        title={maximized ? t("window.restore") : t("window.maximize")}
        onMouseDown={keepFocus}
        onClick={() => void frame.toggleMaximize()}
      >
        {maximized ? <IconWinRestore /> : <IconWinMaximize />}
      </button>
      {/* 和系统的关闭按钮一样：有会话时先确认（见 App 里的 onCloseRequested） */}
      <button className="win-btn close" title={t("window.close")} onClick={() => void frame.close()}>
        <IconWinClose />
      </button>
    </div>
  );
}
