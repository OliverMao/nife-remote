// 界面上的兜底：任何一处 React 渲染出错，都显示错误本身，而不是一片空白。
//
// 为什么要它：远程窗口、文件传输窗口都是各自独立的 WebView，出问题时如果只留一个白窗口，
// 用户和我们都不知道发生了什么（DevTools 在发布版里是关的）。这里把错误、组件栈打出来，
// 用户截个图就能定位——比"卡死了"有用得多。
import { Component, type ErrorInfo, type ReactNode } from "react";

interface State {
  error: Error | null;
  component: string;
}

export class ErrorBoundary extends Component<{ children: ReactNode }, State> {
  state: State = { error: null, component: "" };

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // 控制台里也留一份（dev 下能直接看）
    console.error("interface error", error, info.componentStack);
    this.setState({ component: info.componentStack ?? "" });
  }

  render() {
    const { error, component } = this.state;
    if (!error) return this.props.children;
    return (
      <div className="crash">
        <div className="crash-title">界面出错了</div>
        <div className="crash-msg">{error.message || String(error)}</div>
        <pre className="crash-stack">{[error.stack ?? "", component].filter(Boolean).join("\n")}</pre>
        <button className="crash-retry" onClick={() => location.reload()}>
          重新加载
        </button>
      </div>
    );
  }
}
