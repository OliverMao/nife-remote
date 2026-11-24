// 把类型化的问题 / 提示变成当前界面语言的文字。
//
// 只在显示时调用（组件渲染时），所以切换语言后已经显示的问题和提示也会跟着变。
// 词条在 i18n.ts 的 problem.* / notice.*；每种 kind 都必须有词条（下面的 `as const` 让 TypeScript 检查）。
import { toProblem, type OpenSessionState, type Problem, type RemoteNotice } from "./api";
import { t } from "./i18n";
import type { RemoteSession } from "./session";

/** 本机产生的提示 */
export type LocalNotice =
  | { kind: "h264Undecodable" }
  | { kind: "codecFallback"; codec: string; to: string }
  | { kind: "noOpus" };

/** 被控端发来的"已恢复"：只用来撤下之前的提示，本身不显示 */
export type RecoveredNotice = Extract<RemoteNotice, { kind: "captureRecovered" | "audioRecovered" }>;

/** "已恢复"撤下的是哪种提示 */
export const RECOVERS: Record<RecoveredNotice["kind"], SessionNotice["kind"]> = {
  captureRecovered: "captureFailed",
  audioRecovered: "audioFailed",
};

/** 会话画面上显示的提示：被控端发来的，或者本机产生的 */
export type SessionNotice = Exclude<RemoteNotice, RecoveredNotice> | LocalNotice;

export function problemText(p: Problem): string {
  // 第一次连一台电脑要配对码：窗口里只有一个"配对码"输入框，不是一句状态说明，这里不写文字
  if (p.kind === "pairingRequired") return "";
  const { kind, ...params } = p;
  // 失败带技术细节（对方给的原话，英文）时一并显示：不然只有一个"操作失败"，没法查
  if (p.kind === "failed" && p.detail.trim()) return t("problem.failedDetail", { detail: p.detail });
  return t(`problem.${kind}` as const, params);
}

export function noticeText(n: SessionNotice): string {
  const { kind, ...params } = n;
  return t(`notice.${kind}` as const, params);
}

/** 会话现在的状态（工具栏上、等待/重连时的提示）：连着时是帧率和延迟 */
export function sessionStateText(s: RemoteSession): string {
  switch (s.state) {
    case "live":
      return `${Math.round(s.stats.fps)} fps · ${s.stats.rttMs != null ? Math.round(s.stats.rttMs) : "—"} ms`;
    case "reconnecting":
      return t("common.reconnecting");
    case "waiting":
      return t("common.waiting");
    case "approving":
      return t("common.approving");
    case "connecting":
      return t("common.connecting");
    case "closed":
      return s.closedReason ? problemText(s.closedReason) : t("common.disconnected");
  }
}

/** 开着的远程窗口现在的样子（左栏"会话中"、设备列表）：控制台只看得到状态名，看不到帧率 */
export function openStateText(state: OpenSessionState): string {
  switch (state) {
    case "live":
      return t("devices.open");
    case "reconnecting":
      return t("common.reconnecting");
    case "waiting":
      return t("common.waiting");
    case "approving":
      return t("common.approving");
    case "connecting":
      return t("common.connecting");
    case "closed":
      return t("common.disconnected");
  }
}

/** 任意异常的说明文字（Rust 命令失败时是 Problem） */
export function errorText(e: unknown): string {
  return problemText(toProblem(e));
}
