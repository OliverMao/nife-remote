//! WASAPI 环回采集：录下默认输出设备正在播放的声音（即"系统声音"，不包括麦克风）。
//!
//! 优先让系统把混音格式自动转换为 48 kHz 立体声 float（Windows 10 起支持）；
//! 不支持时按设备混音格式采集，自己做声道下混与线性重采样。

use anyhow::{Context, Result, bail};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
};
use windows::core::GUID;

use std::time::{Duration, Instant};

use nife_remote_proto::media::AUDIO_SAMPLE_RATE;

/// 共享模式缓冲 200 ms：采集线程偶尔被耽搁也不会丢数据
const BUFFER_HNS: i64 = 2_000_000;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
const SUBTYPE_PCM: GUID = GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);
const SUBTYPE_IEEE_FLOAT: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
const SPEAKER_FRONT_LEFT: u32 = 0x1;
const SPEAKER_FRONT_RIGHT: u32 = 0x2;

pub struct Loopback {
    enumerator: IMMDeviceEnumerator,
    device_id: String,
    client: IAudioClient,
    capture: IAudioCaptureClient,
    conv: Converter,
}

impl Loopback {
    pub fn open() -> Result<Self> {
        Self::open_with(true)
    }

    /// `auto_convert=false` 时直接按设备混音格式采集（测试兜底路径用）。
    fn open_with(auto_convert: bool) -> Result<Self> {
        unsafe {
            // 每个线程都需要初始化 COM（重复调用是安全的）
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .context("can't create MMDeviceEnumerator")?;
            let device = enumerator
                .GetDefaultAudioEndpoint(eRender, eConsole)
                .context("no default audio output device")?;
            let device_id = device_id(&device)?;

            let want = float_stereo(AUDIO_SAMPLE_RATE);
            let want_ptr = (&raw const want).cast::<WAVEFORMATEX>();
            let flags = AUDCLNT_STREAMFLAGS_LOOPBACK;
            let auto = flags
                | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
            let init = if auto_convert {
                client.Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    auto,
                    BUFFER_HNS,
                    0,
                    want_ptr,
                    None,
                )
            } else {
                Err(windows::core::Error::empty())
            };
            let (client, conv) = match init {
                Ok(()) => (client, Converter::new(Format::native())),
                Err(e) => {
                    tracing::debug!(
                        "system format conversion unavailable ({e}), using the device mix format"
                    );
                    // 初始化失败后这个 IAudioClient 不能再用，重新激活一个
                    let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
                    let mix = client.GetMixFormat()?;
                    let fmt = Format::parse(mix);
                    let init = client.Initialize(
                        AUDCLNT_SHAREMODE_SHARED,
                        flags,
                        BUFFER_HNS,
                        0,
                        mix,
                        None,
                    );
                    CoTaskMemFree(Some(mix as *const _));
                    let fmt = fmt?;
                    init.context("can't initialize audio capture")?;
                    (client, Converter::new(fmt))
                }
            };
            let capture: IAudioCaptureClient = client.GetService()?;
            client.Start()?;
            tracing::info!(
                rate = conv.fmt.rate,
                channels = conv.fmt.channels,
                kind = ?conv.fmt.kind,
                "audio capture opened"
            );
            Ok(Self {
                enumerator,
                device_id,
                client,
                capture,
                conv,
            })
        }
    }

    /// 读出当前已采集的全部数据（48 kHz 立体声交错 float），追加到 `out`。没有数据时立即返回。
    ///
    /// 返回追加的第一个采样的采集时间（系统给出的 QPC 时间戳；不可用时为 `None`）。
    pub fn read(&mut self, out: &mut Vec<f32>) -> Result<Option<Instant>> {
        let base = out.len();
        let mut first: Option<Instant> = None;
        unsafe {
            loop {
                let n = self.capture.GetNextPacketSize()?;
                if n == 0 {
                    return Ok(first);
                }
                let mut data = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                let mut qpc = 0u64;
                self.capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, Some(&mut qpc))?;
                // 用这一包的时间戳倒推本次读取的起点（前面的包可能没有可用的时间戳）
                if first.is_none() && flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 == 0 {
                    let before = Duration::from_secs_f64(
                        (out.len() - base) as f64 / 2.0 / AUDIO_SAMPLE_RATE as f64,
                    );
                    first =
                        crate::host::clock::from_qpc_100ns(qpc).and_then(|t| t.checked_sub(before));
                }
                let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null();
                let bytes = if silent {
                    None
                } else {
                    Some(std::slice::from_raw_parts(
                        data,
                        frames as usize * self.conv.fmt.block_align,
                    ))
                };
                self.conv.push(bytes, frames as usize, out);
                self.capture.ReleaseBuffer(frames)?;
            }
        }
    }

    /// 默认输出设备是否已经换了（或者没了）。
    pub fn default_changed(&self) -> bool {
        unsafe {
            match self.enumerator.GetDefaultAudioEndpoint(eRender, eConsole) {
                Ok(d) => device_id(&d).map_or(true, |id| id != self.device_id),
                Err(_) => true,
            }
        }
    }
}

impl Drop for Loopback {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
        }
    }
}

fn device_id(device: &IMMDevice) -> Result<String> {
    unsafe {
        let p = device.GetId()?;
        let id = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as *const _));
        Ok(id)
    }
}

fn float_stereo(rate: u32) -> WAVEFORMATEXTENSIBLE {
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_EXTENSIBLE,
            nChannels: 2,
            nSamplesPerSec: rate,
            nAvgBytesPerSec: rate * 8,
            nBlockAlign: 8,
            wBitsPerSample: 32,
            cbSize: (size_of::<WAVEFORMATEXTENSIBLE>() - size_of::<WAVEFORMATEX>()) as u16,
        },
        Samples: WAVEFORMATEXTENSIBLE_0 {
            wValidBitsPerSample: 32,
        },
        dwChannelMask: SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT,
        SubFormat: SUBTYPE_IEEE_FLOAT,
    }
}

// ───────────────────────────── 格式转换 ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum SampleKind {
    F32,
    I16,
    I24,
    I32,
}

#[derive(Debug, Clone, Copy)]
struct Format {
    kind: SampleKind,
    channels: usize,
    rate: u32,
    block_align: usize,
}

impl Format {
    fn native() -> Self {
        Self {
            kind: SampleKind::F32,
            channels: 2,
            rate: AUDIO_SAMPLE_RATE,
            block_align: 8,
        }
    }

    unsafe fn parse(p: *const WAVEFORMATEX) -> Result<Self> {
        let f = unsafe { std::ptr::read_unaligned(p) };
        let bits = f.wBitsPerSample;
        let float = match f.wFormatTag {
            WAVE_FORMAT_EXTENSIBLE if f.cbSize >= 22 => {
                let x = unsafe { std::ptr::read_unaligned(p as *const WAVEFORMATEXTENSIBLE) };
                let sub = x.SubFormat;
                if sub == SUBTYPE_IEEE_FLOAT {
                    true
                } else if sub == SUBTYPE_PCM {
                    false
                } else {
                    bail!("unsupported mix format {sub:?}");
                }
            }
            WAVE_FORMAT_IEEE_FLOAT => true,
            1 => false,
            tag => bail!("unsupported mix format 0x{tag:x}"),
        };
        let kind = match (float, bits) {
            (true, 32) => SampleKind::F32,
            (false, 16) => SampleKind::I16,
            (false, 24) => SampleKind::I24,
            (false, 32) => SampleKind::I32,
            _ => bail!("unsupported sample format ({bits} bits, float={float})"),
        };
        let channels = f.nChannels as usize;
        let block_align = f.nBlockAlign as usize;
        if channels == 0 || f.nSamplesPerSec == 0 || block_align < channels * bits as usize / 8 {
            bail!("invalid mix format");
        }
        Ok(Self {
            kind,
            channels,
            rate: f.nSamplesPerSec,
            block_align,
        })
    }
}

/// 任意 PCM → 48 kHz 立体声 float。
struct Converter {
    fmt: Format,
    /// 输入采样率 / 48000
    step: f64,
    /// 下一个输出采样在输入时间轴上的位置（0 = 上一块的最后一帧）
    pos: f64,
    prev: [f32; 2],
    tmp: Vec<[f32; 2]>,
}

impl Converter {
    fn new(fmt: Format) -> Self {
        Self {
            fmt,
            step: fmt.rate as f64 / AUDIO_SAMPLE_RATE as f64,
            pos: 1.0,
            prev: [0.0; 2],
            tmp: Vec::new(),
        }
    }

    fn sample(&self, b: &[u8]) -> f32 {
        match self.fmt.kind {
            SampleKind::F32 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            SampleKind::I16 => i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0,
            SampleKind::I24 => {
                (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0
            }
            SampleKind::I32 => {
                i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0
            }
        }
    }

    /// 一帧多声道 → 立体声（按 WAVE 声道顺序：FL FR FC LFE BL BR SL SR）。
    fn stereo(&self, frame: &[u8]) -> [f32; 2] {
        let size = match self.fmt.kind {
            SampleKind::I16 => 2,
            SampleKind::I24 => 3,
            _ => 4,
        };
        let ch = |i: usize| self.sample(&frame[i * size..]);
        match self.fmt.channels {
            1 => {
                let m = ch(0);
                [m, m]
            }
            2 => [ch(0), ch(1)],
            n => {
                const K: f32 = std::f32::consts::FRAC_1_SQRT_2;
                let (mut l, mut r, mut norm) = (ch(0), ch(1), 1.0);
                if n >= 3 {
                    let c = ch(2) * K;
                    l += c;
                    r += c;
                    norm += K;
                }
                if n >= 6 {
                    l += ch(4) * K;
                    r += ch(5) * K;
                    norm += K;
                }
                if n >= 8 {
                    l += ch(6) * K;
                    r += ch(7) * K;
                    norm += K;
                }
                [l / norm, r / norm]
            }
        }
    }

    /// `data` 为 `None` 表示这一段是静音。
    fn push(&mut self, data: Option<&[u8]>, frames: usize, out: &mut Vec<f32>) {
        if frames == 0 {
            return;
        }
        let native = self.fmt.kind == SampleKind::F32
            && self.fmt.channels == 2
            && self.fmt.rate == AUDIO_SAMPLE_RATE;
        if native {
            match data {
                Some(b) => out.extend(b.as_chunks::<4>().0.iter().map(|s| f32::from_le_bytes(*s))),
                None => out.resize(out.len() + frames * 2, 0.0),
            }
            return;
        }
        let mut tmp = std::mem::take(&mut self.tmp);
        tmp.clear();
        match data {
            Some(b) => tmp.extend(
                b.chunks_exact(self.fmt.block_align)
                    .take(frames)
                    .map(|f| self.stereo(f)),
            ),
            None => tmp.resize(frames, [0.0; 2]),
        }
        if self.fmt.rate == AUDIO_SAMPLE_RATE {
            out.extend(tmp.iter().flatten());
        } else {
            // 线性插值重采样：位置 0 是上一块的最后一帧，1..=n 是本块
            let n = tmp.len();
            let at = |i: usize| if i == 0 { self.prev } else { tmp[i - 1] };
            loop {
                let i = self.pos as usize;
                if i >= n {
                    break;
                }
                let f = (self.pos - i as f64) as f32;
                let (a, b) = (at(i), at(i + 1));
                out.push(a[0] + (b[0] - a[0]) * f);
                out.push(a[1] + (b[1] - a[1]) * f);
                self.pos += self.step;
            }
            self.pos -= n as f64;
            self.prev = tmp[n - 1];
        }
        self.tmp = tmp;
    }
}
