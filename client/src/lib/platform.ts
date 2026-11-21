// 本机（控制台所在的电脑）是 Windows：无边框窗口，最小化 / 最大化 / 关闭按钮由界面画。
import { api } from "./api";

export const OS = "win" as const;

/** 给 <html> 加上系统的 class（桌面版 `os-win`，网页版 `os-web`，样式里区分） */
export function markPlatform() {
  document.documentElement.classList.add(api.frame ? "os-win" : "os-web");
}
