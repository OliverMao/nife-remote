// 本机麦克风的采集（主线程只负责打开和关掉）：getUserMedia → AudioWorklet（mic-worklet.ts：混成单声道、换成 48 kHz、
// 每 20 ms 一块）→ 经 MessageChannel 直接交给控制台的 Worker（mic-encoder.ts：编成 Opus、打包、发给控制端服务）。
//
// 被控端把这些声音放进它的虚拟麦克风（Windows：Steam Streaming Microphone），那台电脑上的程序就听得到
// （协议见 crates/airhand-proto/src/session.rs 的"麦克风"）。打开浏览器的回声消除：远程的声音从这台电脑的扬声器放出来，
// 不会再被麦克风录回去。
import workletUrl from "./mic-worklet?worker&url";
import { MIC_CONFIG } from "./mic-encoder";
import type { MicControl } from "./mic-worklet";

/** 本机为什么采集不了：浏览器不支持、没允许、没有麦克风、别的原因 */
export type MicError = "unsupported" | "denied" | "noDevice" | "failed";

let support: Promise<boolean> | null = null;

/** 这个浏览器 / WebView 能不能采集并编码麦克风（getUserMedia、AudioWorklet、WebCodecs 的 Opus 编码） */
export function micSupported(): Promise<boolean> {
  support ??= (async () => {
    if (!navigator.mediaDevices?.getUserMedia || typeof AudioWorkletNode === "undefined") return false;
    if (typeof AudioEncoder === "undefined") return false;
    try {
      return (await AudioEncoder.isConfigSupported(MIC_CONFIG)).supported === true;
    } catch {
      return false;
    }
  })();
  return support;
}

function errorOf(e: unknown): MicError {
  const name = e instanceof DOMException ? e.name : "";
  if (name === "NotAllowedError" || name === "SecurityError") return "denied";
  if (name === "NotFoundError" || name === "OverconstrainedError") return "noDevice";
  return "failed";
}

export class MicCapture {
  /** 麦克风没了（拔掉了、被系统收回了）：采集已经停了 */
  onEnded: (() => void) | null = null;

  private gen = 0;
  private stream: MediaStream | null = null;
  private ctx: AudioContext | null = null;
  private node: AudioWorkletNode | null = null;
  /** 把采集到的声音接到 Worker 上；null = 停止 */
  private readonly attach: (port: MessagePort | null) => void;

  constructor(attach: (port: MessagePort | null) => void) {
    this.attach = attach;
  }

  get running(): boolean {
    return this.node !== null;
  }

  /** 开始采集（第一次会请用户允许使用麦克风）。失败时抛出 MicError；中途被 stop 了就什么也不做 */
  async start(): Promise<void> {
    if (this.running) return;
    const gen = ++this.gen;
    if (!(await micSupported())) throw "unsupported" satisfies MicError;
    let stream: MediaStream;
    try {
      stream = await navigator.mediaDevices.getUserMedia({
        audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true, channelCount: 1 },
      });
    } catch (e) {
      throw errorOf(e);
    }
    if (gen !== this.gen) {
      for (const t of stream.getTracks()) t.stop();
      return;
    }
    // 不指定采样率：有的浏览器不允许把麦克风接到采样率不一样的 AudioContext 上，换成 48 kHz 由音频线程做
    const ctx = new AudioContext({ latencyHint: "interactive" });
    try {
      await ctx.audioWorklet.addModule(workletUrl);
      if (gen !== this.gen) throw new Error("stopped");
      const node = new AudioWorkletNode(ctx, "airhand-mic", {
        numberOfInputs: 1,
        numberOfOutputs: 1,
        outputChannelCount: [1],
      });
      ctx.createMediaStreamSource(stream).connect(node);
      // 输出的是静音：接上扬声器只是为了让音频线程一直处理它
      node.connect(ctx.destination);
      const pipe = new MessageChannel();
      node.port.postMessage({ port: pipe.port2 } satisfies MicControl, [pipe.port2]);
      this.attach(pipe.port1);
      await ctx.resume().catch(() => {});
      this.stream = stream;
      this.ctx = ctx;
      this.node = node;
    } catch (e) {
      for (const t of stream.getTracks()) t.stop();
      void ctx.close().catch(() => {});
      if (gen !== this.gen) return;
      console.warn("can't start the microphone", e);
      throw "failed" satisfies MicError;
    }
    stream.getAudioTracks()[0]?.addEventListener("ended", () => {
      if (gen !== this.gen) return;
      this.stop();
      this.onEnded?.();
    });
  }

  stop() {
    this.gen++;
    if (this.node) {
      this.attach(null);
      this.node.port.postMessage({ port: null } satisfies MicControl);
      this.node.disconnect();
      this.node = null;
    }
    void this.ctx?.close().catch(() => {});
    this.ctx = null;
    for (const t of this.stream?.getTracks() ?? []) t.stop();
    this.stream = null;
  }
}
