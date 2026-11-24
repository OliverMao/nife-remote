import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { ErrorBoundary } from "./components/ErrorBoundary";
import "./styles.css";

// 仅开发模式：方便用 DevTools / 自动化脚本查看会话状态（例如测量音画同步）
if (import.meta.env.DEV) {
  (globalThis as { __NIFE_DEV__?: boolean }).__NIFE_DEV__ = true;
}

const root = () => document.getElementById("root")!;

/**
 * 启动阶段出错（模块加载失败、顶层代码抛异常）时把错误写到页面上。
 *
 * 不这么做的话窗口就是一片白——发布版没有 DevTools，用户和我们都看不到任何信息。
 * 界面用动态 `import()` 加载，才能接住"模块本身加载失败"这种错误。
 */
function fail(e: unknown) {
  const text = e instanceof Error ? `${e.message}\n\n${e.stack ?? ""}` : String(e);
  const box = document.createElement("div");
  box.className = "crash";
  const title = document.createElement("div");
  title.className = "crash-title";
  title.textContent = "界面启动失败";
  const stack = document.createElement("pre");
  stack.className = "crash-stack";
  stack.textContent = text;
  box.append(title, stack);
  root().replaceChildren(box);
}

window.addEventListener("error", (e) => fail(e.error ?? e.message));
// 没人接的 Promise 拒绝只记日志：后台任务（会话断开、一块数据超时）冒出来一个拒绝
// 不该把整个界面换成"启动失败"——那些错误会显示在各自的地方
window.addEventListener("unhandledrejection", (e) => console.error("unhandled rejection:", e.reason));

void import("./App")
  .then(({ App }) => {
    createRoot(root()).render(
      <StrictMode>
        {/* 渲染期出错显示错误本身，不要只留一个白窗口 */}
        <ErrorBoundary>
          <App />
        </ErrorBoundary>
      </StrictMode>,
    );
  })
  .catch(fail);
