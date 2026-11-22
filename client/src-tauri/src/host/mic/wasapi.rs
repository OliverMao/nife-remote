//! Windows 的虚拟麦克风：Steam Streaming Microphone（Steam 远程同乐的虚拟声卡驱动，Valve 的、微软签过名的）。
//!
//! 它有一对设备：播放设备"扬声器 (Steam Streaming Microphone)"和录音设备"麦克风 (Steam Streaming Microphone)"，
//! 写进播放设备的字节**原样**从录音设备出来。驱动不换格式，所以两边的格式必须一样，否则录到的声音会变调
//! （实测播放端 48 kHz 立体声、录音端 44.1 kHz 单声道时，1 kHz 的声音录出来是 459 Hz = 1000 × 44100 / 96000）：
//!
//! * 两个设备的设备格式（声音设置里的"默认格式"）一样：共享模式，系统把声音换成那个格式；
//! * 不一样（这台电脑上就是）：独占模式，直接按录音端的格式写进播放端。不改系统的声音设置，
//!   独占的只是这个虚拟设备，而且只在控制端打开麦克风期间（Steam 远程同乐这时用不了它）。
//!
//! 设备按接口名（"Steam Streaming Microphone"）找，不按显示名：显示名前面的"扬声器""麦克风"随系统语言变。
//! 驱动不在、或者被禁用了就是没有（`MicState::NoDevice`）。

use anyhow::{Context, Result, bail};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::StructuredStorage::PropVariantClear;
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, STGM_READ,
};
use windows::Win32::System::Variant::{VT_BLOB, VT_LPWSTR};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::core::GUID;

use nife_remote_proto::media::AUDIO_SAMPLE_RATE;
use nife_remote_proto::session::MicState;

use super::{Format, Output, Sample};

/// 设备的接口名（`PKEY_DeviceInterface_FriendlyName`，不随系统语言变）
const STEAM_MIC: &str = "Steam Streaming Microphone";
const PKEY_DEVICE_INTERFACE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x026e516e_b814_414b_83cd_856d6fef4822),
    pid: 2,
};
/// 共享模式的缓冲：100 ms
const SHARED_BUFFER_HNS: i64 = 1_000_000;
/// 独占模式的缓冲：40 ms（写得越多延迟越大，播放线程每 5 ms 补一次）
const EXCLUSIVE_BUFFER_HNS: i64 = 400_000;

/// 找到的一对设备
struct Pair {
    render: IMMDevice,
    /// 两边的设备格式（`WAVEFORMATEX` 的字节）
    render_format: Vec<u8>,
    capture_format: Vec<u8>,
}

/// 有没有 Steam Streaming Microphone（不打开）
pub fn probe() -> MicState {
    match find() {
        Ok(Some(_)) => MicState::Ready,
        Ok(None) => MicState::NoDevice,
        Err(e) => {
            tracing::warn!("can't look for the virtual microphone: {e:#}");
            MicState::Failed
        }
    }
}

/// 打开播放端（见模块说明：两边格式一样用共享模式，不一样用独占模式按录音端的格式写）
pub fn open() -> Result<Option<Box<dyn Output>>> {
    let Some(pair) = find()? else {
        return Ok(None);
    };
    let capture = Format::parse(&pair.capture_format).context("the microphone's format")?;
    let render = Format::parse(&pair.render_format).context("the speaker side's format")?;
    let same = render.sample == capture.sample
        && render.channels == capture.channels
        && render.rate == capture.rate;
    unsafe {
        let out = if same {
            shared(&pair.render)?
        } else {
            exclusive(&pair.render, &pair.capture_format, capture)?
        };
        out.client.Start()?;
        tracing::info!(
            shared = same,
            ?render,
            ?capture,
            "writing into Steam Streaming Microphone"
        );
        Ok(Some(Box::new(out)))
    }
}

fn find() -> Result<Option<Pair>> {
    unsafe {
        // 每个线程都要初始化 COM（重复调用没关系）
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
            .context("can't create MMDeviceEnumerator")?;
        let (Some((render, render_format)), Some((_, capture_format))) =
            (find_flow(&en, eRender)?, find_flow(&en, eCapture)?)
        else {
            return Ok(None);
        };
        Ok(Some(Pair {
            render,
            render_format,
            capture_format,
        }))
    }
}

/// 一个方向上的 Steam Streaming Microphone 和它的设备格式
unsafe fn find_flow(
    en: &IMMDeviceEnumerator,
    flow: EDataFlow,
) -> Result<Option<(IMMDevice, Vec<u8>)>> {
    unsafe {
        let all = en.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
        for i in 0..all.GetCount()? {
            let Ok(device) = all.Item(i) else { continue };
            let Ok(store) = device.OpenPropertyStore(STGM_READ) else {
                continue;
            };
            if string_prop(&store, &PKEY_DEVICE_INTERFACE_FRIENDLY_NAME).as_deref()
                != Some(STEAM_MIC)
            {
                continue;
            }
            let format = blob_prop(&store, &PKEY_AudioEngine_DeviceFormat)
                .context("Steam Streaming Microphone has no device format")?;
            return Ok(Some((device, format)));
        }
        Ok(None)
    }
}

unsafe fn string_prop(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    unsafe {
        let mut v = store.GetValue(key).ok()?;
        let s = (v.Anonymous.Anonymous.vt == VT_LPWSTR)
            .then(|| v.Anonymous.Anonymous.Anonymous.pwszVal.to_string().ok())
            .flatten();
        let _ = PropVariantClear(&mut v);
        s
    }
}

unsafe fn blob_prop(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<Vec<u8>> {
    unsafe {
        let mut v = store.GetValue(key).ok()?;
        let b = (v.Anonymous.Anonymous.vt == VT_BLOB).then(|| {
            let blob = &v.Anonymous.Anonymous.Anonymous.blob;
            if blob.pBlobData.is_null() {
                Vec::new()
            } else {
                std::slice::from_raw_parts(blob.pBlobData, blob.cbSize as usize).to_vec()
            }
        });
        let _ = PropVariantClear(&mut v);
        b.filter(|b| !b.is_empty())
    }
}

/// 写进播放端
struct Wasapi {
    client: IAudioClient,
    render: IAudioRenderClient,
    /// 缓冲多少帧
    size: u32,
    fmt: Format,
}

/// 共享模式：交给系统的是 48 kHz 单声道 float，系统换成设备格式
unsafe fn shared(device: &IMMDevice) -> Result<Wasapi> {
    unsafe {
        let fmt = Format {
            sample: Sample::F32,
            channels: 1,
            rate: AUDIO_SAMPLE_RATE,
            block_align: 4,
        };
        let wave = WAVEFORMATEX {
            wFormatTag: 3, // WAVE_FORMAT_IEEE_FLOAT
            nChannels: 1,
            nSamplesPerSec: AUDIO_SAMPLE_RATE,
            nAvgBytesPerSec: AUDIO_SAMPLE_RATE * 4,
            nBlockAlign: 4,
            wBitsPerSample: 32,
            cbSize: 0,
        };
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                SHARED_BUFFER_HNS,
                0,
                &wave,
                None,
            )
            .context("can't open Steam Streaming Microphone (shared)")?;
        finish(client, fmt)
    }
}

/// 独占模式：按录音端的设备格式（`wave`，解析出来是 `fmt`）直接写进播放端
unsafe fn exclusive(device: &IMMDevice, wave: &[u8], fmt: Format) -> Result<Wasapi> {
    unsafe {
        let ptr = wave.as_ptr().cast::<WAVEFORMATEX>();
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        let client = match client.Initialize(
            AUDCLNT_SHAREMODE_EXCLUSIVE,
            0,
            EXCLUSIVE_BUFFER_HNS,
            0,
            ptr,
            None,
        ) {
            Ok(()) => client,
            // 缓冲的大小要对齐到设备的要求：按它给的大小重新来一次（初始化失败的 IAudioClient 不能再用）
            Err(e) if e.code() == AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED => {
                let frames = client.GetBufferSize()?;
                let hns = (10_000_000.0 * frames as f64 / fmt.rate as f64).round() as i64;
                let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
                client.Initialize(AUDCLNT_SHAREMODE_EXCLUSIVE, 0, hns, 0, ptr, None)?;
                client
            }
            Err(e) if e.code() == AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED => bail!(
                "exclusive mode is turned off for Steam Streaming Microphone: allow it in the sound settings, \
                 or give its speaker and microphone sides the same default format ({e})"
            ),
            Err(e) if e.code() == AUDCLNT_E_DEVICE_IN_USE => {
                bail!(
                    "Steam Streaming Microphone is in use by another program (Steam Remote Play?): {e}"
                )
            }
            Err(e) => return Err(e).context("can't open Steam Streaming Microphone (exclusive)"),
        };
        finish(client, fmt)
    }
}

unsafe fn finish(client: IAudioClient, fmt: Format) -> Result<Wasapi> {
    unsafe {
        let size = client.GetBufferSize()?;
        tracing::debug!(frames = size, rate = fmt.rate, "virtual microphone buffer");
        let render: IAudioRenderClient = client.GetService()?;
        Ok(Wasapi {
            client,
            render,
            size,
            fmt,
        })
    }
}

impl Output for Wasapi {
    fn format(&self) -> Format {
        self.fmt
    }

    fn buffered(&mut self) -> Result<(usize, usize)> {
        let padding = unsafe { self.client.GetCurrentPadding()? };
        Ok((padding as usize, self.size as usize))
    }

    fn write(&mut self, frames: usize, fill: &mut dyn FnMut(&mut [u8])) -> Result<()> {
        unsafe {
            let p = self.render.GetBuffer(frames as u32)?;
            let buf = std::slice::from_raw_parts_mut(p, frames * self.fmt.block_align);
            fill(buf);
            self.render.ReleaseBuffer(frames as u32, 0)?;
        }
        Ok(())
    }
}

impl Drop for Wasapi {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
        }
    }
}


