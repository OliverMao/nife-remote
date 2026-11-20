// AudioWorklet：本机麦克风的采集（由 mic.ts 通过 `?worker&url` 加载）。
// 把麦克风的声音混成单声道、换成 48 kHz，每 20 ms（960 个采样）一块，经主线程给的端口直接交给控制台的 Worker
// （mic-encoder.ts 在那里编成 Opus 发出去），不经过主线程。

import { MIC_FRAME, MIC_RATE } from "./mic-encoder";

// AudioWorkletGlobalScope 的全局对象（TypeScript 没有对应的 lib）
declare const sampleRate: number;
declare function registerProcessor(name: string, ctor: unknown): void;
declare class AudioWorkletProcessor {
  readonly port: MessagePort;
}

/** 主线程 → 音频线程：PCM 往哪个端口发；`null` = 停止 */
export type MicControl = { port: MessagePort | null };

class AirhandMic extends AudioWorkletProcessor {
  private out: MessagePort | null = null;
  /** 每个输出采样在输入上走多远（输入采样率 / 48000） */
  private readonly step = sampleRate / MIC_RATE;
  /** 下一个输出在上一个输入（prev）和这一个输入之间的位置 */
  private pos = 1;
  private prev = 0;
  private frame = new Float32Array(MIC_FRAME);
  private n = 0;

  constructor() {
    super();
    this.port.onmessage = (e: MessageEvent<MicControl>) => {
      this.out?.close();
      this.out = e.data.port;
      this.n = 0;
    };
  }

  process(inputs: Float32Array[][]): boolean {
    const chans = inputs[0];
    if (!this.out || !chans || chans.length === 0) return true;
    const len = chans[0].length;
    for (let i = 0; i < len; i++) {
      let x = 0;
      for (const c of chans) x += c[i];
      x /= chans.length;
      // 线性插值：说话的声音够用了
      while (this.pos <= 1) {
        this.emit(this.prev + (x - this.prev) * this.pos);
        this.pos += this.step;
      }
      this.pos -= 1;
      this.prev = x;
    }
    return true;
  }

  private emit(v: number) {
    this.frame[this.n++] = v;
    if (this.n < MIC_FRAME) return;
    const full = this.frame;
    this.frame = new Float32Array(MIC_FRAME);
    this.n = 0;
    this.out?.postMessage(full, [full.buffer]);
  }
}

registerProcessor("airhand-mic", AirhandMic);
