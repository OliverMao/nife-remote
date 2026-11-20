import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { store } from "./lib/store";
import "./styles.css";

// 仅开发模式：方便用 DevTools / 自动化脚本查看会话状态（例如测量音画同步）
if (import.meta.env.DEV) {
  Object.assign(window, { __nife_remote: { store } });
  // 界面预览开关（见 backend/web.ts 的 hostStatus）：只在开发时存在
  (globalThis as { __NIFE_DEV__?: boolean }).__NIFE_DEV__ = true;
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
