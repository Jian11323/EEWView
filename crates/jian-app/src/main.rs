//! EEWView（地震视监器）桌面客户端入口。

use anyhow::Result;
use eframe::egui;
use jian_audio::{AudioCue, AudioEngine};
use jian_config::AppConfig;
use jian_core::{AppSnapshot, HealthStatus, IntensityKind};
use jian_net::{load_place_fix, spawn_hub, NetEvent, NetHub};
use jian_travel::TravelEngine;
use jian_map::BasemapMode;
use jian_ui::{MainShell, SettingsAction};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::runtime::Runtime;

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

/// 加载系统中文字体，避免汉字显示为方框。
fn install_cjk_fonts(ctx: &egui::Context) {
    let candidates = [
        // Windows
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\simsun.ttc",
        r"C:\Windows\Fonts\msyhbd.ttc",
        // macOS
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/Library/Fonts/Arial Unicode.ttf",
        // Linux
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
        "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
    ];
    let mut fonts = egui::FontDefinitions::default();
    for path in candidates {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        fonts.font_data.insert(
            "cjk".into(),
            egui::FontData::from_owned(bytes).into(),
        );
        if let Some(fam) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            fam.insert(0, "cjk".into());
        }
        if let Some(fam) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
            fam.push("cjk".into());
        }
        tracing::info!(path, "CJK font loaded");
        ctx.set_fonts(fonts);
        return;
    }
    tracing::warn!("未找到系统中文字体，界面汉字可能显示为方框");
}

/// JMA intensity_level 0–9 → catalog shindo_0…7
fn shindo_file_level(level: u8) -> u8 {
    match level {
        0..=4 => level,
        5 | 6 => 5,
        7 | 8 => 6,
        _ => 7,
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let root = resource_root();
    tracing::info!(root = %root.display(), "resource root");
    let assets = assets_root();
    let cfg = AppConfig::load_merged(root.join("config/default.toml"));
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
        Some(spawn_hub(&cfg.sources))
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
        basemap.show_vector().then_some(geodata_dir.as_path()),
        cfg.map.default_lon,
        cfg.map.default_lat,
        cfg.map.default_zoom,
    );
    snap.tab = match cfg.ui.sidebar_default_tab.to_ascii_lowercase().as_str() {
        "records" | "record" => jian_core::SidebarTab::Records,
        "station" => jian_core::SidebarTab::Station,
        _ => jian_core::SidebarTab::Eew,
    };
    if let Some(ev) = &snap.active {
        shell.map.center_lon = ev.longitude;
        shell.map.center_lat = ev.latitude;
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_title(format!(
                "EEWView · 地震视监器  v{}",
                env!("CARGO_PKG_VERSION")
            )),
        ..Default::default()
    };

    let started = Instant::now();
    let has_local = cfg.local.latitude.is_some() && cfg.local.longitude.is_some();
    eframe::run_native(
        "EEWView",
        options,
        Box::new(move |cc| {
            install_cjk_fonts(&cc.egui_ctx);
            Ok(Box::new(JianApp {
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
                prev_health: (
                    HealthStatus::Abnormal,
                    HealthStatus::Abnormal,
                    HealthStatus::Abnormal,
                ),
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe: {e}"))?;
    Ok(())
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
        if (local_lat.is_none() || local_lon.is_none()) && snap.active.is_none() {
            snap.countdown_s = None;
        }
        return;
    };
    if ev.is_cancel || (ev.origin_ms <= 0 && elapsed.is_none()) {
        snap.countdown_s = None;
        return;
    }
    let total = eng.eta_s_to_site(ev.longitude, ev.latitude, ev.depth_km, lon, lat);
    let remain = if let Some(e) = elapsed {
        (total - e).ceil() as i32
    } else {
        total.ceil() as i32
    };
    snap.countdown_s = Some(remain.max(0));
}

struct JianApp {
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
    prev_health: (HealthStatus, HealthStatus, HealthStatus),
}

impl eframe::App for JianApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(rx) = self.net_rx.as_mut() {
            while let Ok(ev) = rx.try_recv() {
                if let NetEvent::Eew(ref report) = ev {
                    if let Some(audio) = self.audio.as_mut() {
                        let shindo = if report.intensity_kind == IntensityKind::JmaShindo
                            && report.intensity_level > 0
                        {
                            Some(shindo_file_level(report.intensity_level))
                        } else {
                            None
                        };
                        audio.on_eew(
                            &report.agency.0,
                            &report.event_id,
                            report.serial,
                            report.is_final,
                            report.is_cancel,
                            shindo,
                        );
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

        if self.auto_center_pending {
            if let Some(ev) = &self.snap.active {
                if ev.latitude.abs() > 0.01 || ev.longitude.abs() > 0.01 {
                    self.shell.map.center_on(ev.longitude, ev.latitude, Some(6.0));
                    self.auto_center_pending = false;
                }
            }
        }

        if let Some(ev) = self.snap.active.as_ref() {
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
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}

impl JianApp {
    fn tick_health_audio(&mut self) {
        let cur = (
            self.snap.health_jian,
            self.snap.health_wolfx,
            self.snap.health_p2p,
        );
        let prev = self.prev_health;
        if prev == cur {
            return;
        }
        let any_ok = |h: (HealthStatus, HealthStatus, HealthStatus)| {
            matches!(h.0, HealthStatus::Normal)
                || matches!(h.1, HealthStatus::Normal)
                || matches!(h.2, HealthStatus::Normal)
        };
        let all_bad = |h: (HealthStatus, HealthStatus, HealthStatus)| {
            matches!(h.0, HealthStatus::Abnormal)
                && matches!(h.1, HealthStatus::Abnormal)
                && matches!(h.2, HealthStatus::Abnormal)
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
            basemap.show_vector().then_some(self.geodata_dir.as_path()),
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
                self.net_rx = Some(spawn_hub(&self.cfg.sources));
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
