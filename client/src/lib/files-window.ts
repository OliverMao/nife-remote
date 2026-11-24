// 文件传输窗口的参数：桌面版由 Rust 注入 `window.__NIFE_FILES__`（见 src-tauri/src/viewer.rs），
// 网页版从 `?files=<JSON>` 读（见 backend/web.ts）。没有就是普通窗口（管理界面或远程窗口）。
export interface FilesTarget {
  /** 控制码（6 位数字） */
  code: string;
  /** 被控端的名字（还没连上时先显示它） */
  name: string;
}

declare global {
  interface Window {
    __NIFE_FILES__?: { code?: unknown; name?: unknown };
  }
}

const str = (v: unknown) => (typeof v === "string" ? v : "");

/** 这个窗口是不是文件传输窗口 */
export function filesTarget(): FilesTarget | null {
  const injected = window.__NIFE_FILES__;
  if (injected) {
    const code = str(injected.code);
    if (/^\d{6}$/.test(code)) return { code, name: str(injected.name) };
  }
  const raw = new URLSearchParams(location.search).get("files");
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as { code?: unknown; name?: unknown };
    const code = str(parsed.code);
    if (/^\d{6}$/.test(code)) return { code, name: str(parsed.name) };
  } catch {
    /* 不是文件窗口 */
  }
  return null;
}
