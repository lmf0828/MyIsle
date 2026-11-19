//! 实时音频电平（默认渲染端点的峰值），用于驱动频谱随音乐跳动。
//!
//! 通过 WASAPI 的 `IAudioMeterInformation::GetPeakValue` 读取系统正在播放声音的
//! 峰值电平（0.0~1.0），后台线程每约 33ms 采样一次并做轻度平滑，存到原子变量里。
//! 绘制线程用 [`level`] 读取即可。拿不到电平（未播放/无设备）时返回 0。
//!
//! 该模块只做“有没有声音、声音多大”的整体包络，不做频率分解（FFT）。

use std::sync::atomic::{AtomicU32, Ordering};

static LEVEL: AtomicU32 = AtomicU32::new(0);

/// 当前音量峰值（0.0~1.0，已做轻度平滑）。
pub fn level() -> f32 {
    f32::from_bits(LEVEL.load(Ordering::Relaxed)).clamp(0.0, 1.0)
}

/// 启动后台采样线程（进程内只调一次）。
pub fn spawn() {
    std::thread::Builder::new()
        .name("winisle-audio-meter".into())
        .spawn(|| unsafe {
            use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
            use windows::Win32::Media::Audio::{
                eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator,
            };
            use windows::Win32::System::Com::{
                CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
            };

            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

            let mut meter: Option<IAudioMeterInformation> = None;
            let mut backoff = 0u32;
            loop {
                if meter.is_none() {
                    meter = (|| {
                        let en: IMMDeviceEnumerator =
                            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
                        let dev = en.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
                        dev.Activate::<IAudioMeterInformation>(CLSCTX_ALL, None)
                            .ok()
                    })();
                    if meter.is_none() {
                        LEVEL.store(0f32.to_bits(), Ordering::Relaxed);
                    }
                }
                if let Some(m) = &meter {
                    match m.GetPeakValue() {
                        Ok(v) => {
                            let prev = f32::from_bits(LEVEL.load(Ordering::Relaxed));
                            // 上升快、回落稍慢，视觉上更有“跳动感”又不过度抖动
                            let target = v.clamp(0.0, 1.0);
                            let alpha = if target > prev { 0.55 } else { 0.20 };
                            let nv = prev + (target - prev) * alpha;
                            LEVEL.store(nv.to_bits(), Ordering::Relaxed);
                            backoff = 0;
                        }
                        Err(_) => {
                            meter = None;
                        }
                    }
                }
                // 重连退避：拿不到设备时大约每 2 秒重试一次，避免空转
                if meter.is_none() {
                    backoff = backoff.saturating_add(1);
                    if backoff % 60 != 0 {
                        std::thread::sleep(std::time::Duration::from_millis(33));
                        continue;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(33));
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;
    /// 手动探针：需系统正在播放声音。`cargo test --release probe_peak -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn probe_peak() {
        spawn();
        std::thread::sleep(std::time::Duration::from_millis(400));
        let mut mx = 0.0f32;
        for _ in 0..12 {
            let v = level();
            mx = mx.max(v);
            println!("level={:.3}", v);
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        assert!(mx > 0.05, "no audio peak detected (music playing?)");
    }
}
