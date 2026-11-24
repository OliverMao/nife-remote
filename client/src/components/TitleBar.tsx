// 标题栏（无边框窗口的顶栏，照 UU 远程）：左边 Logo + 名字，右边状态 + 通知/账号/菜单 + 窗口按钮。
//
// 这里只有管理界面（我的设备、远程协助、被控中……）：远程画面在每台电脑自己的窗口里，
// 左栏的"会话中"列出它们。
//
// 窗口按钮由界面画（最小化 / 最大化 / 关闭），见 WindowButtons。
import { api } from "../lib/api";
import { AccountButton } from "./AccountMenu";
import { SettingsButton } from "./AppMenu";
import { WindowButtons } from "./WindowButtons";

export function TitleBar() {
  return (
    <header className="titlebar" data-tauri-drag-region="deep">
      <div className="title-spacer" data-tauri-drag-region />
      <div className="title-actions">
        <AccountButton />
        <SettingsButton burger />
      </div>
      {/* 分隔线只在真有窗口按钮（桌面版）时才画：网页版右边是浏览器自己的按钮 */}
      {api.frame && (
        <>
          <span className="ta-sep" aria-hidden />
          {/* 和系统的关闭按钮一样：只把界面藏起来，程序还在托盘里（见 App 里的 onCloseRequested） */}
          <WindowButtons />
        </>
      )}
    </header>
  );
}
