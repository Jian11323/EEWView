//! EEW View（地震视监器）桌面客户端入口。

// GUI 子系统：无控制台窗口；logging 永不 AllocConsole，仅可选写 stderr。
#![windows_subsystem = "windows"]

mod logging;

use anyhow::Result;
use eframe::egui;
use jian_audio::{speak_async, AudioCue, AudioEngine, TsunamiCueKind};
use jian_config::AppConfig;
use jian_core::{
    agency_family, build_eew_tts_script, build_record_tts_script, eew_sidebar_entries,
    cinema_event_key, dedupe_same_eew_events, eew_sidebar_group_key, eew_wave_active_spread,
    is_wave_candidate, max_s_spread_km, origin_is_stale, same_eew_event, should_play_eew_tts,
    AgencyFamily,
    AppSnapshot, EewReport, HealthStatus, IntensityKind,
};
use std::collections::HashMap;
use jian_net::{load_place_fix, spawn_hub_with_assets, NetEvent, NetHub};
use jian_travel::{s_wave_still_spreading_to, TravelEngine};
use jian_map::{zoom_to_fit_radius_km, BasemapMode, ProjectionMode};
use jian_ui::{draw_splash, MainShell, SettingsAction, SplashGate, AppUiTheme};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::runtime::Runtime;
use tracing_appender::non_blocking::WorkerGuard;

fn looks_like_resource_root(dir: &Path) -> bool {
    dir.join("media").is_dir()
        && dir.join("icon").is_dir()
        && (dir.join("geodata").is_dir() || dir.join("config").is_dir())
}

/// 发行包：exe 同目录；开发：`cargo run` 时沿路径找到仓库根。可用 `EEWVIEW_ROOT` 覆盖。
fn resource_root() -> PathBuf {
    if let Ok(p) = std::env::var("EEWVIEW_ROOT") {
        let p = PathBuf::from(p);
        if p.is_dir() {
            return p;
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(Path::to_path_buf);
        for _ in 0..6 {
            let Some(d) = dir else { break };
            if looks_like_resource_root(&d) {
                return d;
            }
            dir = d.parent().map(Path::to_path_buf);
        }
    }
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.pop(); // crates
    dir.pop(); // repo root
    dir
}

fn media_root() -> PathBuf {
    resource_root().join("media")
}

fn resolve_geodata_dir(configured: &str) -> PathBuf {
    let p = Path::new(configured);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        resource_root().join(p)
    }
}

/// 嵌入霞鹜文楷作 CJK 回退。拉丁走 egui 默认字体，汉字才回退文楷（避免文楷当主字体撑爆字形图集）。
/// 使用 from_static：字体页落在映像映射里，避免 from_owned 再堆一份 24MB。
fn install_cjk_fonts(ctx: &egui::Context) {
    const LXGW_WENKAI: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../media/fonts/lxgw-wenkai-v1.522/LXGWWenKai-Regular.ttf"
    ));

    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "lxgw_wenkai".to_string(),
        egui::FontData::from_static(LXGW_WENKAI).into(),
    );
    fonts.families.insert(
        egui::FontFamily::Name("lxgw_wenkai".into()),
        vec!["lxgw_wenkai".to_string()],
    );
    if let Some(fam) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        if !fam.iter().any(|n| n == "lxgw_wenkai") {
            fam.push("lxgw_wenkai".to_string());
        }
    }
    if let Some(fam) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
        if !fam.iter().any(|n| n == "lxgw_wenkai") {
            fam.push("lxgw_wenkai".to_string());
        }
    }
    ctx.set_fonts(fonts);
    tracing::info!(bytes = LXGW_WENKAI.len(), "embedded LXGW WenKai as CJK fallback");
}

fn window_title_for_lang(language: &str) -> String {
    let monitor = match jian_config::normalize_ui_language(language).as_str() {
        "zh-TW" => "地震視監器",
        "ja" => "地震モニター",
        "en" => "Quake Monitor",
        _ => "地震视监器",
    };
    format!(
        "{} \u{00b7} {}  [Beta] v{}",
        jian_ui::APP_DISPLAY_NAME,
        monitor,
        env!("CARGO_PKG_VERSION")
    )
}

fn main() {
    if let Err(e) = run_app() {
        let msg = format!("{e:#}");
        logging::show_fatal_dialog("EEW View", &format!("启动失败：\n{msg}"));
        std::process::exit(1);
    }
}

fn run_app() -> Result<()> {
    run_app_inner(false)
}

fn gl_context_unsupported(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    e.contains("es context") || e.contains("wgl")
}

#[cfg(test)]
mod gl_fallback_tests {
    use super::gl_context_unsupported;

    #[test]
    fn detects_wgl_es_failure() {
        let log = "glutin error: extension to create ES context with wgl is not present";
        assert!(gl_context_unsupported(log));
        assert!(!gl_context_unsupported("eframe: other"));
    }
}

fn run_app_inner(force_soft: bool) -> Result<()> {
    let root = resource_root();
    let media = media_root();
    let cfg = AppConfig::load_merged(root.join("config/default.toml"));
    let mut cfg = cfg;
    cfg.ui.language = jian_config::normalize_ui_language(&cfg.ui.language);
    jian_ui::set_lang_from_config(&cfg.ui.language);
    let log_guard = logging::init_logging(&root, &cfg.log);
    tracing::info!(
        root = %root.display(),
        version = env!("CARGO_PKG_VERSION"),
        "resource root"
    );
    let basemap = BasemapMode::resolve(&cfg.map.basemap, &cfg.ui.theme);
    let geodata_dir = resolve_geodata_dir(&cfg.map.geodata_dir);
    tracing::info!(
        event_pack = %cfg.audio.event_pack,
        theme = %cfg.ui.theme,
        basemap = basemap.as_str(),
        geodata = %geodata_dir.display(),
        tile = %format!("{}/{}", cfg.map.tile_base, cfg.map.tile_source),
        jian = cfg.sources.jian.enabled,
        wolfx = cfg.sources.wolfx.enabled,
        p2p = cfg.sources.p2pquake.enabled,
        fssn = cfg.sources.fssn.enabled,
        jian_at = cfg
            .sources
            .jian
            .access_token
            .as_ref()
            .map(|s| s.trim().starts_with("at_"))
            .unwrap_or(false),
        jian_rt = cfg
            .sources
            .jian
            .refresh_token
            .as_ref()
            .map(|s| s.trim().starts_with("rt_"))
            .unwrap_or(false),
        "config ready"
    );

    let travel = match TravelEngine::load_named(
        media.join("travel"),
        &cfg.travel.primary,
        &cfg.travel.fallback,
        cfg.travel.linear_last_resort,
    ) {
        Ok(eng) => {
            let t = eng.travel_time(jian_travel::Wave::S, 40.0, 100.0);
            tracing::info!(
                primary = %cfg.travel.primary,
                s_travel_40km_100km_s = t,
                "travel table loaded"
            );
            Some(eng)
        }
        Err(e) => {
            tracing::warn!("走时表未加载（先 cargo run -p convert_travel_tables）: {e}");
            None
        }
    };

    load_place_fix(media.join("place"));

    let audio = match AudioEngine::from_catalog(
        media.join("sound/catalog.json"),
        media.join("sound"),
        &cfg.audio.event_pack,
        &cfg.audio.countdown_pack,
        cfg.audio.master_volume,
        cfg.audio.countdown_volume,
        cfg.audio.mute,
    ) {
        Ok(a) => Some(a),
        Err(e) => {
            tracing::warn!("音效引擎未启动: {e:#}");
            None
        }
    };

    let live = cfg.any_source_enabled();
    let mut snap = AppSnapshot::empty();
    if live {
        tracing::info!("waiting for source events");
    } else {
        tracing::info!("all sources disabled");
    }

    let rt = Runtime::new()?;
    let net_rx = {
        let _enter = rt.enter();
        let stations_dir = media.join("stations");
        let mut hub = spawn_hub_with_assets(
            &cfg.sources,
            stations_dir.is_dir().then_some(stations_dir),
        );
        hub.set_weather_overlay(cfg.ui.show_global_radar, cfg.ui.show_global_cloud);
        Some(hub)
    };
    let _ = live;

    refresh_countdown(
        &mut snap,
        travel.as_ref(),
        cfg.local.latitude,
        cfg.local.longitude,
        None,
    );

    let mut shell = MainShell::default();
    shell.show_nied_clock = cfg.ui.show_nied_clock;
    shell.intensity_scale = cfg.ui.intensity_scale.clone();
    shell.local_lat = cfg.local.latitude;
    shell.local_lon = cfg.local.longitude;
    shell.configure_map(
        basemap,
        cfg.map.tile_base.clone(),
        cfg.map.tile_source.clone(),
        Some(geodata_dir.as_path()),
        cfg.map.default_lon,
        cfg.map.default_lat,
        cfg.map.default_zoom,
        ProjectionMode::parse(&cfg.map.projection),
        cfg.ui.show_faults,
    );
    shell
        .map
        .set_tile_capacity(jian_util::perf::recommended_tile_slots(
            cfg.performance.device.eq_ignore_ascii_case("cpu"),
        ));
    snap.tab = match cfg.ui.sidebar_default_tab.to_ascii_lowercase().as_str() {
        "records" | "record" => jian_core::SidebarTab::Records,
        "station" => jian_core::SidebarTab::Station,
        "volcano" => jian_core::SidebarTab::Volcano,
        _ => jian_core::SidebarTab::Eew,
    };
    if let Some(ev) = &snap.active {
        shell.map.center_lon = ev.longitude;
        shell.map.center_lat = ev.latitude;
    }

    let win = &cfg.window;
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([
            win.width.clamp(640.0, 7680.0),
            win.height.clamp(480.0, 4320.0),
        ])
        .with_title(window_title_for_lang(&cfg.ui.language));
    if let (Some(x), Some(y)) = (win.x, win.y) {
        viewport = viewport.with_position([x, y]);
    }
    if win.maximized {
        viewport = viewport.with_maximized(true);
    }
    if let Some(icon) = jian_ui::app_icon_data() {
        viewport = viewport.with_icon(icon);
    }
    if cfg.ui.always_on_top {
        viewport = viewport.with_always_on_top();
    }

    // 固定 OpenGL（Glow）。WGL 无法创建 ES 上下文时自动改软渲染再试一次。
    let options = build_native_options(viewport, &cfg.performance, force_soft);

    let started = Instant::now();
    let has_local = cfg.local.latitude.is_some() && cfg.local.longitude.is_some();
    snap.show_wave_rings = cfg.ui.show_wave_rings;
    let run_err = eframe::run_native(
        jian_ui::APP_DISPLAY_NAME,
        options,
        Box::new(move |cc| {
            install_cjk_fonts(&cc.egui_ctx);
            // 强制嵌入式 viewport，防止历史 set_embed_viewports(false) 残留逻辑
            cc.egui_ctx.set_embed_viewports(true);
            Ok(Box::new(JianApp {
                _log_guard: log_guard,
                resource_root: root,
                snap,
                shell,
                travel,
                started,
                local_lat: cfg.local.latitude,
                local_lon: cfg.local.longitude,
                has_local,
                geodata_dir,
                media_dir: media,
                cfg,
                rt,
                net_rx,
                audio,
                auto_center_pending: live,
                last_follow_key: None,
                last_record_follow_key: None,
                last_eew_tts: HashMap::new(),
                last_eew_tts_script: HashMap::new(),
                cinema_idx: 0,
                cinema_next_at: Instant::now(),
                cinema_seen_key: String::new(),
                prev_health: [HealthStatus::Abnormal; 5],
                prev_tsunami_key: String::new(),
                window_geom_save_at: None,
                last_window_geom: None,
                splash: SplashGate::default(),
            }))
        }),
    );
    if let Err(e) = run_err {
        let msg = e.to_string();
        if !force_soft && gl_context_unsupported(&msg) {
            tracing::warn!("OpenGL ES/WGL 不可用，回退 CPU 软渲染后重试");
            return run_app_inner(true);
        }
        return Err(anyhow::anyhow!("eframe: {msg}"));
    }
    Ok(())
}

fn build_native_options(
    viewport: egui::ViewportBuilder,
    perf: &jian_config::PerformanceConfig,
    force_soft: bool,
) -> eframe::NativeOptions {
    let device = perf.device.to_ascii_lowercase();
    let profile = perf.profile.to_ascii_lowercase();
    let hw = if force_soft || device == "cpu" {
        eframe::HardwareAcceleration::Off
    } else {
        eframe::HardwareAcceleration::Preferred
    };
    eframe::NativeOptions {
        viewport,
        vsync: profile != "extreme",
        hardware_acceleration: hw,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    }
}

/// 按忙碌类型选重绘间隔，避免老机器空载/软忙仍按目标帧率烧 CPU/GPU。
#[derive(Clone, Copy)]
enum RepaintNeed {
    /// 拖拽/飞跃/启动画面：跟手
    Interactive,
    /// 波圈动画
    Waves,
    /// 设置页打开或倒计时秒显
    Soft,
    /// 完全空闲
    Idle,
}

fn repaint_ms(perf: &jian_config::PerformanceConfig, need: RepaintNeed) -> u64 {
    match need {
        // 跟手/飞跃：严格按用户目标帧率（60→约17ms；不再被波圈档位二次压低）
        RepaintNeed::Interactive => perf.repaint_interval_ms(),
        RepaintNeed::Waves => perf.wave_repaint_ms(),
        RepaintNeed::Soft => perf.soft_repaint_ms(),
        RepaintNeed::Idle => perf.idle_repaint_ms(),
    }
}

fn clear_countdown(snap: &mut AppSnapshot) {

    snap.countdown_s = None;
    snap.countdown_local_level = 0;
    snap.countdown_local_text.clear();
}

fn refresh_countdown(
    snap: &mut AppSnapshot,
    travel: Option<&TravelEngine>,
    local_lat: Option<f64>,
    local_lon: Option<f64>,
    elapsed: Option<f64>,
) {
    let (Some(eng), Some(lat), Some(lon), Some(ev)) =
        (travel, local_lat, local_lon, snap.active.as_ref())
    else {
        if local_lat.is_none() || local_lon.is_none() || snap.active.is_none() {
            clear_countdown(snap);
        }
        return;
    };
    // 调查中（仮定震源）：对齐要石，不播/不显本地倒计时
    if ev.is_cancel || ev.is_assumption || (ev.origin_ms <= 0 && elapsed.is_none()) {
        clear_countdown(snap);
        return;
    }
    let elapsed_s = if let Some(e) = elapsed {
        e
    } else if ev.origin_ms > 0 {
        let now_ms = chrono::Utc::now().timestamp_millis();
        ((now_ms - ev.origin_ms) as f64 / 1000.0).max(0.0)
    } else {
        0.0
    };
    // PS 波走完最大传播距离后停止并隐藏（与地图波圈 / 左上预警条同一条件）
    let max_km = max_s_spread_for(ev.magnitude, &snap.wave_spread_km);
    if !s_wave_still_spreading_to(eng, ev.depth_km, elapsed_s, max_km) {
        clear_countdown(snap);
        return;
    }
    let surface = jian_util::haversine_km(ev.longitude, ev.latitude, lon, lat);
    // 超过 2000km：不再显示本地倒计时（与走时表远距回退边界一致）
    if surface > 2000.0 {
        clear_countdown(snap);
        return;
    }
    // 本机已在该震级 S 波最大传播范围之外：波到不了，不显示倒计时
    let max_reach = max_s_spread_for(ev.magnitude, &snap.wave_spread_km);
    if surface > max_reach * 1.05 {
        clear_countdown(snap);
        return;
    }
    // JMA2001 超 2000km 自动回退 ak135（再不行直线兜底）
    let total = eng.eta_s_to_site(ev.longitude, ev.latitude, ev.depth_km, lon, lat);
    if !total.is_finite() || total <= 0.0 {
        clear_countdown(snap);
        return;
    }
    // 对齐 eewcn：到达后挂 0 秒，直到 PS 波圈走完才卸倒计时
    let remain = (total - elapsed_s).ceil().max(0.0) as i32;
    let (lv, text) = jian_core::estimate_at_site(
        ev.intensity_kind,
        ev.magnitude,
        ev.depth_km,
        surface,
    );
    snap.countdown_s = Some(remain);
    snap.countdown_local_kind = ev.intensity_kind;
    snap.countdown_local_level = lv;
    snap.countdown_local_text = text;
}

fn max_s_spread_for(magnitude: f64, custom: &[(f64, f64, f64)]) -> f64 {
    max_s_spread_km(magnitude, custom)
}

fn eew_wave_live(
    e: &EewReport,
    now_ms: i64,
    spread: &[(f64, f64, f64)],
    travel: Option<&TravelEngine>,
) -> bool {
    if let Some(eng) = travel {
        if !is_wave_candidate(e) {
            return false;
        }
        if e.origin_ms <= 0 {
            return true;
        }
        let age_ms = now_ms - e.origin_ms;
        if age_ms < -60_000 {
            return false;
        }
        let elapsed = (age_ms as f64 / 1000.0).max(0.0);
        let max_km = max_s_spread_km(e.magnitude, spread);
        s_wave_still_spreading_to(eng, e.depth_km, elapsed, max_km)
    } else {
        eew_wave_active_spread(e, now_ms, spread)
    }
}

struct JianApp {
    /// 保持非阻塞日志 worker 存活至退出
    _log_guard: WorkerGuard,
    resource_root: PathBuf,
    snap: AppSnapshot,
    shell: MainShell,
    travel: Option<TravelEngine>,
    started: Instant,
    local_lat: Option<f64>,
    local_lon: Option<f64>,
    has_local: bool,
    geodata_dir: PathBuf,
    media_dir: PathBuf,
    cfg: AppConfig,
    rt: Runtime,
    net_rx: Option<NetHub>,
    audio: Option<AudioEngine>,
    auto_center_pending: bool,
    /// 已跟焦的预警 (event_id, serial)，避免每帧重复居中
    last_follow_key: Option<(String, u32)>,
    last_record_follow_key: Option<String>,
    /// 各侧栏源槽上次已播 TTS 的报文（对齐 eewcn last_report_data_dict）
    last_eew_tts: HashMap<String, EewReport>,
    /// 同槽上次朗读原文；文案相同绝不重读
    last_eew_tts_script: HashMap<String, String>,
    /// 多震 cinema 轮播：当前索引与下次切换时刻
    cinema_idx: usize,
    cinema_next_at: Instant,
    /// 已见过多震 id 签名，用于发现新震立刻跳转
    cinema_seen_key: String,
    prev_health: [HealthStatus; 5],
    prev_tsunami_key: String,
    /// 窗口几何变更后延迟写入用户配置
    window_geom_save_at: Option<Instant>,
    last_window_geom: Option<(f32, f32, Option<f32>, Option<f32>, bool)>,
    splash: SplashGate,
}

impl eframe::App for JianApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.set_embed_viewports(true);
        self.tick_window_geometry(ctx);

        if let Some(rx) = self.net_rx.as_mut() {
            while let Ok(ev) = rx.try_recv() {
                let eew_for_tts = match &ev {
                    NetEvent::Eew(report) => Some(report.clone()),
                    _ => None,
                };
                if let NetEvent::Eew(ref report) = ev {
                    // 发震过期：不播预警音（仍可进列表供回看）
                    let alert_ok = !origin_is_stale(report.origin_ms);
                    // SREV 仅 JMA（对齐要石：issue/update/final/cancel + caution/warn）
                    if alert_ok && agency_family(&report.agency.0) == AgencyFamily::Jma {
                        if let Some(audio) = self.audio.as_mut() {
                            // 启动界面期间历史灌入只登记不播
                            if self.splash.active() {
                                audio.seed_eew_seen(
                                    &report.agency.0,
                                    &report.event_id,
                                    report.serial,
                                );
                            } else {
                                let play_caution = report.intensity_kind
                                    == IntensityKind::JmaShindo
                                    && report.intensity_level >= 3
                                    && !report.is_assumption;
                                audio.on_eew(
                                    &report.agency.0,
                                    &report.event_id,
                                    report.serial,
                                    report.is_final,
                                    report.is_cancel,
                                    report.is_warn,
                                    play_caution,
                                );
                            }
                        }
                    } else if !alert_ok {
                        if let Some(audio) = self.audio.as_mut() {
                            audio.seed_eew_seen(
                                &report.agency.0,
                                &report.event_id,
                                report.serial,
                            );
                        }
                    }
                }
                jian_net::apply_event_filtered(
                    &mut self.snap,
                    ev,
                    Some(&self.cfg.filters),
                );
                // 预警 TTS：侧栏展示条 + 内容/文案去重。
                // 启动界面未结束：只登记基线，不朗读；消失后才播后续更新。
                if let Some(report) = eew_for_tts {
                    if self.cfg.audio.tts_enabled
                        && !self.cfg.audio.mute
                        && !report.is_cancel
                        && !origin_is_stale(report.origin_ms)
                    {
                        if let Some((_, shown)) = eew_sidebar_entries(&self.snap.eew_list)
                            .into_iter()
                            .find(|(_, e)| same_eew_event(e, &report))
                        {
                            let accepted = shown.serial == report.serial
                                && (shown.magnitude - report.magnitude).abs() < 0.05
                                && shown.agency.0 == report.agency.0;
                            if accepted {
                                let slot = eew_sidebar_group_key(&shown.agency.0);
                                let script = build_eew_tts_script(shown);
                                let same_script = self
                                    .last_eew_tts_script
                                    .get(&slot)
                                    .is_some_and(|s| s == &script);
                                let last = self.last_eew_tts.get(&slot);
                                if !same_script && should_play_eew_tts(last, shown) {
                                    if !self.splash.active() {
                                        speak_async(script.clone());
                                    }
                                    self.last_eew_tts.insert(slot.clone(), shown.clone());
                                    self.last_eew_tts_script.insert(slot, script);
                                }
                            }
                        }
                    }
                }
            }
        }

        // 速报 TTS（含预警中的确认速报；跳转与否由 upsert_record 决定）
        // 启动界面期间丢弃待播，避免冷启动 list 灌入朗读
        if self.cfg.audio.tts_enabled && !self.cfg.audio.mute {
            if let Some(rec) = self.snap.pending_tts_record.take() {
                if !self.splash.active() {
                    speak_async(build_record_tts_script(&rec));
                }
            }
        } else {
            self.snap.pending_tts_record = None;
        }

        // 同步自定义震级传播表到快照（仅变更时写入，避免每帧分配+排序）
        {
            let mut next: Vec<(f64, f64, f64)> = self
                .cfg
                .travel
                .mag_spread_km
                .iter()
                .map(|p| (p.min_magnitude, p.max_magnitude, p.max_km))
                .collect();
            next.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            if self.snap.wave_spread_km != next {
                self.snap.wave_spread_km = next;
            }
        }

        self.tick_health_audio();
        self.tick_tsunami_audio();
        self.tick_typhoon_update();

        // 启动：稍等快照汇聚后优先跟有效预警；没有则跟最新速报。
        // 注意：eew_list 里常有已过期历史项，不能仅用「列表非空」卡住 pending，
        // 否则相机一直停在默认东亚视角，侧栏速报已选中也不会飞过去。
        if self.auto_center_pending {
            let elapsed = self.started.elapsed().as_secs_f64();
            if elapsed >= 1.2 {
                if let Some((lon, lat, _zoom)) = self.snap.focus_latest_event() {
                    let z = if self.snap.active.is_some() {
                        gq_jump_zoom()
                    } else {
                        8.5
                    };
                    self.shell.map.fly_to(lon, lat, Some(z), 0.9);
                    self.auto_center_pending = false;
                    if let Some(ev) = &self.snap.active {
                        self.last_follow_key = Some((ev.event_id.clone(), ev.serial));
                    }
                } else if let Some((lon, lat, zoom, key)) = self.snap.focus_latest_record() {
                    // 速报对准震中，勿用球体日本构图东偏（否则堪察加会被拉成北太平洋全景）
                    self.shell.map.fly_to_centered(lon, lat, Some(zoom), 1.0);
                    self.last_record_follow_key = Some(key);
                    self.auto_center_pending = false;
                } else if elapsed >= 4.0 {
                    // 仍无坐标数据：放弃启动跟焦，交给后续实时跟焦
                    self.auto_center_pending = false;
                }
            }
        } else if self.cfg.ui.auto_follow_eew {
            // 多震轮播：新震立刻跳；到点切换；更新报只刷数据，不打断停留计时
            // 注意：upsert_eew 已先写入 active，不能用 active==live 判断「是否新震」
            let browsing_station = self
                .snap
                .active
                .as_ref()
                .is_some_and(|e| e.event_id.starts_with("station:"));
            let browsing_record = self
                .snap
                .active
                .as_ref()
                .is_some_and(|e| e.event_id.starts_with("record:"));
            // 用户点选速报并锁镜头时，勿把 active/Tab 抢回预警（新预警到达仍强制打断）
            let hold_record_pick =
                browsing_record && self.shell.map.user_camera_override;
            let now = chrono::Utc::now().timestamp_millis();
            // 跟焦/轮播：传播中独立地震全进同一队列（一源多震 + 多源多震），按发震旧→新顺序轮切
            // 左上框仍每源一框；仅合并同事件镜像
            let live: Vec<&EewReport> = {
                let raw: Vec<&EewReport> = self
                    .snap
                    .eew_list
                    .iter()
                    .filter(|e| {
                        eew_wave_live(e, now, &self.snap.wave_spread_km, self.travel.as_ref())
                    })
                    .collect();
                dedupe_same_eew_events(&raw)
            };
            // 新预警必须打断测站浏览；仅轮播停留时保留测站锁定
            let allow_follow = !browsing_station || !live.is_empty();
            if allow_follow {
                let live = live;
                let seen_key = live
                    .iter()
                    .map(|e| cinema_event_key(e))
                    .collect::<Vec<_>>()
                    .join(";");
                if live.is_empty() {
                    self.cinema_seen_key.clear();
                } else if live.len() == 1 {
                    let ev = live[0];
                    let key = (ev.event_id.clone(), ev.serial);
                    let prev_id = self.last_follow_key.as_ref().map(|(id, _)| id.as_str());
                    let is_new_event = prev_id != Some(ev.event_id.as_str());
                    // 速报/测站曾抢焦点时：跟焦键未变也要把 active/Tab 拉回预警
                    let active_is_this_eew = self.snap.active.as_ref().is_some_and(|a| {
                        a.event_id == ev.event_id
                            && !a.event_id.starts_with("record:")
                            && !a.event_id.starts_with("station:")
                    });
                    if is_new_event {
                        // 新预警：强制飞跃（无视用户拖拽锁定）
                        let z = gq_jump_zoom();
                        self.shell.map.fly_to(ev.longitude, ev.latitude, Some(z), 0.9);
                        self.shell.map.user_camera_override = false;
                        self.last_follow_key = Some(key);
                        self.cinema_idx = 0;
                        self.cinema_seen_key = seen_key;
                        self.snap.active = Some(ev.clone());
                        self.snap.tab = jian_core::SidebarTab::Eew;
                        self.snap.overlay_mode = jian_core::OverlayMode::Wave;
                    } else if hold_record_pick {
                        // 保留用户点选的速报镜头
                    } else if self.last_follow_key.as_ref() != Some(&key) {
                        // 同震更新报：刷新 active，不重飞
                        self.last_follow_key = Some(key);
                        self.snap.active = Some(ev.clone());
                        self.snap.tab = jian_core::SidebarTab::Eew;
                        self.snap.overlay_mode = jian_core::OverlayMode::Wave;
                    } else if !active_is_this_eew {
                        self.snap.active = Some(ev.clone());
                        self.snap.tab = jian_core::SidebarTab::Eew;
                        self.snap.overlay_mode = jian_core::OverlayMode::Wave;
                    }
                } else {
                    let mut jump_idx: Option<usize> = None;
                    let mut is_new_arrival = false;
                    if seen_key != self.cinema_seen_key {
                        let old: std::collections::HashSet<&str> = self
                            .cinema_seen_key
                            .split(';')
                            .filter(|s| !s.is_empty())
                            .collect();
                        // 新震：按队列顺序取最早出现的新项（与旧→新排序一致）
                        for (i, e) in live.iter().enumerate() {
                            let id = cinema_event_key(e);
                            if !old.contains(id.as_str()) {
                                jump_idx = Some(i);
                                is_new_arrival = true;
                                break;
                            }
                        }
                        if jump_idx.is_none() {
                            // 集合变化（有震结束）：尽量钉住当前事件，再按序继续
                            let cur_key = self
                                .snap
                                .active
                                .as_ref()
                                .map(cinema_event_key)
                                .or_else(|| {
                                    self.last_follow_key.as_ref().map(|(id, _)| {
                                        // 回退：用 event_id 在 live 里找
                                        live.iter()
                                            .find(|e| e.event_id == *id)
                                            .map(|e| cinema_event_key(e))
                                            .unwrap_or_default()
                                    })
                                })
                                .unwrap_or_default();
                            if let Some(i) =
                                live.iter().position(|e| cinema_event_key(e) == cur_key)
                            {
                                self.cinema_idx = i;
                            } else {
                                let idx = self.cinema_idx.min(live.len() - 1);
                                jump_idx = Some(idx);
                                is_new_arrival = true;
                            }
                        }
                        self.cinema_seen_key = seen_key;
                    } else if !self.shell.map.user_camera_override
                        && Instant::now() >= self.cinema_next_at
                    {
                        // 按队列顺序 +1 轮播（跨源、同源同一圈）
                        jump_idx = Some((self.cinema_idx + 1) % live.len());
                    }
                    if let Some(idx) = jump_idx {
                        if is_new_arrival || !self.shell.map.user_camera_override {
                            let ev = live[idx];
                            let z = gq_jump_zoom();
                            self.shell.map.fly_to(ev.longitude, ev.latitude, Some(z), 0.9);
                            if is_new_arrival {
                                self.shell.map.user_camera_override = false;
                            }
                            self.last_follow_key = Some((ev.event_id.clone(), ev.serial));
                            self.cinema_idx = idx;
                            // GlobalQuake cinemaModeSwitchTime 默认 10 秒
                            self.cinema_next_at =
                                Instant::now() + std::time::Duration::from_secs_f64(10.0);
                            self.snap.active = Some(ev.clone());
                            self.snap.tab = jian_core::SidebarTab::Eew;
                            self.snap.overlay_mode = jian_core::OverlayMode::Wave;
                        }
                    } else if !hold_record_pick {
                        // 左上框点选 / 侧栏选中某条 live 时，跟齐 cinema 索引并重置停留
                        if let Some(a) = self.snap.active.as_ref() {
                            if let Some(i) = live.iter().position(|e| same_eew_event(e, a)) {
                                if i != self.cinema_idx {
                                    self.cinema_idx = i;
                                    self.cinema_next_at = Instant::now()
                                        + std::time::Duration::from_secs_f64(10.0);
                                }
                            }
                        }
                        // 停留中：同步当前震中最新报文，绝不因 serial 重置计时或跳转
                        let idx = self.cinema_idx.min(live.len() - 1);
                        let ev = live[idx];
                        let key = (ev.event_id.clone(), ev.serial);
                        if self.last_follow_key.as_ref() != Some(&key)
                            || self.snap.active.as_ref().map(|a| a.serial) != Some(ev.serial)
                        {
                            self.last_follow_key = Some(key);
                            self.snap.active = Some(ev.clone());
                            self.snap.tab = jian_core::SidebarTab::Eew;
                            self.snap.overlay_mode = jian_core::OverlayMode::Wave;
                        }
                    }
                }
            }
        }
        // 海啸预报到期自动清除（注意报/警报等靠解除报文）
        {
            let now = chrono::Utc::now().timestamp_millis();
            self.snap.prune_expired_tsunami(now);
        }

        // 无预警：相机始终聚焦最新一条速报
        if !self.auto_center_pending && self.started.elapsed().as_secs_f64() >= 2.5 {
            let now = chrono::Utc::now().timestamp_millis();
            let eew_live = self.snap.eew_list.iter().any(|e| {
                eew_wave_live(e, now, &self.snap.wave_spread_km, self.travel.as_ref())
            });
            let browsing_station = self
                .snap
                .active
                .as_ref()
                .is_some_and(|e| e.event_id.starts_with("station:"));
            if eew_live {
                self.snap.pending_record_fly = None;
                self.last_record_follow_key = None;
            } else if !browsing_station && !self.shell.map.user_camera_override {
                if let Some((lon, lat, zoom)) = self.snap.pending_record_fly.take() {
                    if lat.abs() > 0.01 || lon.abs() > 0.01 {
                        self.shell.map.fly_to_centered(lon, lat, Some(zoom), 1.0);
                    }
                }
                // 无预警：列表头变化时选中并跟焦最新有坐标速报（对准震中，不做日本构图东偏）
                if let Some((lon, lat, zoom, key)) = self.snap.latest_record_camera() {
                    if self.last_record_follow_key.as_ref() != Some(&key) {
                        let _ = self.snap.focus_latest_record();
                        self.shell.map.fly_to_centered(lon, lat, Some(zoom), 1.0);
                        self.last_record_follow_key = Some(key);
                    }
                }
            }
        } else {
            self.snap.pending_record_fly = None;
        }

        if let Some(ev) = self.snap.active.as_ref() {
            let is_eew = !ev.event_id.starts_with("record:")
                && !ev.event_id.starts_with("station:");
            if is_eew {
                let elapsed = if ev.origin_ms > 0 {
                    let now = chrono::Utc::now().timestamp_millis();
                    ((now - ev.origin_ms) as f64 / 1000.0).max(0.0)
                } else {
                    self.started.elapsed().as_secs_f64()
                };
                refresh_countdown(
                    &mut self.snap,
                    self.travel.as_ref(),
                    self.local_lat,
                    self.local_lon,
                    Some(elapsed),
                );
            } else {
                clear_countdown(&mut self.snap);
            }
        }

        // 倒计时播报：仅本机位置有效时（禁止无位置假播）
        if let Some(audio) = self.audio.as_mut() {
            let remain = if self.has_local {
                self.snap.countdown_s
            } else {
                None
            };
            audio.tick_countdown(remain);
        }

        let (ui_action, persist_cfg) = self.shell.ui(
            ctx,
            &mut self.snap,
            self.travel.as_ref(),
            &mut self.cfg,
        );
        if persist_cfg {
            if let Err(e) = self.cfg.save_user_overlay() {
                tracing::warn!("保存侧栏/图例偏好失败: {e:#}");
            }
        }
        if let Some(action) = ui_action {
            match action {
                SettingsAction::Saved { cfg, close } => {
                    match self.apply_saved_config(cfg) {
                        Ok((path, renderer_changed)) => {
                            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                                if self.cfg.ui.always_on_top {
                                    egui::WindowLevel::AlwaysOnTop
                                } else {
                                    egui::WindowLevel::Normal
                                },
                            ));
                            ctx.send_viewport_cmd(egui::ViewportCommand::Title(
                                window_title_for_lang(&self.cfg.ui.language),
                            ));
                            self.snap.show_wave_rings = self.cfg.ui.show_wave_rings;
                            let mut note = format!("已保存到 {}", path.display());
                            if renderer_changed {
                                note.push_str("；渲染设备需重启生效，其余已热重载");
                            }
                            self.shell.settings.set_save_note(note);
                            if close {
                                self.shell.settings.close_after_save();
                            }
                        }
                        Err(e) => self
                            .shell
                            .settings
                            .set_save_note(format!("保存失败：{e}")),
                    }
                }
                SettingsAction::TestSound {
                    mute,
                    master_volume,
                    countdown_volume,
                    event_pack,
                    countdown_pack,
                } => {
                    if let Some(audio) = self.audio.as_mut() {
                        audio.set_mute(mute);
                        audio.set_volumes(master_volume, countdown_volume);
                        audio.set_packs(&event_pack, &countdown_pack);
                        audio.play(AudioCue::Issue);
                        // 试听后恢复已应用配置，避免草稿音量「偷改」运行态
                        audio.set_mute(self.cfg.audio.mute);
                        audio.set_volumes(
                            self.cfg.audio.master_volume,
                            self.cfg.audio.countdown_volume,
                        );
                        audio.set_packs(
                            &self.cfg.audio.event_pack,
                            &self.cfg.audio.countdown_pack,
                        );
                    }
                }
                SettingsAction::Cancelled => {}
                SettingsAction::ClearTileCache => {
                    self.shell.map.clear_tile_cache();
                    self.shell
                        .settings
                        .set_save_note("已清除内存瓦片缓存");
                }
                SettingsAction::OpenLogDir => {
                    let dir = logging::resolve_log_dir(&self.resource_root, &self.cfg.log);
                    match logging::open_log_dir(&dir) {
                        Ok(()) => self
                            .shell
                            .settings
                            .set_save_note(format!("已打开日志目录 {}", dir.display())),
                        Err(e) => self
                            .shell
                            .settings
                            .set_save_note(format!("打开日志目录失败：{e}")),
                    }
                }
            }
        }

        // 启动画面：底图可用后淡出关闭（最短展示 + 约 0.55s 淡出）
        let load = self.shell.map.load_state();
        if self.splash.tick(load) {
            let theme = AppUiTheme::from_config(&self.cfg.ui.theme);
            draw_splash(
                ctx,
                &theme.palette(),
                self.shell.map.load_status(),
                load,
                self.splash.opacity(),
            );
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }

        // 有指针活动立即重绘，避免空闲降频把点击「吃掉」
        let pointer_busy = ctx.input(|i| {
            i.pointer.any_down()
                || i.pointer.button_clicked(egui::PointerButton::Primary)
                || i.pointer.button_released(egui::PointerButton::Primary)
                || i.pointer.button_clicked(egui::PointerButton::Secondary)
                || i.smooth_scroll_delta.length_sq() > 0.0
        });
        let now_ms = chrono::Utc::now().timestamp_millis();
        let waves_busy = self.snap.show_wave_rings
            && self.snap.eew_list.iter().any(|e| {
                eew_wave_live(e, now_ms, &self.snap.wave_spread_km, self.travel.as_ref())
            });
        let need = if pointer_busy
            || self.shell.map.wants_anim_repaint()
            || self.splash.active()
        {
            RepaintNeed::Interactive
        } else if waves_busy {
            RepaintNeed::Waves
        } else if self.shell.settings.open || self.snap.countdown_s.is_some() {
            RepaintNeed::Soft
        } else {
            RepaintNeed::Idle
        };
        let fps_ms = repaint_ms(&self.cfg.performance, need);
        ctx.request_repaint_after(std::time::Duration::from_millis(fps_ms));
    }
}

impl JianApp {
    fn tick_window_geometry(&mut self, ctx: &egui::Context) {
        let (w, h, x, y, maximized) = ctx.input(|i| {
            let v = i.viewport();
            let inner = v.inner_rect.unwrap_or(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(self.cfg.window.width, self.cfg.window.height),
            ));
            let outer = v.outer_rect;
            let (x, y) = outer
                .map(|r| (Some(r.min.x), Some(r.min.y)))
                .unwrap_or((self.cfg.window.x, self.cfg.window.y));
            (
                inner.width().round(),
                inner.height().round(),
                x.map(|v| v.round()),
                y.map(|v| v.round()),
                v.maximized.unwrap_or(false),
            )
        });
        if w < 200.0 || h < 150.0 {
            return;
        }
        let geom = (w, h, x, y, maximized);
        if self.last_window_geom.as_ref() == Some(&geom) {
            if let Some(at) = self.window_geom_save_at {
                if at.elapsed() >= std::time::Duration::from_millis(600) {
                    self.window_geom_save_at = None;
                    let same = (self.cfg.window.width - w).abs() < 0.5
                        && (self.cfg.window.height - h).abs() < 0.5
                        && self.cfg.window.x == x
                        && self.cfg.window.y == y
                        && self.cfg.window.maximized == maximized;
                    if same {
                        return;
                    }
                    self.cfg.window.width = w;
                    self.cfg.window.height = h;
                    self.cfg.window.x = x;
                    self.cfg.window.y = y;
                    self.cfg.window.maximized = maximized;
                    if let Err(e) = self.cfg.save_user_overlay() {
                        tracing::warn!("保存窗口几何失败: {e:#}");
                    } else {
                        tracing::debug!(w, h, ?x, ?y, maximized, "窗口几何已保存");
                    }
                }
            }
            return;
        }
        self.last_window_geom = Some(geom);
        self.window_geom_save_at = Some(Instant::now());
    }

    /// 台风指纹变化：提示音 + 视角跟焦（对齐 eewcn MapTyphoonLogic）。
    fn tick_typhoon_update(&mut self) {
        // 启动界面只登记 pending，消失后再播/飞（首包跟焦不丢）
        if self.splash.active() {
            return;
        }
        let play_hint = self.snap.pending_typhoon_hint;
        self.snap.pending_typhoon_hint = false;
        let fly = self.snap.pending_typhoon_fly.take();
        if play_hint {
            if let Some(audio) = self.audio.as_mut() {
                audio.play(AudioCue::UiNotify);
            }
        }
        let Some((lon, lat, radius_km)) = fly else {
            return;
        };
        if !self.cfg.ui.show_typhoons || !self.cfg.sources.typhoon.enabled {
            return;
        }
        // eewcn: zoomLevel<=3 不跟焦；锁定视角（用户拖拽）也不跟
        if self.shell.map.zoom <= 3.0 || self.shell.map.user_camera_override {
            return;
        }
        let z = zoom_to_fit_radius_km(
            lat,
            radius_km.max(1.0),
            self.shell.map.last_viewport_min_px,
            0.45,
        )
        .min(7.0);
        self.shell.map.fly_to(lon, lat, Some(z), 1.0);
    }

    fn tick_health_audio(&mut self) {
        let cur = [
            self.snap.health_jian,
            self.snap.health_wolfx,
            self.snap.health_p2p,
            self.snap.health_pancakes,
            self.snap.health_fssn,
        ];
        let prev = self.prev_health;
        if prev == cur {
            return;
        }
        // 启动界面未结束：源接通不滴声，避免误扰
        if self.splash.active() {
            self.prev_health = cur;
            return;
        }
        let any_ok = |h: [HealthStatus; 5]| h.iter().any(|s| matches!(s, HealthStatus::Normal));
        let all_bad = |h: [HealthStatus; 5]| h.iter().all(|s| matches!(s, HealthStatus::Abnormal));
        if let Some(audio) = self.audio.as_mut() {
            if !any_ok(prev) && any_ok(cur) {
                audio.play(AudioCue::UiConnectOk);
            } else if !all_bad(prev) && all_bad(cur) {
                audio.play(AudioCue::UiConnectFail);
            }
        }
        self.prev_health = cur;
    }


    fn tick_tsunami_audio(&mut self) {
        let key = match self.snap.tsunami.as_ref() {
            Some(t) if !t.cancelled => format!("{}|{}", t.grade, t.issued_ms),
            Some(_) => "cancel".into(),
            None => String::new(),
        };
        if key == self.prev_tsunami_key {
            return;
        }
        let prev = self.prev_tsunami_key.clone();
        self.prev_tsunami_key = key.clone();
        // 启动界面期间灌入的海啸状态不播 SREV 海啸音
        if self.splash.active() {
            return;
        }
        let Some(audio) = self.audio.as_mut() else {
            return;
        };
        // upsert 解除报会直接清空为 None；到期 prune 同理 → key 为空也视为解除
        let is_cancel =
            key == "cancel" || (key.is_empty() && !prev.is_empty() && prev != "cancel");
        if is_cancel {
            if !prev.is_empty() && prev != "cancel" {
                let g = tsunami_audio_grade(prev.split('|').next().unwrap_or("Forecast"));
                audio.play(AudioCue::Tsunami {
                    grade: g,
                    kind: TsunamiCueKind::Cancel,
                });
            }
            return;
        }
        if key.is_empty() {
            return;
        }
        let grade = key.split('|').next().unwrap_or("Forecast");
        let g = tsunami_audio_grade(grade);
        let kind = if prev.is_empty() || prev == "cancel" {
            TsunamiCueKind::Issue
        } else {
            TsunamiCueKind::Update
        };
        audio.play(AudioCue::Tsunami { grade: g, kind });
    }

    fn apply_saved_config(&mut self, new_cfg: AppConfig) -> Result<(PathBuf, bool), String> {
        let mut new_cfg = new_cfg;
        new_cfg.ui.language = jian_config::normalize_ui_language(&new_cfg.ui.language);
        jian_ui::set_lang_from_config(&new_cfg.ui.language);

        let sources_same = self.cfg.sources_runtime_equal(&new_cfg);
        let weather_same = self.cfg.ui.show_global_radar == new_cfg.ui.show_global_radar
            && self.cfg.ui.show_global_cloud == new_cfg.ui.show_global_cloud;
        let travel_same = self.cfg.travel.primary == new_cfg.travel.primary
            && self.cfg.travel.fallback == new_cfg.travel.fallback
            && self.cfg.travel.linear_last_resort == new_cfg.travel.linear_last_resort;
        let renderer_changed = self.cfg.performance.device.to_ascii_lowercase()
            != new_cfg.performance.device.to_ascii_lowercase();
        self.cfg = new_cfg;
        // 日志 stderr 热切换（控制台层启动时已挂载，此处只拨门闩 + Alloc/Free）
        logging::set_console_output(self.cfg.log.console);

        self.local_lat = self.cfg.local.latitude;
        self.local_lon = self.cfg.local.longitude;
        self.has_local = self.local_lat.is_some() && self.local_lon.is_some();
        self.shell.local_lat = self.local_lat;
        self.shell.local_lon = self.local_lon;
        self.shell.show_nied_clock = self.cfg.ui.show_nied_clock;
        self.shell.intensity_scale = self.cfg.ui.intensity_scale.clone();
        self.shell.home_lon = self.cfg.map.default_lon;
        self.shell.home_lat = self.cfg.map.default_lat;
        self.shell.home_zoom = self.cfg.map.default_zoom;

        let basemap = BasemapMode::resolve(&self.cfg.map.basemap, &self.cfg.ui.theme);
        self.geodata_dir = resolve_geodata_dir(&self.cfg.map.geodata_dir);
        self.shell.reload_basemap(
            basemap,
            self.cfg.map.tile_base.clone(),
            self.cfg.map.tile_source.clone(),
            Some(self.geodata_dir.as_path()),
            ProjectionMode::parse(&self.cfg.map.projection),
            self.cfg.ui.show_faults,
        );
        self.shell
            .map
            .set_tile_capacity(jian_util::perf::recommended_tile_slots(
                self.cfg.performance.device.eq_ignore_ascii_case("cpu"),
            ));

        if !travel_same {
            match TravelEngine::load_named(
                self.media_dir.join("travel"),
                &self.cfg.travel.primary,
                &self.cfg.travel.fallback,
                self.cfg.travel.linear_last_resort,
            ) {
                Ok(eng) => {
                    tracing::info!("走时表已按设置重载");
                    self.travel = Some(eng);
                }
                Err(e) => tracing::warn!("走时表重载失败: {e}"),
            }
        }

        if let Some(audio) = self.audio.as_mut() {
            audio.set_mute(self.cfg.audio.mute);
            audio.set_volumes(self.cfg.audio.master_volume, self.cfg.audio.countdown_volume);
            audio.set_packs(&self.cfg.audio.event_pack, &self.cfg.audio.countdown_pack);
        }

        if !sources_same {
            tracing::info!("数据源已变更，正在重连");
            drop(self.net_rx.take());
            let _enter = self.rt.enter();
            let stations_dir = self.media_dir.join("stations");
            self.net_rx = Some(spawn_hub_with_assets(
                &self.cfg.sources,
                stations_dir.is_dir().then_some(stations_dir),
            ));
        }
        if !sources_same || !weather_same {
            if let Some(hub) = self.net_rx.as_mut() {
                let _enter = self.rt.enter();
                hub.set_weather_overlay(
                    self.cfg.ui.show_global_radar,
                    self.cfg.ui.show_global_cloud,
                );
            }
        }

        match self.cfg.save_user_overlay() {
            Ok(path) => {
                tracing::info!(path = %path.display(), "用户配置已保存");
                Ok((path, renderer_changed))
            }
            Err(e) => {
                tracing::warn!("保存用户配置失败: {e:#}");
                Err(format!("{e:#}"))
            }
        }
    }
}



/// 预警跟焦落地固定 Z11；落地后由 auto_zoom 从 Z11 缓慢线性跟 S 波缩小。
fn gq_jump_zoom() -> f64 {
    11.0
}

fn tsunami_audio_grade(grade: &str) -> u8 {
    match grade {
        "MajorWarning" | "Major" | "Warning" => 3,
        "Watch" => 2,
        _ => 1,
    }
}
