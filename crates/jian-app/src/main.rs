//! EEWView（地震视监器）桌面客户端入口。

use anyhow::Result;
use eframe::egui;
use jian_audio::AudioEngine;
use jian_config::AppConfig;
use jian_core::{AppSnapshot, IntensityKind};
use jian_net::{apply_event, spawn_hub, NetEvent, NetRx};
use jian_travel::TravelEngine;
use jian_ui::MainShell;
use std::path::PathBuf;
use std::time::Instant;
use tokio::runtime::Runtime;

fn assets_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.pop(); // crates
    dir.pop(); // repo root
    dir.join("assets")
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

    let root = assets_root();
    let cfg = AppConfig::load_default_or_builtin(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config/default.toml"),
    );
    tracing::info!(
        event_pack = %cfg.audio.event_pack,
        tile = %format!("{}/{}", cfg.map.tile_base, cfg.map.tile_source),
        jian = cfg.sources.jian.enabled,
        wolfx = cfg.sources.wolfx.enabled,
        p2p = cfg.sources.p2pquake.enabled,
        jian_token = cfg.sources.jian.access_token.is_some(),
        "config ready"
    );

    let travel = match TravelEngine::load_default(root.join("travel")) {
        Ok(eng) => {
            let t = eng.travel_time(jian_travel::Wave::S, 40.0, 100.0);
            tracing::info!(s_travel_40km_100km_s = t, "JMA2001 travel table loaded");
            Some(eng)
        }
        Err(e) => {
            tracing::warn!("走时表未加载（先 cargo run -p convert_travel_tables）: {e}");
            None
        }
    };

    let audio = match AudioEngine::from_catalog(
        root.join("sound/catalog.json"),
        root.join("sound"),
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
        Some(rt.block_on(async { spawn_hub(&cfg.sources) }))
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
    shell.configure_map(
        cfg.map.tile_base.clone(),
        cfg.map.tile_source.clone(),
        cfg.map.default_lon,
        cfg.map.default_lat,
        cfg.map.default_zoom,
    );
    if let Some(ev) = &snap.active {
        shell.map.center_lon = ev.longitude;
        shell.map.center_lat = ev.latitude;
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_title("EEWView · 地震视监器"),
        ..Default::default()
    };

    let started = Instant::now();
    let has_local = cfg.local.latitude.is_some() && cfg.local.longitude.is_some();
    eframe::run_native(
        "EEWView",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(JianApp {
                snap,
                shell,
                travel,
                started,
                local_lat: cfg.local.latitude,
                local_lon: cfg.local.longitude,
                has_local,
                _rt: rt,
                net_rx,
                audio,
                auto_center_pending: live,
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
    if ev.origin_ms <= 0 && elapsed.is_none() {
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
    _rt: Runtime,
    net_rx: Option<NetRx>,
    audio: Option<AudioEngine>,
    auto_center_pending: bool,
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
                            false,
                            false,
                            shindo,
                        );
                    }
                }
                apply_event(&mut self.snap, ev);
            }
        }

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

        self.shell
            .ui(ctx, &mut self.snap, self.travel.as_ref());
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}
