//! 性能诊断：根据 CPU / 内存 / 显示器刷新率粗分档，给出渲染与性能模式建议。
//! 天梯分数为经验启发式，非精确跑分。

use jian_config::recommend_target_fps;

/// 诊断结果与推荐配置。
#[derive(Debug, Clone)]
pub struct PerfDiagnosis {
    pub cpu_name: String,
    pub ram_gb: f64,
    /// 主屏当前刷新率（Hz）；读取失败则为 `None`
    pub display_hz: Option<u32>,
    pub gpu_hint: String,
    /// `cpu` | `gpu`
    pub device: String,
    /// `opengl` | `vulkan`
    pub gpu_api: String,
    /// `eco` | `normal` | `extreme`
    pub profile: String,
    /// 建议目标帧率（已按机器预算与显示器刷新率封顶并对齐档位）
    pub target_fps: u32,
    pub summary: String,
}

/// 采集本机信息并给出建议（无 GPU 枚举时用 CPU/内存启发式）。
pub fn diagnose() -> PerfDiagnosis {
    let mut sys = sysinfo::System::new();
    sys.refresh_cpu_all();
    sys.refresh_memory();

    let cpu_name = sys
        .cpus()
        .first()
        .map(|c| c.brand().trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "未知 CPU".into());
    let ram_gb = sys.total_memory() as f64 / (1024.0 * 1024.0 * 1024.0);
    let cpu_score = score_cpu(&cpu_name);
    let ram_score = score_ram(ram_gb);
    let total = cpu_score + ram_score;
    let display_hz = primary_display_hz();

    // 机器预算：能扛住的上限档；最终再与显示器刷新率取交并对齐档位。
    let (device, gpu_api, profile, machine_budget, gpu_hint) =
        if total < 35.0 || ram_gb < 6.0 {
            (
                "cpu",
                "opengl",
                "eco",
                30_u32,
                "未检测到强独显迹象，建议 CPU 软渲染 + 节能".into(),
            )
        } else if total < 70.0 {
            (
                "gpu",
                "opengl",
                "normal",
                90,
                "中端机：GPU + OpenGL，正常模式".into(),
            )
        } else if total < 110.0 {
            (
                "gpu",
                "vulkan",
                "normal",
                165,
                "中高端：GPU + Vulkan，正常模式（有余量可试极致）".into(),
            )
        } else {
            (
                "gpu",
                "vulkan",
                "extreme",
                300,
                "高端：GPU + Vulkan，极致模式（帧率跟主屏刷新率）".into(),
            )
        };

    let target_fps = recommend_target_fps(machine_budget, display_hz);
    let hz_text = match display_hz {
        Some(hz) => format!("主屏刷新率：{hz} Hz"),
        None => "主屏刷新率：未检测到（按 60 Hz 封顶）".into(),
    };

    let summary = format!(
        "CPU：{cpu_name}\n内存：{ram_gb:.1} GB（估分 CPU {cpu_score:.0} / 内存 {ram_score:.0}）\n{hz_text}\n{gpu_hint}\n建议：设备={device}，API={gpu_api}，档位={profile}，帧率={target_fps}\n（设备/GPU 接口需重启；性能档位与帧率立即生效）"
    );

    PerfDiagnosis {
        cpu_name,
        ram_gb,
        display_hz,
        gpu_hint,
        device: device.into(),
        gpu_api: gpu_api.into(),
        profile: profile.into(),
        target_fps,
        summary,
    }
}

fn score_ram(gb: f64) -> f64 {
    if gb < 4.0 {
        5.0
    } else if gb < 8.0 {
        15.0
    } else if gb < 16.0 {
        30.0
    } else if gb < 32.0 {
        45.0
    } else {
        55.0
    }
}

fn score_cpu(name: &str) -> f64 {
    let n = name.to_ascii_lowercase();
    // 粗略天梯：旗舰 > 主流 > 低端；含 Apple / AMD / Intel 常见关键词
    let mut s = 40.0;
    if n.contains("i9") || n.contains("ryzen 9") || n.contains("m3 max") || n.contains("m2 max") {
        s = 95.0;
    } else if n.contains("i7")
        || n.contains("ryzen 7")
        || n.contains("m3 pro")
        || n.contains("m2 pro")
    {
        s = 80.0;
    } else if n.contains("i5")
        || n.contains("ryzen 5")
        || n.contains("m3")
        || n.contains("m2")
        || n.contains("m1")
    {
        s = 60.0;
    } else if n.contains("i3")
        || n.contains("ryzen 3")
        || n.contains("pentium")
        || n.contains("celeron")
        || n.contains("athlon")
    {
        s = 25.0;
    }
    // 代数加成（很粗）
    for (k, bonus) in [
        ("14th", 8.0),
        ("13th", 6.0),
        ("12th", 4.0),
        ("11th", 2.0),
        ("9000", 8.0),
        ("7000", 5.0),
        ("5000", 2.0),
    ] {
        if n.contains(k) {
            s += bonus;
            break;
        }
    }
    s
}

/// 读取主显示器当前刷新率（Hz）。
fn primary_display_hz() -> Option<u32> {
    #[cfg(windows)]
    {
        windows_primary_display_hz()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(windows)]
fn windows_primary_display_hz() -> Option<u32> {
    const ENUM_CURRENT_SETTINGS: u32 = 0xFFFF_FFFF;

    /// 截取到 `dmDisplayFrequency` 的 DEVMODEW 前缀（Unicode）。
    #[repr(C)]
    struct DevModeWFreq {
        dm_device_name: [u16; 32],
        dm_spec_version: u16,
        dm_driver_version: u16,
        dm_size: u16,
        dm_driver_extra: u16,
        dm_fields: u32,
        _union: [u8; 16],
        dm_color: i16,
        dm_duplex: i16,
        dm_y_resolution: i16,
        dm_tt_option: i16,
        dm_collate: i16,
        dm_form_name: [u16; 32],
        dm_log_pixels: u16,
        dm_bits_per_pel: u32,
        dm_pels_width: u32,
        dm_pels_height: u32,
        dm_display_flags: u32,
        dm_display_frequency: u32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn EnumDisplaySettingsW(
            lpsz_device_name: *const u16,
            i_mode_num: u32,
            lp_dev_mode: *mut DevModeWFreq,
        ) -> i32;
        fn GetDC(hwnd: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
        fn ReleaseDC(hwnd: *mut core::ffi::c_void, hdc: *mut core::ffi::c_void) -> i32;
    }
    #[link(name = "gdi32")]
    extern "system" {
        fn GetDeviceCaps(hdc: *mut core::ffi::c_void, index: i32) -> i32;
    }

    let mut mode = unsafe { std::mem::zeroed::<DevModeWFreq>() };
    mode.dm_size = std::mem::size_of::<DevModeWFreq>() as u16;
    let ok = unsafe {
        EnumDisplaySettingsW(std::ptr::null(), ENUM_CURRENT_SETTINGS, &mut mode)
    };
    if ok != 0 {
        let freq = mode.dm_display_frequency;
        if (30..=1000).contains(&freq) {
            return Some(freq);
        }
    }

    // 回退：GetDeviceCaps(VREFRESH)
    const VREFRESH: i32 = 116;
    unsafe {
        let hdc = GetDC(std::ptr::null_mut());
        if hdc.is_null() {
            return None;
        }
        let hz = GetDeviceCaps(hdc, VREFRESH);
        ReleaseDC(std::ptr::null_mut(), hdc);
        if (30..=1000).contains(&(hz as u32)) {
            Some(hz as u32)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use jian_config::{recommend_target_fps, snap_target_fps_at_most};

    #[test]
    fn recommend_respects_display_cap() {
        assert_eq!(snap_target_fps_at_most(240), 240);
        assert_eq!(snap_target_fps_at_most(200), 180);
        assert_eq!(recommend_target_fps(300, Some(240)), 240);
        assert_eq!(recommend_target_fps(60, Some(240)), 60);
        assert_eq!(recommend_target_fps(300, None), 60);
    }

    #[test]
    fn diagnose_reads_local_display() {
        let d = super::diagnose();
        eprintln!(
            "display_hz={:?} target_fps={} profile={}",
            d.display_hz, d.target_fps, d.profile
        );
        assert!(jian_config::TARGET_FPS_PRESETS.contains(&d.target_fps));
    }
}
