//! EEWView（地震视监器）桌面客户端入口（P3：三源网络骨架）。

use anyhow::Result;
use eframe::egui;
use jian_config::AppConfig;
use jian_core::AppSnapshot;
use jian_net::{apply_event, spawn_hub, NetRx};
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

    let live = cfg.any_source_enabled();
    let mut snap = if live {
        tracing::info!("P3 live：空快照，等待 Wolfx / P2P / Jian 事件");
        AppSnapshot::empty()
    } else {
        tracing::info!("全部数据源关闭 → 使用 demo 假数据");
        AppSnapshot::demo()
    };

    let rt = Runtime::new()?;
    let net_rx = if live {
        Some(rt.block_on(async { spawn_hub(&cfg.sources) }))
    } else {
        None
    };

    // 有本机位置 + 走时表 + 活动事件时刷新倒计时
    refresh_countdown(&mut snap, travel.as_ref(), cfg.local.latitude, cfg.local.longitude, None);

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
                _rt: rt,
                net_rx,
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
        if snap.countdown_s.is_some() && local_lat.is_none() {
            // demo 占位可保留；live 无位置则清空
            if snap.eew_list.is_empty() && snap.records.is_empty() {
                // empty live
            }
        }
        return;
    };
    if ev.origin_ms <= 0 && elapsed.is_none() {
        // 无发震时刻时不假装倒计时
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
    _rt: Runtime,
    net_rx: Option<NetRx>,
    /// 收到首条带坐标的活动事件后居中一次
    auto_center_pending: bool,
}

impl eframe::App for JianApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(rx) = self.net_rx.as_mut() {
            while let Ok(ev) = rx.try_recv() {
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

        // 真实 origin_ms：用墙钟差；否则（demo）用启动经过时间
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

        self.shell
            .ui(ctx, &mut self.snap, self.travel.as_ref());
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}
