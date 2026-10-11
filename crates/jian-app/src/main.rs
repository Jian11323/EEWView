//! EEW View（地震视监器）桌面客户端入口。

// release：GUI 子系统，双击不弹控制台；debug：保留控制台便于看日志
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod logging;

use anyhow::Result;
use eframe::egui;
use jian_audio::{AudioCue, AudioEngine, TsunamiCueKind};
use jian_config::AppConfig;
use jian_core::{
    agency_family, live_wave_eews, AgencyFamily, AppSnapshot, HealthStatus, IntensityKind,
};
use jian_net::{load_place_fix, spawn_hub_with_assets, NetEvent, NetHub};
use jian_travel::{max_s_wave_spread_km, s_wave_still_spreading, TravelEngine, Wave};
use jian_map::{zoom_to_fit_radius_km, BasemapMode, ProjectionMode};
use jian_ui::{MainShell, SettingsAction};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::runtime::Runtime;
use tracing_appender::non_blocking::WorkerGuard;

fn looks_like_resource_root(dir: &Path) -> bool {
    dir.join("assets").is_dir()
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

fn assets_root() -> PathBuf {
    resource_root().join("assets")
}

fn resolve_geodata_dir(configured: &str) -> PathBuf {
    let p = Path::new(configured);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        resource_root().join(p)
    }
}

/// 编译嵌入霞鹜文楷 Regular（约 25MB；不另嵌 Mono、不读系统字体，加快编译与启动）。
fn install_cjk_fonts(ctx: &egui::Context) {
    const LXGW_WENKAI: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/lxgw-wenkai-v1.522/LXGWWenKai-Regular.ttf"
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
        fam.insert(0, "lxgw_wenkai".to_string());
    }
    if let Some(fam) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
        fam.insert(0, "lxgw_wenkai".to_string());
    }
    ctx.set_fonts(fonts);
    tracing::info!(bytes = LXGW_WENKAI.len(), "embedded LXGW WenKai");
}

fn main() -> Result<()> {
    let root = resource_root();
    let assets = assets_root();
    let cfg = AppConfig::load_merged(root.join("config/default.toml"));
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
        jian_token = cfg.sources.jian.access_token.is_some(),
        "config ready"
    );

    let travel = match TravelEngine::load_named(
        assets.join("travel"),
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

    load_place_fix(assets.join("place"));

    let audio = match AudioEngine::from_catalog(
        assets.join("sound/catalog.json"),
        assets.join("sound"),
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
    let mut snap = if live {
        tracing::info!("waiting for source events");
        AppSnapshot::empty()
    } else {
        tracing::info!("all sources disabled → offline sample");
        AppSnapshot::offline_sample()
    };

    let rt = Runtime::new()?;
    let net_rx = if live {
        let _enter = rt.enter();
        let stations_dir = assets.join("stations");
        Some(spawn_hub_with_assets(
            &cfg.sources,
            stations_dir.is_dir().then_some(stations_dir),
        ))
    } else {
        None
    };

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
    );
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

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 720.0])
        .with_title(format!(
            "{} \u{00b7} 地震视监器  [Beta] v{}",
            jian_ui::APP_DISPLAY_NAME,
            env!("CARGO_PKG_VERSION")
        ));
    if let Some(icon) = jian_ui::app_icon_data() {
        viewport = viewport.with_icon(icon);
    }
    if cfg.ui.always_on_top {
        viewport = viewport.with_always_on_top();
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let started = Instant::now();
    let has_local = cfg.local.latitude.is_some() && cfg.local.longitude.is_some();
    snap.show_wave_rings = cfg.ui.show_wave_rings;
    eframe::run_native(
        jian_ui::APP_DISPLAY_NAME,
        options,
        Box::new(move |cc| {
            install_cjk_fonts(&cc.egui_ctx);
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
                assets_dir: assets,
                cfg,
                rt,
                net_rx,
                audio,
                auto_center_pending: live,
                last_follow_key: None,
                last_record_follow_key: None,
                cinema_idx: 0,
                cinema_next_at: Instant::now(),
                cinema_seen_key: String::new(),
                prev_health: (
                    HealthStatus::Abnormal,
                    HealthStatus::Abnormal,
                    HealthStatus::Abnormal,
                    HealthStatus::Abnormal,
                ),
                prev_tsunami_key: String::new(),
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe: {e}"))?;
    Ok(())
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
    // PS 波走完最大传播距离后停止并隐藏（与地图波圈同一条件）
    if !s_wave_still_spreading(eng, ev.magnitude, ev.depth_km, elapsed_s) {
        clear_countdown(snap);
        return;
    }
    let total = eng.eta_s_to_site(ev.longitude, ev.latitude, ev.depth_km, lon, lat);
    // 用 ceil：显示/语音与「还剩整秒」一致；音效侧按秒边界重启滴滴
    let remain = (total - elapsed_s).ceil() as i32;
    // EEWCN：到达本机后倒计时隐藏（不再挂 0 秒）
    if remain <= 0 {
        clear_countdown(snap);
        return;
    }
    let surface = jian_util::haversine_km(ev.longitude, ev.latitude, lon, lat);
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
    assets_dir: PathBuf,
    cfg: AppConfig,
    rt: Runtime,
    net_rx: Option<NetHub>,
    audio: Option<AudioEngine>,
    auto_center_pending: bool,
    /// 已跟焦的预警 (event_id, serial)，避免每帧重复居中
    last_follow_key: Option<(String, u32)>,
    last_record_follow_key: Option<String>,
    /// 多震 cinema 轮播：当前索引与下次切换时刻
    cinema_idx: usize,
    cinema_next_at: Instant,
    /// 已见过多震 id 签名，用于发现新震立刻跳转
    cinema_seen_key: String,
    prev_health: (HealthStatus, HealthStatus, HealthStatus, HealthStatus),
    prev_tsunami_key: String,
}

impl eframe::App for JianApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(rx) = self.net_rx.as_mut() {
            while let Ok(ev) = rx.try_recv() {
                if let NetEvent::Eew(ref report) = ev {
                    // SREV 仅 JMA（对齐要石：issue/update/final/cancel + caution/warn）
                    if agency_family(&report.agency.0) == AgencyFamily::Jma {
                        if let Some(audio) = self.audio.as_mut() {
                            let play_caution = report.intensity_kind == IntensityKind::JmaShindo
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
                }
                jian_net::apply_event_filtered(
                    &mut self.snap,
                    ev,
                    Some(&self.cfg.filters),
                );
            }
        }

        self.tick_health_audio();
        self.tick_tsunami_audio();

        // 启动：稍等快照汇聚后跳到最新预警（速报不进左上角，也不自动跟焦）
        if self.auto_center_pending {
            let has_data = !self.snap.eew_list.is_empty();
            if has_data && self.started.elapsed().as_secs_f64() >= 1.2 {
                if let Some((lon, lat, _zoom)) = self.snap.focus_latest_event() {
                    let z = if let Some(ev) = &self.snap.active {
                        let now = chrono::Utc::now().timestamp_millis();
                        let elapsed = if ev.origin_ms > 0 {
                            ((now - ev.origin_ms) as f64 / 1000.0).max(0.0)
                        } else {
                            0.0
                        };
                        let s_km = if let Some(eng) = self.travel.as_ref() {
                            eng.surface_distance_for_elapsed(Wave::S, ev.depth_km, elapsed)
                        } else {
                            elapsed * 5.0
                        };
                        let r = s_km
                            .min(max_s_wave_spread_km(ev.magnitude))
                            .max(80.0);
                        zoom_to_fit_radius_km(
                            ev.latitude,
                            r.max(35.0),
                            self.shell.map.last_viewport_min_px,
                            0.58,
                        )
                        .clamp(6.8, 11.2)
                    } else {
                        7.2
                    };
                    self.shell.map.fly_to(lon, lat, Some(z), 1.0);
                    self.auto_center_pending = false;
                    if let Some(ev) = &self.snap.active {
                        self.last_follow_key = Some((ev.event_id.clone(), ev.serial));
                    }
                }
            } else if self.started.elapsed().as_secs_f64() >= 4.0 {
                // 超时无预警：改跟最新速报
                if self.snap.eew_list.is_empty() {
                    if let Some((lon, lat, zoom, key)) = self.snap.focus_latest_record() {
                        self.shell.map.fly_to(lon, lat, Some(zoom), 1.0);
                        self.last_record_follow_key = Some(key);
                    }
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
            if !browsing_station {
                let now = chrono::Utc::now().timestamp_millis();
                let live = live_wave_eews(&self.snap.eew_list, now, 30 * 60 * 1000);
                let seen_key = live
                    .iter()
                    .map(|e| format!("{}|{}", e.agency.0, e.event_id))
                    .collect::<Vec<_>>()
                    .join(";");
                if live.is_empty() {
                    self.cinema_seen_key.clear();
                } else if live.len() == 1 {
                    let ev = live[0];
                    let key = (ev.event_id.clone(), ev.serial);
                    let prev_id = self.last_follow_key.as_ref().map(|(id, _)| id.as_str());
                    let is_new_event = prev_id != Some(ev.event_id.as_str());
                    if is_new_event {
                        // 新预警：强制飞跃（无视用户拖拽锁定）
                        let z = gq_jump_zoom(
                            ev,
                            now,
                            self.travel.as_ref(),
                            self.shell.map.last_viewport_min_px,
                        );
                        self.shell.map.fly_to(ev.longitude, ev.latitude, Some(z), 1.6);
                        self.last_follow_key = Some(key);
                        self.cinema_idx = 0;
                        self.cinema_seen_key = seen_key;
                        self.snap.active = Some(ev.clone());
                        self.snap.tab = jian_core::SidebarTab::Eew;
                        self.snap.overlay_mode = jian_core::OverlayMode::Wave;
                    } else if self.last_follow_key.as_ref() != Some(&key) {
                        // 同震更新报：刷新 active，不重飞
                        self.last_follow_key = Some(key);
                        self.snap.active = Some(ev.clone());
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
                        for (i, e) in live.iter().enumerate() {
                            let id = format!("{}|{}", e.agency.0, e.event_id);
                            if !old.contains(id.as_str()) {
                                jump_idx = Some(i);
                                is_new_arrival = true;
                                break;
                            }
                        }
                        if jump_idx.is_none() {
                            // 集合变化（有震结束）时夹紧索引，不立刻重飞除非当前项已消失
                            let idx = self.cinema_idx.min(live.len() - 1);
                            let cur_id = format!("{}|{}", live[idx].agency.0, live[idx].event_id);
                            if self.cinema_seen_key.is_empty() || !seen_key.contains(&cur_id) {
                                jump_idx = Some(idx);
                                is_new_arrival = true;
                            } else {
                                self.cinema_idx = idx;
                            }
                        }
                        self.cinema_seen_key = seen_key;
                    } else if !self.shell.map.user_camera_override
                        && Instant::now() >= self.cinema_next_at
                    {
                        jump_idx = Some((self.cinema_idx + 1) % live.len());
                    }
                    if let Some(idx) = jump_idx {
                        if is_new_arrival || !self.shell.map.user_camera_override {
                            let ev = live[idx];
                            let z = gq_jump_zoom(
                                ev,
                                now,
                                self.travel.as_ref(),
                                self.shell.map.last_viewport_min_px,
                            );
                            self.shell.map.fly_to(ev.longitude, ev.latitude, Some(z), 1.6);
                            self.last_follow_key = Some((ev.event_id.clone(), ev.serial));
                            self.cinema_idx = idx;
                            let hold = (2.5 + ev.magnitude * 0.8).clamp(5.0, 8.0);
                            self.cinema_next_at =
                                Instant::now() + std::time::Duration::from_secs_f64(hold);
                            self.snap.active = Some(ev.clone());
                            self.snap.tab = jian_core::SidebarTab::Eew;
                            self.snap.overlay_mode = jian_core::OverlayMode::Wave;
                        }
                    } else {
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
            let eew_live = !jian_core::live_wave_eews(&self.snap.eew_list, now, 30 * 60 * 1000)
                .is_empty();
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
                        self.shell.map.fly_to(lon, lat, Some(zoom), 1.0);
                    }
                }
                if let Some((lon, lat, zoom, key)) = self.snap.latest_record_camera() {
                    if self.last_record_follow_key.as_ref() != Some(&key) {
                        self.shell.map.fly_to(lon, lat, Some(zoom), 1.0);
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

        if let Some(action) = self
            .shell
            .ui(ctx, &mut self.snap, self.travel.as_ref(), &self.cfg)
        {
            match action {
                SettingsAction::Saved { cfg, close } => {
                    match self.apply_saved_config(cfg) {
                        Ok(path) => {
                            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                                if self.cfg.ui.always_on_top {
                                    egui::WindowLevel::AlwaysOnTop
                                } else {
                                    egui::WindowLevel::Normal
                                },
                            ));
                            self.snap.show_wave_rings = self.cfg.ui.show_wave_rings;
                            self.shell
                                .settings
                                .set_save_note(format!("已保存到 {}", path.display()));
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
        // 空闲降频；波圈/飞跃由 UI·地图侧按需 60fps 请求，此处不因列表非空常驻高刷
        if self.shell.map.is_flying() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }
}

impl JianApp {
    fn tick_health_audio(&mut self) {
        let cur = (
            self.snap.health_jian,
            self.snap.health_wolfx,
            self.snap.health_p2p,
            self.snap.health_pancakes,
        );
        let prev = self.prev_health;
        if prev == cur {
            return;
        }
        let any_ok = |h: (HealthStatus, HealthStatus, HealthStatus, HealthStatus)| {
            matches!(h.0, HealthStatus::Normal)
                || matches!(h.1, HealthStatus::Normal)
                || matches!(h.2, HealthStatus::Normal)
                || matches!(h.3, HealthStatus::Normal)
        };
        let all_bad = |h: (HealthStatus, HealthStatus, HealthStatus, HealthStatus)| {
            matches!(h.0, HealthStatus::Abnormal)
                && matches!(h.1, HealthStatus::Abnormal)
                && matches!(h.2, HealthStatus::Abnormal)
                && matches!(h.3, HealthStatus::Abnormal)
        };
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
        let Some(audio) = self.audio.as_mut() else {
            return;
        };
        if key.is_empty() {
            return;
        }
        if key == "cancel" {
            if !prev.is_empty() && prev != "cancel" {
                let g = tsunami_audio_grade(prev.split('|').next().unwrap_or("Forecast"));
                audio.play(AudioCue::Tsunami {
                    grade: g,
                    kind: TsunamiCueKind::Cancel,
                });
            }
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

    fn apply_saved_config(&mut self, new_cfg: AppConfig) -> Result<PathBuf, String> {
        let sources_same = self.cfg.sources_runtime_equal(&new_cfg);
        let travel_same = self.cfg.travel.primary == new_cfg.travel.primary
            && self.cfg.travel.fallback == new_cfg.travel.fallback
            && self.cfg.travel.linear_last_resort == new_cfg.travel.linear_last_resort;
        self.cfg = new_cfg;

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
        );

        if !travel_same {
            match TravelEngine::load_named(
                self.assets_dir.join("travel"),
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
            if self.cfg.any_source_enabled() {
                let _enter = self.rt.enter();
                let stations_dir = self.assets_dir.join("stations");
                self.net_rx = Some(spawn_hub_with_assets(
                    &self.cfg.sources,
                    stations_dir.is_dir().then_some(stations_dir),
                ));
            }
        }

        match self.cfg.save_user_overlay() {
            Ok(path) => {
                tracing::info!(path = %path.display(), "用户配置已保存");
                Ok(path)
            }
            Err(e) => {
                tracing::warn!("保存用户配置失败: {e:#}");
                Err(format!("{e:#}"))
            }
        }
    }
}



/// GQ cinema 跳转缩放：按发震年龄估视野，并用当前 S 半径夹一层，只在跳转瞬间用一次。
fn gq_jump_zoom(
    ev: &jian_core::EewReport,
    now_ms: i64,
    travel: Option<&TravelEngine>,
    viewport_min_px: f32,
) -> f64 {
    let elapsed = if ev.origin_ms > 0 {
        ((now_ms - ev.origin_ms) as f64 / 1000.0).max(0.0)
    } else {
        0.0
    };
    let s_km = if let Some(eng) = travel {
        eng.surface_distance_for_elapsed(Wave::S, ev.depth_km, elapsed)
    } else {
        elapsed * 5.0
    };
    // 视野略收：S 圈约占短边 58%，默认更靠近震中
    let r = s_km
        .min(max_s_wave_spread_km(ev.magnitude))
        .max(28.0);
    let fit_s = zoom_to_fit_radius_km(ev.latitude, r, viewport_min_px, 0.58);
    let age_min = if ev.origin_ms > 0 {
        ((now_ms - ev.origin_ms) as f64 / 60_000.0).max(0.0)
    } else {
        0.0
    };
    let scroll = (age_min / 5.0).clamp(0.05, 1.2);
    let prefer = (12.2 - scroll * 2.0).clamp(7.0, 11.2);
    prefer.min(fit_s).clamp(6.8, 11.2)
}

fn tsunami_audio_grade(grade: &str) -> u8 {
    match grade {
        "MajorWarning" | "Major" | "Warning" => 3,
        "Watch" => 2,
        _ => 1,
    }
}
