//! 性能诊断：根据 CPU / 内存粗分档，给出渲染与性能模式建议。
//! 天梯分数为经验启发式，非精确跑分。

/// 诊断结果与推荐配置。
#[derive(Debug, Clone)]
pub struct PerfDiagnosis {
    pub cpu_name: String,
    pub ram_gb: f64,
    pub gpu_hint: String,
    /// `cpu` | `gpu`
    pub device: String,
    /// `opengl` | `vulkan`
    pub gpu_api: String,
    /// `eco` | `normal` | `extreme`
    pub profile: String,
    /// 建议目标帧率
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

    // 无独立 GPU 枚举时：低配偏 CPU 软渲染；中高配 GPU+Vulkan
    let (device, gpu_api, profile, target_fps, gpu_hint) = if total < 35.0 || ram_gb < 6.0 {
        (
            "cpu",
            "opengl",
            "eco",
            20_u32,
            "未检测到强独显迹象，建议 CPU 软渲染 + 节能 + 20fps".into(),
        )
    } else if total < 70.0 {
        (
            "gpu",
            "opengl",
            "normal",
            30,
            "中端机：GPU + OpenGL，正常模式 + 30fps".into(),
        )
    } else if total < 110.0 {
        (
            "gpu",
            "vulkan",
            "normal",
            30,
            "中高端：GPU + Vulkan，正常模式 + 30fps".into(),
        )
    } else {
        (
            "gpu",
            "vulkan",
            "extreme",
            60,
            "高端：GPU + Vulkan，极致模式 + 60fps".into(),
        )
    };

    let summary = format!(
        "CPU：{cpu_name}\n内存：{ram_gb:.1} GB（估分 CPU {cpu_score:.0} / 内存 {ram_score:.0}）\n{gpu_hint}\n建议：设备={device}，API={gpu_api}，档位={profile}，帧率={target_fps}\n（设备/GPU 接口需重启；性能档位与帧率立即生效）"
    );

    PerfDiagnosis {
        cpu_name,
        ram_gb,
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
    } else if n.contains("i7") || n.contains("ryzen 7") || n.contains("m3 pro") || n.contains("m2 pro") {
        s = 80.0;
    } else if n.contains("i5") || n.contains("ryzen 5") || n.contains("m3") || n.contains("m2") || n.contains("m1") {
        s = 60.0;
    } else if n.contains("i3") || n.contains("ryzen 3") || n.contains("pentium") || n.contains("celeron") || n.contains("athlon") {
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
