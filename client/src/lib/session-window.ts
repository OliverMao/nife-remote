// 远程窗口：连上一台电脑之后，远程画面在**它自己的窗口**里（桌面版是新的 Tauri 窗口，网页版是新的浏览器
// 标签页），管理界面（控制台）只负责打开 / 切过去。
//
// 要连哪台、开始的画质、配对码由打开窗口的一方带过来：
// * 桌面版：创建窗口时注入的 `window.__NIFE_SESSION__`（见 client/src-tauri/src/viewer.rs）；
// * 网页版：地址里的 `?session=`（见 lib/backend/web.ts 的 openSessionWindow）。
//
// 会话就在这个窗口里建立（见 components/SessionWindow.tsx）：关掉窗口就是断开这一路。
import type { QualitySettings } from "./api";

export interface SessionTarget {
  /** 6 位控制码 */
  code: string;
  /** 这个窗口固定看对方的哪个显示器（从显示器标签拖出来的窗口）；null = 由被控端决定 */
  display: number | null;
  /** 开始时的画质（连上之后还能在控制中心里改） */
  settings: QualitySettings;
  /** 用户输入的配对码（一般没有：第一次连由窗口里就地输） */
  pairing: string | null;
}

/** 本窗口是不是一个远程窗口；是的话返回要连的那台。不是（管理界面）返回 null */
export function sessionTarget(): SessionTarget | null {
  const injected = (globalThis as { __NIFE_SESSION__?: unknown }).__NIFE_SESSION__;
  if (injected) return asTarget(injected);
  if (typeof location === "undefined") return null;
  const raw = new URLSearchParams(location.search).get("session");
  if (!raw) return null;
  try {
    return asTarget(JSON.parse(raw));
  } catch {
    // 地址被人乱改了：当管理界面打开，不让它连到别的地方去
    return null;
  }
}

function asTarget(v: unknown): SessionTarget | null {
  if (typeof v !== "object" || v === null) return null;
  const t = v as Partial<SessionTarget>;
  const code = String(t.code ?? "");
  if (!/^\d{6}$/.test(code)) return null;
  const settings = asSettings(t.settings);
  if (!settings) return null;
  const display = typeof t.display === "number" && Number.isInteger(t.display) && t.display >= 0 ? t.display : null;
  return { code, display, settings, pairing: typeof t.pairing === "string" ? t.pairing : null };
}

/** 画质是打开窗口的一方给的（网页版在地址里，别人能改）：只接受看得懂的，不认识就当不是远程窗口 */
function asSettings(v: unknown): QualitySettings | null {
  if (typeof v !== "object" || v === null) return null;
  const s = v as Partial<QualitySettings>;
  const num = (x: unknown) => typeof x === "number" && Number.isFinite(x) && x >= 0;
  if (!num(s.fps) || !num(s.bitrateKbps) || !num(s.maxShort) || typeof s.adaptive !== "boolean") return null;
  if (s.codec !== "h264" && s.codec !== "h265" && s.codec !== "av1") return null;
  return s as QualitySettings;
}
