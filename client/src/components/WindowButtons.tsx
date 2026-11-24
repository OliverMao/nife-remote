// 无边框窗口右上角的最小化 / 最大化 / 关闭（管理界面和远程窗口共用）。
//
// 远程窗口的"关闭"就是断开这一路（见 SessionView）：这里只负责画按钮，关窗口时做什么由调用方给。
import { useEffect, useState } from "react";
import { api } from "../lib/api";
import { useT } from "../lib/i18n";
import { IconWinClose, IconWinMaximize, IconWinMinimize, IconWinRestore } from "./Icons";

export function WindowButtons({ onClose }: { onClose?: () => void }) {
  const t = useT();
  const frame = api.frame;
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!frame) return;
    const update = () =>
      void frame.isMaximized().then((m) => {
        setMaximized(m);
        // 最大化时四角不圆（和系统的做法一致，见 styles.css 的 html.maximized）
        document.documentElement.classList.toggle("maximized", m);
      });
    update();
    return frame.onResized(update);
  }, [frame]);

  if (!frame) return null;
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
      <button className="win-btn close" title={t("window.close")} onClick={() => (onClose ? onClose() : void frame.close())}>
        <IconWinClose />
      </button>
    </div>
  );
}

/** 点窗口按钮时别让焦点跑到别处（关掉后会停在一个不存在的元素上） */
function keepFocus(e: React.MouseEvent) {
  e.preventDefault();
}
