// 一个会话的麦克风编码（在 Worker 里，见 console-worker.ts）：音频线程（mic-worklet.ts）经端口直接送来 48 kHz
// 单声道 PCM，每 20 ms 一块；WebCodecs 编成 Opus，打成声音包（media.ts 的 audioPacket）交给 Worker 发出去。
// 带上之前几包的副本、加密、走哪条路是控制端服务的事（见 crates/nife-remote-link/src/controller/mod.rs 的"麦克风"）。
import { audioPacket } from "./media";

/** 与 nife_remote_proto::media 的麦克风格式一致：48 kHz 单声道，每包 20 ms（960 个采样） */
export const MIC_RATE = 48000;
export const MIC_FRAME = 960;

/** Opus 48 kHz 单声道，每包 20 ms，32 kbps：说话足够清楚 */
export const MIC_CONFIG: AudioEncoderConfig = {
  codec: "opus",
  sampleRate: MIC_RATE,
  numberOfChannels: 1,
  bitrate: 32_000,
  opus: { frameDuration: 20_000 },
};

/** 一个 Opus 包最多这么大（nife_remote_proto::media::MAX_OPUS_PACKET 的下限，32 kbps 的包只有一百来字节） */
const MAX_OPUS = 900;
/** 编码跟不上（几乎不会发生）：排着这么多没编的就丢掉重来 */
const MAX_QUEUE = 10;

export class MicEncoder {
  private port: MessagePort | null = null;
  private encoder: AudioEncoder | null = null;
  private seq = 0;
  /** 下一包是这一段的第一包（刚打开） */
  private first = true;
  /** 这一段的第一个采样的时刻（微秒）：采集时间按采样数往后算，一个会话里只会往前走 */
  private base = 0;
  private frames = 0;
  /** 打好的声音包交给谁（Worker 把它发给控制端服务） */
  private readonly out: (packet: Uint8Array) => void;

  constructor(out: (packet: Uint8Array) => void) {
    this.out = out;
  }

  /** 开始 / 停止（主线程打开、关闭了这个会话的麦克风） */
  setPort(port: MessagePort | null) {
    this.closeEncoder();
    this.port?.close();
    this.port = port;
    if (port) port.onmessage = (e: MessageEvent<Float32Array<ArrayBuffer>>) => this.push(e.data);
  }

  private push(pcm: Float32Array<ArrayBuffer>) {
    if (!this.encoder || this.encoder.state === "closed") this.openEncoder();
    const enc = this.encoder;
    if (!enc) return;
    if (enc.encodeQueueSize > MAX_QUEUE) {
      this.closeEncoder();
      return;
    }
    const timestamp = this.base + Math.round((this.frames * 1e6) / MIC_RATE);
    this.frames += pcm.length;
    const data = new AudioData({
      format: "f32-planar",
      sampleRate: MIC_RATE,
      numberOfChannels: 1,
      numberOfFrames: pcm.length,
      timestamp,
      data: pcm,
    });
    try {
      enc.encode(data);
    } catch {
      this.closeEncoder();
    } finally {
      data.close();
    }
  }

  private openEncoder() {
    const encoder = new AudioEncoder({
      output: (chunk) => this.output(encoder, chunk),
      error: (e) => {
        console.warn("microphone encoder error", e);
        if (this.encoder === encoder) this.closeEncoder();
      },
    });
    try {
      encoder.configure(MIC_CONFIG);
    } catch (e) {
      console.warn("can't configure the microphone encoder", e);
      return;
    }
    this.encoder = encoder;
    this.first = true;
    this.frames = 0;
    // 新的一段从现在算起：比之前的包都晚，被控端按采集时间排先后
    this.base = Math.round((performance.timeOrigin + performance.now()) * 1000);
  }

  private closeEncoder() {
    if (this.encoder && this.encoder.state !== "closed") this.encoder.close();
    this.encoder = null;
  }

  private output(encoder: AudioEncoder, chunk: EncodedAudioChunk) {
    if (this.encoder !== encoder || chunk.byteLength > MAX_OPUS) return;
    const opus = new Uint8Array(chunk.byteLength);
    chunk.copyTo(opus);
    this.seq = (this.seq + 1) >>> 0;
    this.out(audioPacket({ seq: this.seq, start: this.first, captureUs: chunk.timestamp }, opus));
    this.first = false;
  }
}
