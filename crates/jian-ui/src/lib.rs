//! 主界面分区：地图视口 + 浮层信息头 / 图例 / 倒计时 + 右栏列表。

mod brand;
mod intensity_icons;
mod settings;
mod splash;
mod theme;

pub use brand::{app_icon_data, logo_texture, APP_DISPLAY_NAME};
pub use splash::{draw_splash, SplashGate};

use chrono::{FixedOffset, Local};
use egui::{Color32, FontFamily, FontId, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2};
use intensity_icons::IntensityIcons;
use jian_core::palette::color_for;
use jian_core::{
    agency_bracket, eew_sidebar_entries, eew_wave_active_spread,
    is_wave_candidate, max_s_spread_km, place_with_agency, prefer_intensity_scale, AppSnapshot,
    EewReport, IntensityKind, ListSelection, OverlayMode, SidebarTab,
};
use jian_core::palette::{cn_intensity_rgb, instrumental_band_rgb};
use jian_map::{
    zoom_to_fit_radius_km, BasemapLoadState, BasemapMode, MapUiResult, MapViewport,
    ProjectionMode, TyphoonHoverHit, VectorStyle, OCEAN,
};
use jian_travel::{max_s_wave_spread_km, s_wave_still_spreading_to, TravelEngine, Wave};
use std::path::Path;
use theme::{ThemePalette, UiTheme};

pub use settings::{MapPickKind, SettingsAction, SettingsSession, SourceHealthView};
pub use theme::UiTheme as AppUiTheme;

/// 左上预警条 / 波圈 / 叉标共用：有走时表时按表半径判定，与倒计时卸除条件一致。
fn live_wave_eews_travel<'a>(
    list: &'a [EewReport],
    now_ms: i64,
    spread: &[(f64, f64, f64)],
    travel: Option<&TravelEngine>,
) -> Vec<&'a EewReport> {
    list.iter()
        .filter(|e| {
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
        })
        .collect()
}

fn overlay_child<R>(
    ui: &mut Ui,
    id_salt: &str,
    rect: egui::Rect,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    // 与 CentralPanel 同层绘制，避免 Foreground Area 的 Sense::hover 挡住 Background 命中
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id_salt)
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min))
            .sense(Sense::hover()),
    );
    add(&mut child)
}

pub struct MainShell {
    pub map: MapViewport,
    /// 地图波圈用经过时间（秒）
    pub map_elapsed: f64,
    /// Home / 复位用默认视口
    pub home_lon: f64,
    pub home_lat: f64,
    pub home_zoom: f64,
    /// 本机经纬（图例 Home 与地图标记）
    pub local_lon: Option<f64>,
    pub local_lat: Option<f64>,
    pub show_nied_clock: bool,
    pub intensity_scale: String,
    pub settings: SettingsSession,
    intensity_icons: IntensityIcons,
}

impl Default for MainShell {
    fn default() -> Self {
        Self {
            map: MapViewport::default(),
            map_elapsed: 0.0,
            home_lon: 135.0,
            home_lat: 35.0,
            home_zoom: 5.0,
            local_lon: None,
            local_lat: None,
            show_nied_clock: false,
            intensity_scale: "auto".into(),
            settings: SettingsSession::default(),
            intensity_icons: IntensityIcons::default(),
        }
    }
}

impl MainShell {
    pub fn configure_map(
        &mut self,
        basemap: BasemapMode,
        tile_base: impl Into<String>,
        tile_source: impl Into<String>,
        geodata_dir: Option<&Path>,
        lon: f64,
        lat: f64,
        zoom: f64,
        projection: ProjectionMode,
        show_faults: bool,
    ) {
        self.map.set_projection_mode(projection);
        self.map
            .configure(basemap, tile_base, tile_source, geodata_dir, show_faults);
        self.map.center_lon = lon;
        self.map.center_lat = lat;
        self.map.zoom = zoom;
        self.home_lon = lon;
        self.home_lat = lat;
        self.home_zoom = zoom;
    }

    /// 仅换底图/瓦片/投影，不改当前视口中心。
    pub fn reload_basemap(
        &mut self,
        basemap: BasemapMode,
        tile_base: impl Into<String>,
        tile_source: impl Into<String>,
        geodata_dir: Option<&Path>,
        projection: ProjectionMode,
        show_faults: bool,
    ) {
        let lon = self.map.center_lon;
        let lat = self.map.center_lat;
        let zoom = self.map.zoom;
        self.map.set_projection_mode(projection);
        self.map
            .configure(basemap, tile_base, tile_source, geodata_dir, show_faults);
        self.map.center_lon = lon;
        self.map.center_lat = lat;
        self.map.zoom = zoom;
    }

    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        snap: &mut AppSnapshot,
        travel: Option<&TravelEngine>,
        cfg: &jian_config::AppConfig,
    ) -> Option<SettingsAction> {
        let ui_theme = UiTheme::from_config(&cfg.ui.theme);
        ui_theme.apply_egui(ctx);
        let p = ui_theme.palette();
        // 暗色：要石海洋 #202124；亮色：主题 map_bg（避免纯黑闪底）
        self.map.set_ocean_fill(if ui_theme.is_light() { p.map_bg } else { OCEAN });
        self.map
            .set_vector_style(VectorStyle::for_theme(&cfg.ui.theme));
        self.map
            .set_paint_quality(jian_map::PaintQuality::from_profile(&cfg.performance.profile));

        snap.prune_finished_eews();
        self.intensity_icons.ensure(ctx);
        self.map_elapsed += ctx.input(|i| i.stable_dt) as f64;
        let now_ms = chrono::Utc::now().timestamp_millis();
        snap.show_wave_rings = cfg.ui.show_wave_rings;
        // 多震：只要仍有未取消的传播中事件就画波圈（不因当前选中项取消而全灭）
        let show_waves = snap.overlay_mode == OverlayMode::Wave && snap.show_wave_rings;
        let scale = self.intensity_scale.clone();

        let live_refs =
            live_wave_eews_travel(&snap.eew_list, now_ms, &snap.wave_spread_km, travel);
        let mut wave_owned: Vec<_> = live_refs.into_iter().cloned().collect();
        // 无有效发震时刻时用 map_elapsed 驱动波圈（上游 origin 缺测）
        let sample_elapsed = self.map_elapsed % 120.0;
        let mut map_ui = MapUiResult::default();

        // 波圈动画按档位降频，避免 uncapped request_repaint 拉高 CPU
        if show_waves && !wave_owned.is_empty() {
            let ms = cfg.performance.wave_repaint_ms();
            ctx.request_repaint_after(std::time::Duration::from_millis(ms));
        }

        // SidePanel：小窗优先保地图宽度，侧栏可缩到下限
        let screen_w = ctx.screen_rect().width();
        const MAP_MIN_W: f32 = 360.0;
        const SIDE_IDEAL: f32 = 280.0;
        const SIDE_MIN: f32 = 168.0;
        let sidebar_w = if screen_w < MAP_MIN_W + SIDE_MIN {
            (screen_w * 0.36).clamp(140.0, SIDE_MIN)
        } else {
            let cap = (screen_w - MAP_MIN_W).min(SIDE_IDEAL);
            cap.min(screen_w * 0.28).max(SIDE_MIN)
        };
        let mut side_action = None;

        egui::SidePanel::right("eew_sidebar")
            .exact_width(sidebar_w)
            .resizable(false)
            .frame(Frame::NONE.fill(p.sidebar_bg))
            .show(ctx, |ui| {
                side_action = draw_sidebar(
                    ui,
                    &p,
                    snap,
                    scale.as_str(),
                    &self.intensity_icons,
                    cfg.ui.show_volcanoes,
                    [
                        cfg.sources.jian.enabled,
                        cfg.sources.wolfx.enabled,
                        cfg.sources.p2pquake.enabled,
                        cfg.sources.pancakes.enabled,
                    ],
                );
            });

        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(p.map_bg))
            .show(ctx, |ui| {
                let map_rect = ui.available_rect_before_wrap();
                let home = match (self.local_lon, self.local_lat) {
                    (Some(lon), Some(lat)) => Some((lon, lat)),
                    _ => None,
                };
                for ev in &mut wave_owned {
                    if ev.origin_ms <= 0 {
                        ev.origin_ms = now_ms - (sample_elapsed * 1000.0) as i64;
                    }
                }
                let selected_volcano = snap.selection.and_then(|s| {
                    if s.tab == SidebarTab::Volcano {
                        Some(s.index)
                    } else {
                        None
                    }
                });
                let volcanoes_for_map = if cfg.ui.show_volcanoes {
                    snap.volcanoes.as_slice()
                } else {
                    &[]
                };
                let typhoons_for_map = if cfg.ui.show_typhoons && cfg.sources.typhoon.enabled {
                    snap.typhoons.as_slice()
                } else {
                    &[]
                };
                map_ui = self.map.ui(
                    ui,
                    snap.active.as_ref(),
                    &wave_owned,
                    &wave_owned,
                    travel,
                    now_ms,
                    show_waves,
                    &snap.stations,
                    &snap.records,
                    volcanoes_for_map,
                    selected_volcano,
                    typhoons_for_map,
                    &cfg.ui.typhoon_forecast_agency,
                    home,
                    snap.tsunami.as_ref(),
                    cfg.ui.show_faults,
                );
                if cfg.ui.auto_zoom_waves
                    && show_waves
                    && !self.map.is_flying()
                    && !self.map.user_camera_override
                {
                    let focus = snap.active.as_ref().filter(|e| {
                        !e.is_cancel
                            && !e.event_id.starts_with("record:")
                            && !e.event_id.starts_with("station:")
                            && (e.latitude.abs() > 0.01 || e.longitude.abs() > 0.01)
                    });
                    if let Some(ev) = focus {
                        let elapsed = if ev.origin_ms > 0 {
                            ((now_ms - ev.origin_ms) as f64 / 1000.0).max(0.0)
                        } else {
                            sample_elapsed
                        };
                        let s_km = if let Some(eng) = travel {
                            eng.surface_distance_for_elapsed(Wave::S, ev.depth_km, elapsed)
                        } else {
                            elapsed * 5.0
                        };
                        let r = s_km
                            .min(max_s_wave_spread_km(ev.magnitude))
                            .max(8.0);
                        // 勿设 9.0 地板：大波圈需要 zoom<9，否则 S 波出屏却不缩小
                        let z = zoom_to_fit_radius_km(
                            ev.latitude,
                            r,
                            map_rect.width().min(map_rect.height()),
                            0.42,
                        )
                        .clamp(3.0, 12.0);
                        let rate = if z < self.map.zoom - 0.8 {
                            0.22
                        } else {
                            0.12
                        };
                        self.map.apply_linear_zoom_out(
                            z,
                            rate,
                            Some(ev.longitude),
                            Some(ev.latitude),
                        );
                        ctx.request_repaint_after(std::time::Duration::from_millis(
                            cfg.performance.wave_repaint_ms(),
                        ));
                    }
                }
                draw_map_load_banner(ui, &p, self.map.load_state(), self.map.load_status());
                if let Some(kind) = self.settings.map_pick {
                    draw_map_pick_banner(ui, &p, kind);
                }
                draw_event_header(ui, &p, snap, scale.as_str(), &self.intensity_icons, travel);
                draw_tsunami_banner(ui, &p, snap, travel);
                let station_card_bottom = draw_realtime_intensity_card(ui, &p, snap);
                draw_typhoon_detail_below_stations(
                    ui,
                    &p,
                    map_ui.typhoon_hover.as_ref(),
                    typhoons_for_map,
                    station_card_bottom.bottom_y,
                );
                if let Some(net) = station_card_bottom.clicked {
                    if let Some((lon, lat, zoom)) = snap.focus_max_station(net) {
                        self.map.fly_to_centered(lon, lat, Some(zoom.clamp(7.5, 11.0)), 0.4);
                        self.map.user_camera_override = true;
                    }
                }
                if cfg.ui.show_legend {
                    draw_legend_bar(ui, &p);
                }
                draw_countdown_hud(ui, &p, snap, self.show_nied_clock, map_rect);
            });

        if let Some(act) = side_action {
            match act {
                SideAction::List(tab, idx) => {
                    if let Some((lon, lat, zoom)) = snap.select_list_item(tab, idx) {
                        let vp = ctx.screen_rect();
                        let map_w = (vp.width() - sidebar_w).max(120.0);
                        let map_h = vp.height().max(120.0);
                        match tab {
                            SidebarTab::Eew | SidebarTab::Records => {
                                let r_km = if tab == SidebarTab::Eew {
                                    snap.eew_list.get(idx).map(|e| {
                                        let max_km =
                                            max_s_spread_km(e.magnitude, &snap.wave_spread_km);
                                        let elapsed = if e.origin_ms > 0 {
                                            ((now_ms - e.origin_ms) as f64 / 1000.0).max(0.0)
                                        } else {
                                            0.0
                                        };
                                        let live = if let Some(eng) = travel {
                                            s_wave_still_spreading_to(
                                                eng, e.depth_km, elapsed, max_km,
                                            )
                                        } else {
                                            eew_wave_active_spread(
                                                e,
                                                now_ms,
                                                &snap.wave_spread_km,
                                            )
                                        };
                                        if !live {
                                            return 90.0;
                                        }
                                        let s_km = if let Some(eng) = travel {
                                            eng.surface_distance_for_elapsed(
                                                Wave::S,
                                                e.depth_km,
                                                elapsed,
                                            )
                                        } else {
                                            elapsed * 4.0
                                        };
                                        s_km.min(max_km).max(18.0)
                                    })
                                } else {
                                    Some(55.0)
                                }
                                .unwrap_or(55.0);
                                let end_z = zoom_to_fit_radius_km(
                                    lat,
                                    r_km,
                                    map_w.min(map_h),
                                    0.52,
                                )
                                .clamp(8.0, 12.5);
                                let dur = if tab == SidebarTab::Eew { 1.2 } else { 1.0 };
                                self.map.fly_to_centered(lon, lat, Some(end_z), dur);
                                // 手动点选：锁定镜头，避免影院轮播 / 跟焦抢走速报跳转
                                self.map.user_camera_override = true;
                            }
                            SidebarTab::Station => {
                                // 短飞跃 + 勿强行拉到 9.5（大缩放差会触发底图重建卡顿）
                                let z = zoom.clamp(7.5, 11.0);
                                self.map.fly_to_centered(lon, lat, Some(z), 0.4);
                                self.map.user_camera_override = true;
                            }
                            SidebarTab::Volcano => {
                                self.map.fly_to_centered(lon, lat, Some(zoom.max(9.0)), 0.85);
                                self.map.user_camera_override = true;
                            }
                        }
                        if snap.overlay_mode == OverlayMode::Wave {
                            self.map_elapsed = 0.0;
                        }
                    }
                }
                SideAction::Settings => {
                    self.settings.open_with(cfg);
                }
            }
        }

        if let Some((lon, lat)) = map_ui.click {
            self.apply_map_pick(lon, lat);
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.settings.map_pick = None;
        }
        if self.settings.capture_view {
            self.settings.apply_capture_view(
                round_coord(self.map.center_lon),
                round_coord(self.map.center_lat),
                round_zoom(self.map.zoom),
            );
        }

        let health = SourceHealthView {
            jian: snap.health_jian,
            wolfx: snap.health_wolfx,
            p2p: snap.health_p2p,
            pancakes: snap.health_pancakes,
            jian_enabled: cfg.sources.jian.enabled,
            wolfx_enabled: cfg.sources.wolfx.enabled,
            p2p_enabled: cfg.sources.p2pquake.enabled,
            pancakes_enabled: cfg.sources.pancakes.enabled,
        };
        self.settings.ui(ctx, cfg, &health)
    }

    fn apply_map_pick(&mut self, lon: f64, lat: f64) {
        let Some(kind) = self.settings.map_pick else {
            return;
        };
        self.settings.apply_map_pick_coords(
            kind,
            round_coord(lon),
            round_coord(lat),
            round_zoom(self.map.zoom),
        );
    }
}

fn round_coord(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

fn round_zoom(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn draw_map_pick_banner(ui: &mut Ui, p: &ThemePalette, kind: MapPickKind) {
    let text = match kind {
        MapPickKind::DefaultViewport => "点击地图设置默认视口（Esc 取消）",
        MapPickKind::LocalLocation => "点击地图设置本机位置（Esc 取消）",
    };
    let pos = ui.max_rect().center_top() + Vec2::new(-140.0, 44.0);
    let rect = egui::Rect::from_min_size(pos, Vec2::new(280.0, 40.0));
    overlay_child(ui, "map_pick_banner", rect, |ui| {
        Frame::NONE
            .fill(p.panel_glass_soft)
            .stroke(Stroke::new(1.0_f32, p.accent))
            .inner_margin(Margin::symmetric(12, 6))
            .corner_radius(8.0)
            .show(ui, |ui| {
                ui.label(RichText::new(text).size(13.0).color(p.text));
            });
    });
}

fn format_header_time(origin_ms: i64) -> String {
    if origin_ms <= 0 {
        return "----/--/-- --:--:--".into();
    }
    let Some(dt) = chrono::DateTime::from_timestamp_millis(origin_ms) else {
        return "----/--/-- --:--:--".into();
    };
    let cst = FixedOffset::east_opt(8 * 3600).unwrap();
    dt.with_timezone(&cst)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

fn format_list_time(origin_ms: i64) -> String {
    if origin_ms <= 0 {
        return "—".into();
    }
    let Some(dt) = chrono::DateTime::from_timestamp_millis(origin_ms) else {
        return "—".into();
    };
    let cst = FixedOffset::east_opt(8 * 3600).unwrap();
    dt.with_timezone(&cst).format("%Y-%m-%d %H:%M").to_string()
}

fn rgb(c: jian_core::palette::Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

#[allow(dead_code)]
fn glass_frame(p: &ThemePalette) -> Frame {
    Frame::NONE
        .fill(p.panel_glass)
        .stroke(Stroke::new(1.0_f32, p.panel_border))
        .inner_margin(Margin::symmetric(12, 10))
        .corner_radius(10.0)
}

fn intensity_chip(
    kind: IntensityKind,
    level: u8,
    text: &str,
    scale: &str,
) -> (IntensityKind, u8, Color32, String) {
    let (k, lv, label) = prefer_intensity_scale(kind, level, text, scale);
    (k, lv, rgb(color_for(k, lv)), label)
}

/// 绘制烈度/震度图标；无贴图时回退色块+文案。
fn paint_intensity(
    ui: &mut Ui,
    icons: &IntensityIcons,
    kind: IntensityKind,
    level: u8,
    text: &str,
    scale: &str,
    size: f32,
    instrumental: bool,
) {
    let (k, lv, col, label) = intensity_chip(kind, level, text, scale);
    let (resp, painter) = ui.allocate_painter(Vec2::splat(size), Sense::hover());
    if let Some(tex) = icons.get_ex(k, lv, instrumental) {
        painter.image(
            tex.id(),
            resp.rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    } else {
        painter.rect_filled(resp.rect, 4.0, col);
        painter.text(
            resp.rect.center(),
            egui::Align2::CENTER_CENTER,
            &label,
            FontId::proportional(if label.chars().count() > 2 {
                size * 0.28
            } else {
                size * 0.45
            }),
            Color32::BLACK,
        );
    }
}

fn draw_map_load_banner(ui: &mut Ui, p: &ThemePalette, state: BasemapLoadState, status: &str) {
    let text = match state {
        BasemapLoadState::Empty => return,
        BasemapLoadState::Ready => return,
        BasemapLoadState::LoadingWorld | BasemapLoadState::LoadingRegional => {
            if status.is_empty() {
                "正在加载底图…"
            } else {
                status
            }
        }
        BasemapLoadState::Failed => {
            if status.is_empty() {
                "底图加载失败"
            } else {
                status
            }
        }
    };
    let color = if state == BasemapLoadState::Failed {
        p.danger
    } else {
        p.text_muted
    };
    let pos = ui.max_rect().center_top() + Vec2::new(-120.0, 10.0);
    let rect = egui::Rect::from_min_size(pos, Vec2::new(260.0, 36.0));
    overlay_child(ui, "map_load_banner", rect, |ui| {
        Frame::NONE
            .fill(p.panel_glass_soft)
            .stroke(Stroke::new(1.0_f32, p.panel_border))
            .inner_margin(Margin::symmetric(12, 6))
            .corner_radius(8.0)
            .show(ui, |ui| {
                ui.label(RichText::new(text).size(12.0).color(color));
            });
    });
}

/// 左上预警框与右上实时震度卡共用顶边。
const OVERLAY_TOP_Y: f32 = 12.0;

/// 左上预警条：每个侧栏源槽只留一条（与列表一致）；GQ 优先当前焦点。
fn header_live_eews<'a>(
    snap: &'a AppSnapshot,
    travel: Option<&TravelEngine>,
) -> Vec<&'a EewReport> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let live = live_wave_eews_travel(&snap.eew_list, now_ms, &snap.wave_spread_km, travel);
    let mut best: Vec<&EewReport> = Vec::new();
    for e in live {
        let key = jian_core::eew_sidebar_group_key(&e.agency.0);
        if let Some(pos) = best
            .iter()
            .position(|b| jian_core::eew_sidebar_group_key(&b.agency.0) == key)
        {
            let cur = best[pos];
            let prefer_focus = snap.active.as_ref().is_some_and(|a| {
                a.agency.0 == e.agency.0 && a.event_id == e.event_id
            });
            let cur_focus = snap.active.as_ref().is_some_and(|a| {
                a.agency.0 == cur.agency.0 && a.event_id == cur.event_id
            });
            if prefer_focus
                || (!cur_focus
                    && (e.origin_ms > cur.origin_ms
                        || (e.origin_ms == cur.origin_ms && e.serial >= cur.serial)))
            {
                best[pos] = e;
            }
        } else {
            best.push(e);
        }
    }
    // GQ 槽若有焦点则置顶，其余按发震新→旧
    best.sort_by(|a, b| {
        let a_gq = jian_core::agency_family(&a.agency.0) == jian_core::AgencyFamily::Gq;
        let b_gq = jian_core::agency_family(&b.agency.0) == jian_core::AgencyFamily::Gq;
        b_gq.cmp(&a_gq).then_with(|| b.origin_ms.cmp(&a.origin_ms))
    });
    best
}

/// 左上角预警条总高度（海啸条避让）。
fn eew_header_stack_height(snap: &AppSnapshot, travel: Option<&TravelEngine>) -> f32 {
    let n = header_live_eews(snap, travel).len();
    if n == 0 {
        return 0.0;
    }
    // 单条约 96px + 条间距 6
    96.0 * n as f32 + 6.0 * (n.saturating_sub(1) as f32)
}

fn draw_event_header(
    ui: &mut Ui,
    _p: &ThemePalette,
    snap: &AppSnapshot,
    scale: &str,
    icons: &IntensityIcons,
    travel: Option<&TravelEngine>,
) {
    let live = header_live_eews(snap, travel);
    if live.is_empty() {
        return;
    }

    const LEFT_PANEL_X: f32 = 12.0;
    const LEFT_PANEL_W: f32 = 340.0;
    const ROW_GAP: f32 = 6.0;
    let pos = ui.max_rect().left_top() + Vec2::new(LEFT_PANEL_X, OVERLAY_TOP_Y);
    let rect = egui::Rect::from_min_size(
        pos,
        Vec2::new(LEFT_PANEL_W + 24.0, ui.max_rect().height().max(120.0)),
    );
    overlay_child(ui, "event_header", rect, |ui| {
            ui.spacing_mut().item_spacing.y = ROW_GAP;
            ui.vertical(|ui| {
                for (i, ev) in live.iter().enumerate() {
                    let agency = agency_bracket(&ev.agency.0);
                    let idx_mark = if live.len() > 1 {
                        format!("{} ", circled_index(i + 1))
                    } else {
                        String::new()
                    };
                    let title = if ev.is_cancel {
                        format!("{idx_mark}{} ({agency}) \u{00b7} 取消", ev.place)
                    } else if ev.serial > 0 && ev.serial < 10_000 {
                        format!("{idx_mark}{} ({agency}) #{}", ev.place, ev.serial)
                    } else {
                        format!("{idx_mark}{} ({agency})", ev.place)
                    };
                    let time_line = format_header_time(ev.origin_ms);
                    let mag_line = if ev.is_assumption && ev.magnitude <= 0.0 {
                        "调查中".to_string()
                    } else {
                        format!("M{:.1}", ev.magnitude)
                    };
                    let depth_line = if ev.is_assumption
                        && ev.latitude.abs() < 0.01
                        && ev.longitude.abs() < 0.01
                    {
                        "调查中".to_string()
                    } else {
                        format!("{:.0}km", ev.depth_km)
                    };
                    let focused = snap.active.as_ref().is_some_and(|a| {
                        a.agency.0 == ev.agency.0 && a.event_id == ev.event_id
                    });
                    let stroke = if focused {
                        Stroke::new(1.5_f32, Color32::from_rgb(0x7D, 0xF9, 0xFF))
                    } else {
                        Stroke::new(1.0_f32, Color32::from_rgb(0x55, 0x55, 0x66))
                    };
                    Frame::NONE
                        .fill(Color32::from_rgba_unmultiplied(28, 28, 32, 230))
                        .stroke(stroke)
                        .inner_margin(Margin::symmetric(10, 8))
                        .corner_radius(6.0)
                        .show(ui, |ui| {
                            ui.set_width(LEFT_PANEL_W);
                            ui.horizontal(|ui| {
                                paint_intensity(
                                    ui,
                                    icons,
                                    ev.intensity_kind,
                                    ev.intensity_level,
                                    &ev.max_intensity_text,
                                    scale,
                                    72.0,
                                    false,
                                );
                                ui.add_space(10.0);
                                ui.vertical(|ui| {
                                    ui.set_min_width(180.0);
                                    ui.label(
                                        RichText::new(title)
                                            .color(Color32::WHITE)
                                            .strong()
                                            .size(16.0),
                                    );
                                    ui.add_space(2.0);
                                    ui.label(
                                        RichText::new(time_line)
                                            .color(Color32::WHITE)
                                            .size(14.0)
                                            .monospace(),
                                    );
                                    ui.add_space(2.0);
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(mag_line)
                                                .color(Color32::WHITE)
                                                .strong()
                                                .size(15.0),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.label(
                                                    RichText::new(depth_line)
                                                        .color(Color32::from_rgb(
                                                            0xDD, 0xDD, 0xEE,
                                                        ))
                                                        .size(14.0),
                                                );
                                            },
                                        );
                                    });
                                });
                            });
                        });
                }
            });
        });
}

/// ①②③… 序号（多震堆叠，对齐 EEWCN）。
fn circled_index(n: usize) -> char {
    match n {
        1 => '\u{2460}',
        2 => '\u{2461}',
        3 => '\u{2462}',
        4 => '\u{2463}',
        5 => '\u{2464}',
        6 => '\u{2465}',
        7 => '\u{2466}',
        8 => '\u{2467}',
        9 => '\u{2468}',
        10 => '\u{2469}',
        _ => '\u{00b7}',
    }
}

fn draw_tsunami_banner(
    ui: &mut Ui,
    p: &ThemePalette,
    snap: &AppSnapshot,
    travel: Option<&TravelEngine>,
) {
    let Some(t) = snap.tsunami.as_ref() else {
        return;
    };
    if t.cancelled {
        return;
    }
    // 仅 JMA：NMEFC/CENC 类「海啸信息」报文不进顶条
    if t.areas_list.is_empty() && t.title.contains("海啸信息") {
        return;
    }
    let (r, g, b) = jian_core::tsunami_official_rgb(&t.grade);
    let accent = Color32::from_rgb(r, g, b);
    let title = if t.title.is_empty() {
        jian_core::tsunami_grade_zh(&t.grade).to_string()
    } else {
        t.title.clone()
    };
    let desc = match t.grade.as_str() {
        "MajorWarning" | "Major" => "大海啸警报：请立即撤离到高处或坚固建筑。",
        "Warning" => "海啸警报：海岸附近请立即撤离。",
        "Watch" => "海啸注意报：注意海面异常，远离海岸。",
        _ => "海面可能发生些许变动，但无需采取特殊防灾措施。",
    };
    // 预报/注意报用深底+色框；警报级用官方色实底
    let (fill, border, title_c, body_c) = match t.grade.as_str() {
        "MajorWarning" | "Major" | "Warning" => (
            Color32::from_rgba_unmultiplied(r, g, b, 230),
            accent,
            Color32::WHITE,
            Color32::from_rgb(0xF8, 0xFA, 0xFC),
        ),
        "Watch" => (
            Color32::from_rgba_unmultiplied(20, 20, 24, 230),
            accent,
            Color32::from_rgb(0xFF, 0xFF, 0x66),
            Color32::from_rgb(0xE5, 0xE7, 0xEB),
        ),
        _ => (
            Color32::from_rgba_unmultiplied(12, 24, 40, 235),
            accent,
            Color32::from_rgb(0x7D, 0xF9, 0xFF),
            Color32::from_rgb(0xD1, 0xD5, 0xDB),
        ),
    };
    // 有预警信息头时下移（按堆叠条数）；无预警时贴到左上角
    let top_y = {
        let h = eew_header_stack_height(snap, travel);
        if h > 0.0 {
            OVERLAY_TOP_Y + h + 8.0
        } else {
            OVERLAY_TOP_Y
        }
    };
    const LEFT_PANEL_X: f32 = 12.0;
    const LEFT_PANEL_W: f32 = 340.0;
    let pos = ui.max_rect().left_top() + Vec2::new(LEFT_PANEL_X, top_y);
    let rect = egui::Rect::from_min_size(pos, Vec2::new(LEFT_PANEL_W + 24.0, 220.0));
    overlay_child(ui, "tsunami_banner", rect, |ui| {
            Frame::NONE
                .fill(fill)
                .stroke(Stroke::new(2.0_f32, border))
                .corner_radius(6.0)
                .inner_margin(Margin::symmetric(10, 10))
                .show(ui, |ui| {
                    ui.set_width(LEFT_PANEL_W);
                    ui.label(
                        RichText::new(format!("{title}生效中"))
                            .color(title_c)
                            .strong()
                            .size(16.0),
                    );
                    ui.add_space(4.0);
                    ui.label(RichText::new(desc).color(body_c).size(12.5));
                    if t.expire_ms > 0 {
                        ui.add_space(2.0);
                        let exp = chrono::DateTime::from_timestamp_millis(t.expire_ms)
                            .map(|dt| {
                                dt.with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
                                    .format("预计解除 %Y/%m/%d %H:%M")
                                    .to_string()
                            })
                            .unwrap_or_else(|| "预计解除 —".into());
                        ui.label(RichText::new(exp).color(body_c).size(11.5));
                    }
                    ui.add_space(6.0);
                    let line = ui.available_width();
                    let (resp, painter) = ui.allocate_painter(Vec2::new(line, 2.0), Sense::hover());
                    painter.rect_filled(resp.rect, 0.0, border);
                    ui.add_space(4.0);
                    let area_line = if t.areas.is_empty() || t.areas == "—" {
                        jian_core::tsunami_grade_zh(&t.grade).to_string()
                    } else {
                        t.areas.clone()
                    };
                    ui.label(RichText::new(area_line).color(body_c).size(12.0));
                });
            let _ = p;
        });
}


/// 台风实况详情：固定在测站卡片下方，避免地图锚点浮层带尾巴。
fn draw_typhoon_detail_below_stations(
    ui: &mut Ui,
    p: &ThemePalette,
    hover: Option<&TyphoonHoverHit>,
    typhoons: &[jian_core::TyphoonInfo],
    station_bottom_y: f32,
) {
    let hit_owned;
    let hit: Option<&TyphoonHoverHit> = if let Some(h) = hover {
        Some(h)
    } else if let Some(tf) = typhoons.first() {
        let last = tf.points.last();
        let time_text = last.map(|x| x.time_text.clone()).unwrap_or_default();
        let strong = if tf.strong.is_empty() {
            last.map(|x| x.strong.clone()).unwrap_or_default()
        } else {
            tf.strong.clone()
        };
        hit_owned = TyphoonHoverHit {
            anchor: egui::pos2(0.0, 0.0),
            id: tf.id.clone(),
            name: tf.name.clone(),
            strong,
            power: if tf.power > 0 {
                tf.power
            } else {
                last.map(|x| x.power).unwrap_or(0)
            },
            time_text,
            longitude: tf.longitude,
            latitude: tf.latitude,
            speed_ms: last
                .map(|x| x.speed_ms)
                .filter(|&v| v > 0.0)
                .unwrap_or(tf.wind_speed_ms),
            pressure_hpa: last
                .map(|x| x.pressure_hpa)
                .filter(|&v| v > 0.0)
                .unwrap_or(tf.pressure_hpa),
            move_direction: last
                .map(|x| x.move_direction.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| tf.move_direction.clone()),
            move_speed_kmh: last
                .map(|x| x.move_speed_kmh)
                .filter(|&v| v > 0.0)
                .unwrap_or(tf.move_speed_kmh),
            is_center: true,
        };
        Some(&hit_owned)
    } else {
        None
    };
    let Some(hit) = hit else {
        return;
    };

    let time = format_typhoon_time_ui(&hit.time_text);
    let header = if time.is_empty() {
        format!("{} {}", hit.strong, hit.name)
    } else {
        format!("{} {}  {}", hit.strong, hit.name, time)
    };
    let power = if hit.power > 0 {
        format!("{}级", hit.power)
    } else {
        "\u{2014}".into()
    };
    let speed = if hit.speed_ms > 0.0 {
        format!("{:.0} m/s ({power})", hit.speed_ms)
    } else {
        format!("\u{2014} ({power})")
    };
    let pressure = if hit.pressure_hpa > 0.0 {
        format!("{:.0} hPa", hit.pressure_hpa)
    } else {
        "\u{2014}".into()
    };
    let mv = if hit.move_direction.is_empty() && hit.move_speed_kmh <= 0.0 {
        "\u{2014}".into()
    } else {
        format!(
            "{} ({:.0} KM/H)",
            if hit.move_direction.is_empty() {
                "\u{2014}"
            } else {
                hit.move_direction.as_str()
            },
            hit.move_speed_kmh
        )
    };
    let lines = [
        format!("编号: {}", hit.id),
        format!(
            "中心位置: ({:.1}\u{00b0}N, {:.1}\u{00b0}E)",
            hit.latitude, hit.longitude
        ),
        format!("最大风速: {speed}"),
        format!("中心气压: {pressure}"),
        format!("移动速度: {mv}"),
    ];

    // 与右上三测站卡片同宽对齐，并与卡片底边留足间距，避免贴地图
    const CARD_W: f32 = 240.0; // 76*3 + 6*2
    let pad = 8.0_f32;
    let line_h = 16.0_f32;
    let header_h = 26.0_f32;
    let body_h = pad + line_h * lines.len() as f32 + 4.0;
    let total_h = header_h + 2.0 + body_h;
    let right = ui.max_rect().right() - 12.0;
    let origin = egui::pos2(right - CARD_W, station_bottom_y + 14.0);
    let rect = egui::Rect::from_min_size(origin, Vec2::new(CARD_W, total_h));
    let (r, g, b) = jian_core::typhoon_color_rgb(&hit.strong, hit.power);
    overlay_child(ui, "typhoon_detail_card", rect, |ui| {
        let painter = ui.painter();
        let header_rect = egui::Rect::from_min_size(rect.min, Vec2::new(CARD_W, header_h));
        let body_rect = egui::Rect::from_min_size(
            egui::pos2(rect.min.x, header_rect.bottom() + 2.0),
            Vec2::new(CARD_W, body_h),
        );
        painter.rect_filled(header_rect, 4.0, Color32::from_rgb(r, g, b).gamma_multiply(0.85));
        painter.rect_stroke(
            header_rect,
            4.0,
            Stroke::new(1.0_f32, Color32::from_rgb(0xAA, 0xAA, 0xBB)),
            egui::StrokeKind::Outside,
        );
        painter.rect_filled(
            body_rect,
            4.0,
            Color32::from_rgba_unmultiplied(0x12, 0x14, 0x1C, 0xF0),
        );
        painter.rect_stroke(
            body_rect,
            4.0,
            Stroke::new(1.0_f32, Color32::from_rgb(0x44, 0x48, 0x58)),
            egui::StrokeKind::Outside,
        );
        painter.text(
            egui::pos2(header_rect.min.x + pad, header_rect.center().y),
            egui::Align2::LEFT_CENTER,
            header,
            FontId::proportional(13.0),
            Color32::WHITE,
        );
        let mut y = body_rect.min.y + pad * 0.6;
        for line in &lines {
            painter.text(
                egui::pos2(body_rect.min.x + pad, y),
                egui::Align2::LEFT_TOP,
                line,
                FontId::proportional(12.0),
                p.text,
            );
            y += line_h;
        }
    });
}

fn format_typhoon_time_ui(time_str: &str) -> String {
    let bytes = time_str.as_bytes();
    let mut i = 0usize;
    while i + 10 <= bytes.len() {
        if bytes[i].is_ascii_digit()
            && bytes[i + 1].is_ascii_digit()
            && bytes[i + 2].is_ascii_digit()
            && bytes[i + 3].is_ascii_digit()
            && (bytes[i + 4] == b'-' || bytes[i + 4] == b'/')
        {
            let day = std::str::from_utf8(&bytes[i + 8..i + 10])
                .ok()
                .and_then(|s| s.parse::<u32>().ok());
            let mut j = i + 10;
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'T') {
                j += 1;
            }
            if let Some(day) = day {
                if j + 2 <= bytes.len()
                    && bytes[j].is_ascii_digit()
                    && bytes[j + 1].is_ascii_digit()
                {
                    if let Ok(hour) = std::str::from_utf8(&bytes[j..j + 2])
                        .unwrap_or("")
                        .parse::<u32>()
                    {
                        return format!("{day}日{hour}时");
                    }
                }
            }
        }
        i += 1;
    }
    time_str.to_string()
}

/// 测站网实时震度卡片布局结果。
struct StationCardLayout {
    clicked: Option<&'static str>,
    /// 卡片底边 Y；无卡片时为顶栏下方起点，供台风详情承接。
    bottom_y: f32,
}

/// 地图右上角实时震度/烈度卡片（对齐 EEWCN sourceMaxCard）。
/// 点击卡片返回台网 key，供跳转到该网最大测站。
fn draw_realtime_intensity_card(
    ui: &mut Ui,
    _p: &ThemePalette,
    snap: &AppSnapshot,
) -> StationCardLayout {
    let top_y = ui.max_rect().right_top().y + OVERLAY_TOP_Y;
    if snap.stations.is_empty() {
        return StationCardLayout {
            clicked: None,
            bottom_y: top_y,
        };
    }
    // (key, label, max_value, is_kma_mmi, ts)
    let mut cards: Vec<(&'static str, &'static str, f64, bool, i64)> = Vec::new();
    for (key, label) in [("kmoni", "Kmoni"), ("snet", "S-Net"), ("kma", "KMA")] {
        let mut max_v = -99.0_f64;
        let mut any = false;
        let is_kma = key == "kma";
        for s in &snap.stations {
            if !s.id.starts_with(&format!("{key}:")) {
                continue;
            }
            any = true;
            let v = if is_kma {
                s.intensity_level as f64
            } else {
                s.instrumental
            };
            if v > max_v {
                max_v = v;
            }
        }
        if any {
            let ts = snap.station_updated_ms.get(key).copied().unwrap_or(0);
            cards.push((key, label, max_v, is_kma, ts));
        }
    }
    if cards.is_empty() {
        let max_inst = snap
            .stations
            .iter()
            .map(|s| s.instrumental)
            .fold(-99.0, f64::max);
        let ts = snap
            .station_updated_ms
            .values()
            .copied()
            .max()
            .unwrap_or(0);
        cards.push(("all", "NIED", max_inst, false, ts));
    }

    const CARD_W: f32 = 76.0;
    const CARD_H: f32 = 88.0;
    let gap = 6.0_f32;
    let now_ms = chrono::Utc::now().timestamp_millis();
    const STALE_MS: i64 = 8_000;
    let total_w = CARD_W * cards.len() as f32 + gap * (cards.len().saturating_sub(1) as f32);
    let top_right = ui.max_rect().right_top() + Vec2::new(-12.0, OVERLAY_TOP_Y);
    let rect = egui::Rect::from_min_size(
        egui::pos2(top_right.x - total_w.max(72.0), top_right.y),
        Vec2::new(total_w.max(72.0), CARD_H),
    );
    let mut clicked: Option<&'static str> = None;
    overlay_child(ui, "rt_intensity_card", rect, |ui| {
        ui.set_min_size(Vec2::new(total_w, CARD_H));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (k, label, val, is_kma, ts) in &cards {
                let (value, value_color) = if *is_kma {
                    if *val >= 1.0 {
                        let lv = (*val).round().clamp(1.0, 12.0) as u8;
                        let rgb = jian_core::palette::cn_intensity_rgb(lv);
                        (format!("{lv}"), Color32::from_rgb(rgb.r, rgb.g, rgb.b))
                    } else {
                        ("-".into(), Color32::from_rgb(0x66, 0x66, 0x66))
                    }
                } else if *val >= -3.0 {
                    let rgb = jian_core::instrumental_rgb(*val);
                    (
                        jian_core::instrumental_half_step_text(*val),
                        Color32::from_rgb(rgb.r, rgb.g, rgb.b),
                    )
                } else {
                    ("-".into(), Color32::from_rgb(0x66, 0x66, 0x66))
                };
                let time_text = if *ts > 0 {
                    format_clock_hms(*ts)
                } else {
                    "--:--:--".into()
                };
                let stale = if *k == "snet" { 90_000_i64 } else { STALE_MS };
                let connected = *ts > 0 && (now_ms - *ts).abs() <= stale;
                let time_color = if connected {
                    Color32::WHITE
                } else {
                    Color32::from_rgb(0xFF, 0x44, 0x44)
                };

                let (r, resp) =
                    ui.allocate_exact_size(Vec2::new(CARD_W, CARD_H), Sense::click());
                if resp.clicked() {
                    clicked = Some(*k);
                }
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                let painter = ui.painter();
                painter.rect_filled(r, 6.0, Color32::from_rgb(0x1A, 0x1A, 0x1A));
                let head = egui::Rect::from_min_size(r.min, Vec2::new(r.width(), 20.0));
                painter.rect_filled(head, 0.0, Color32::from_rgb(0x2D, 0x2D, 0x2D));
                painter.rect_stroke(
                    r,
                    6.0,
                    Stroke::new(1.0_f32, Color32::from_rgb(0x40, 0x40, 0x40)),
                    egui::StrokeKind::Inside,
                );
                painter.hline(
                    r.x_range(),
                    head.bottom(),
                    Stroke::new(1.0_f32, Color32::from_rgb(0x40, 0x40, 0x40)),
                );
                painter.text(
                    head.center(),
                    egui::Align2::CENTER_CENTER,
                    *label,
                    FontId::proportional(12.0),
                    Color32::WHITE,
                );
                let body = egui::Rect::from_min_max(
                    egui::pos2(r.left(), head.bottom()),
                    egui::pos2(r.right(), r.bottom() - 18.0),
                );
                // 加粗：双层叠绘
                let value_font = FontId::new(28.0, FontFamily::Proportional);
                painter.text(
                    body.center() + Vec2::new(0.35, 0.0),
                    egui::Align2::CENTER_CENTER,
                    &value,
                    value_font.clone(),
                    value_color,
                );
                painter.text(
                    body.center(),
                    egui::Align2::CENTER_CENTER,
                    &value,
                    value_font,
                    value_color,
                );
                painter.text(
                    egui::pos2(r.center().x, r.bottom() - 10.0),
                    egui::Align2::CENTER_CENTER,
                    time_text,
                    FontId::proportional(11.0),
                    time_color,
                );
            }
        });
    });
    StationCardLayout {
        clicked,
        bottom_y: rect.bottom(),
    }
}



fn format_clock_hms(ms: i64) -> String {
    use chrono::{TimeZone, Utc};
    let Some(dt) = Utc.timestamp_millis_opt(ms).single() else {
        return "--:--:--".into();
    };
    let local = dt.with_timezone(&Local);
    local.format("%H:%M:%S").to_string()
}


fn draw_legend_bar(ui: &mut Ui, p: &ThemePalette) {
    // 样式：  图例
    //      12□ □7
    // 数字紧贴色块；左列个位右对齐；右列强弱用 +/-
    const ROW: f32 = 17.0;
    const SW: f32 = 14.0;
    const MID: f32 = 2.0;
    const LEFT_W: f32 = 18.0;
    const RIGHT_W: f32 = 28.0;
    const CN_N: usize = 12;
    const SH_N: usize = 13;
    const SH_ROWS: [(&str, u8); SH_N] = [
        ("7", 20),
        ("6+", 19),
        ("6-", 18),
        ("5+", 17),
        ("5-", 16),
        ("4", 15),
        ("3", 13),
        ("2", 11),
        ("1", 9),
        ("0", 7),
        ("-1", 5),
        ("-2", 3),
        ("-3", 0),
    ];
    let width = LEFT_W + SW + MID + SW + RIGHT_W;
    let rows = CN_N.max(SH_N);
    let body_h = ROW * rows as f32;
    let bottom_left = ui.max_rect().left_bottom() + Vec2::new(4.0, -4.0);
    let h = body_h + 22.0;
    let rect = egui::Rect::from_min_max(
        egui::pos2(bottom_left.x, bottom_left.y - h),
        egui::pos2(bottom_left.x + width, bottom_left.y),
    );
    overlay_child(ui, "legend_bar", rect, |ui| {
        ui.spacing_mut().window_margin = Margin::ZERO;
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        ui.set_min_size(Vec2::new(width, h));
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.set_width(width);
            ui.label(
                RichText::new("图例")
                    .color(p.text_muted)
                    .strong()
                    .size(15.0),
            );
        });
        let (resp, painter) = ui.allocate_painter(Vec2::new(width, body_h), Sense::hover());
        let r = resp.rect;
        let cn_sw_x = r.left() + LEFT_W;
        let sh_sw_x = cn_sw_x + SW + MID;

        for i in 0..CN_N {
            let lv = (12 - i) as u8;
            let y0 = r.top() + ROW * i as f32;
            let cy = y0 + ROW * 0.5;
            painter.text(
                egui::pos2(cn_sw_x, cy),
                egui::Align2::RIGHT_CENTER,
                format!("{lv}"),
                FontId::proportional(12.0),
                p.text_dim,
            );
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(cn_sw_x, y0 + 1.5),
                    egui::pos2(cn_sw_x + SW, y0 + ROW - 1.5),
                ),
                1.5,
                rgb(cn_intensity_rgb(lv)),
            );
        }

        for i in 0..SH_N {
            let (label, band) = SH_ROWS[i];
            let y0 = r.top() + ROW * i as f32;
            let cy = y0 + ROW * 0.5;
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(sh_sw_x, y0 + 1.5),
                    egui::pos2(sh_sw_x + SW, y0 + ROW - 1.5),
                ),
                1.5,
                rgb(instrumental_band_rgb(band)),
            );
            painter.text(
                egui::pos2(sh_sw_x + SW, cy),
                egui::Align2::LEFT_CENTER,
                label,
                FontId::proportional(12.0),
                p.text_muted,
            );
        }
    });
}

fn draw_countdown_hud(
    ui: &mut Ui,
    p: &ThemePalette,
    snap: &AppSnapshot,
    show_nied: bool,
    map_rect: egui::Rect,
) {
    let local = Local::now();
    let clock = local.format("%Y/%m/%d %H:%M:%S").to_string();
    let jst_clock = if show_nied {
        let jst = FixedOffset::east_opt(9 * 3600).unwrap();
        Some(
            local
                .with_timezone(&jst)
                .format("%Y/%m/%d %H:%M:%S")
                .to_string(),
        )
    } else {
        None
    };

    // 贴地图视口右下角；时钟用绝对坐标绘制，避免 bottom_up 嵌套把标题/时间叠成重影
    let br = map_rect.right_bottom() + Vec2::new(-4.0, -4.0);
    let hud_w = 280.0_f32;
    let clock_block_h = 36.0_f32;
    let n_clocks = 1 + usize::from(show_nied);
    let countdown_h = if snap.countdown_s.is_some() { 72.0 } else { 0.0 };
    let gap = 6.0_f32;
    let hud_h = countdown_h + clock_block_h * n_clocks as f32 + gap + 8.0;
    let rect = egui::Rect::from_min_size(
        egui::pos2(br.x - hud_w, br.y - hud_h),
        Vec2::new(hud_w, hud_h),
    );
    overlay_child(ui, "countdown_hud", rect, |ui| {
        ui.spacing_mut().window_margin = Margin::ZERO;
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        let r = ui.max_rect();
        let mut y_bottom = r.bottom() - 2.0;

        // 自下而上：北京时间 →（可选）东京时间
        y_bottom = paint_tz_clock_at(ui, r.right() - 2.0, y_bottom, "北京时间", &clock);
        if let Some(jst) = jst_clock.as_ref() {
            y_bottom = paint_tz_clock_at(ui, r.right() - 2.0, y_bottom, "东京时间", jst);
        }

        if let Some(sec) = snap.countdown_s {
            let cd_top = (y_bottom - gap - countdown_h).max(r.top());
            let cd_rect = egui::Rect::from_min_max(
                egui::pos2(r.left(), cd_top),
                egui::pos2(r.right(), y_bottom - gap),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(cd_rect), |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                    let title = if sec > 0 { "S波即将到达" } else { "S波已到达" };
                    outlined_label(ui, title, Color32::WHITE, 14.0, false);
                    ui.allocate_ui_with_layout(
                        Vec2::new(ui.available_width(), 44.0),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            let mm = sec / 60;
                            let ss = sec % 60;
                            let time_txt = format!("{mm}:{ss:02}");
                            let time_color = if sec > 10 {
                                Color32::from_rgb(0xFF, 0xEB, 0x3B)
                            } else {
                                p.danger
                            };
                            Frame::NONE
                                .fill(Color32::from_rgba_unmultiplied(0x66, 0x66, 0x66, 0x80))
                                .inner_margin(Margin::symmetric(6, 2))
                                .show(ui, |ui| {
                                    outlined_label(ui, &time_txt, time_color, 34.0, false);
                                });
                            let lv = snap.countdown_local_level;
                            let label = if snap.countdown_local_text.is_empty() {
                                if lv == 0 {
                                    "0".into()
                                } else {
                                    lv.to_string()
                                }
                            } else {
                                snap.countdown_local_text.clone()
                            };
                            let bg = if lv == 0 {
                                Color32::from_rgb(0x2A, 0x2A, 0x2E)
                            } else {
                                rgb(color_for(snap.countdown_local_kind, lv))
                            };
                            let fg = intensity_fg(bg);
                            let border = if lv == 0 {
                                Stroke::new(1.0_f32, Color32::WHITE)
                            } else {
                                Stroke::NONE
                            };
                            Frame::NONE
                                .fill(bg)
                                .stroke(border)
                                .inner_margin(Margin::symmetric(8, 2))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(label)
                                            .color(fg)
                                            .strong()
                                            .size(34.0),
                                    );
                                });
                        },
                    );
                });
            });
        }
    });
}

/// 在右下角绝对位置绘制一块时钟；返回下一块可用的 bottom y。
fn paint_tz_clock_at(ui: &mut Ui, right: f32, bottom: f32, title: &str, datetime: &str) -> f32 {
    let h = 36.0_f32;
    let top = bottom - h;
    let x = right.floor();
    let painter = ui.painter();
    let title_font = FontId::new(13.0, FontFamily::Proportional);
    let time_font = FontId::new(15.0, FontFamily::Proportional);
    painter.text(
        egui::pos2(x, (top + 12.0).floor()),
        egui::Align2::RIGHT_CENTER,
        title,
        title_font,
        Color32::from_rgb(0xCC, 0xCC, 0xCC),
    );
    painter.text(
        egui::pos2(x, (top + 28.0).floor()),
        egui::Align2::RIGHT_CENTER,
        datetime,
        time_font,
        Color32::WHITE,
    );
    top - 2.0
}

fn intensity_fg(bg: Color32) -> Color32 {
    let y = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
    if y > 160.0 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

fn outlined_label(ui: &mut Ui, text: &str, color: Color32, size: f32, monospace: bool) {
    let font = if monospace {
        FontId::monospace(size)
    } else {
        FontId::proportional(size)
    };
    let galley = ui.fonts(|f| f.layout_no_wrap(text.to_string(), font, color));
    let size_v = galley.size();
    let (resp, painter) = ui.allocate_painter(size_v, Sense::hover());
    let pos = resp.rect.min;
    // 仅四向细描边，避免八向叠绘把笔画撑成「加粗」
    let stroke = Color32::from_rgba_unmultiplied(0, 0, 0, 160);
    for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
        painter.galley(pos + Vec2::new(dx, dy), galley.clone(), stroke);
    }
    painter.galley(pos, galley, color);
}

enum SideAction {
    List(SidebarTab, usize),
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RowClick {
    None,
    Primary,
}

fn health_dot(p: &ThemePalette, status: jian_core::HealthStatus) -> Color32 {
    match status {
        jian_core::HealthStatus::Normal => p.ok,
        jian_core::HealthStatus::Fluctuating => p.warn,
        jian_core::HealthStatus::Abnormal => p.danger,
    }
}

/// 返回侧栏操作
fn draw_sidebar(
    ui: &mut Ui,
    p: &ThemePalette,
    snap: &mut AppSnapshot,
    scale: &str,
    icons: &IntensityIcons,
    show_volcano_icons: bool,
    source_enabled: [bool; 4],
) -> Option<SideAction> {
    let mut clicked = None;
    ui.set_min_width(ui.available_width());
    Frame::NONE
        .fill(p.sidebar_bg)
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("地震情报实况")
                        .color(p.text)
                        .strong()
                        .size(16.0),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("设置").size(13.0).color(p.text_muted))
                                .fill(p.tab_idle)
                                .corner_radius(6.0)
                                .min_size(Vec2::new(52.0, 26.0)),
                        )
                        .clicked()
                    {
                        clicked = Some(SideAction::Settings);
                    }
                });
            });

            ui.add_space(6.0);
            // 仅色点 + 源名；状态文案放悬停，避免四源挤成一团
            ui.horizontal(|ui| {
                for (i, (name, st)) in [
                    ("Jian", snap.health_jian),
                    ("Wolfx", snap.health_wolfx),
                    ("P2P", snap.health_p2p),
                    ("Pancakes", snap.health_pancakes),
                ]
                .into_iter()
                .enumerate()
                {
                    let on = source_enabled[i];
                    let tip = if on {
                        format!("{name}\u{00b7}{}", st.as_zh())
                    } else {
                        format!("{name}\u{00b7}关闭")
                    };
                    let dot = if on {
                        health_dot(p, st)
                    } else {
                        p.text_dim
                    };
                    let resp = ui
                        .horizontal(|ui| {
                            let (r, painter) =
                                ui.allocate_painter(Vec2::splat(9.0), Sense::hover());
                            painter.circle_filled(r.rect.center(), 3.8, dot);
                            ui.label(RichText::new(name).size(12.5).color(p.text_dim));
                        })
                        .response;
                    resp.on_hover_text(tip);
                    ui.add_space(4.0);
                }
            });

            ui.add_space(8.0);
            draw_side_tabs(ui, p, &mut snap.tab);
            ui.add_space(6.0);

            let tab = snap.tab;
            let sel = snap.selection;
            if tab == SidebarTab::Station {
                if snap.stations.is_empty() {
                    empty_hint(
                        ui,
                        p,
                        "暂无测站",
                        "启用 Jian 测站 /kmoni 后，实时震度点将显示于此。",
                    );
                } else {
                    // 顶层虚拟列表（勿套进外层 ScrollArea，否则 available_height 无效）
                    let row_h = 58.0_f32;
                    let n = snap.stations.len();
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show_rows(ui, row_h, n, |ui, row_range| {
                            for i in row_range {
                                let s = &snap.stations[i];
                                let selected = sel
                                    == Some(ListSelection {
                                        tab: SidebarTab::Station,
                                        index: i,
                                    });
                                let col = match s.intensity_kind {
                                    IntensityKind::CnIntensity => {
                                        if s.intensity_level >= 1 {
                                            let c = jian_core::palette::cn_intensity_rgb(
                                                s.intensity_level,
                                            );
                                            Color32::from_rgb(c.r, c.g, c.b)
                                        } else {
                                            Color32::from_rgb(0x66, 0x66, 0x66)
                                        }
                                    }
                                    IntensityKind::JmaShindo if s.instrumental >= -3.0 => {
                                        let c = jian_core::instrumental_rgb(s.instrumental);
                                        Color32::from_rgb(c.r, c.g, c.b)
                                    }
                                    _ => {
                                        intensity_chip(
                                            s.intensity_kind,
                                            s.intensity_level,
                                            &s.intensity_text,
                                            scale,
                                        )
                                        .2
                                    }
                                };
                                if list_row(
                                    ui,
                                    p,
                                    icons,
                                    &s.name,
                                    &s.id,
                                    "",
                                    s.intensity_kind,
                                    s.intensity_level,
                                    &s.intensity_text,
                                    scale,
                                    col,
                                    selected,
                                    egui::Id::new(("st", i)),
                                ) == RowClick::Primary
                                {
                                    clicked = Some(SideAction::List(SidebarTab::Station, i));
                                }
                            }
                        });
                }
            } else {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        match tab {
                            SidebarTab::Records => {
                                if snap.records.is_empty() {
                                    empty_hint(
                                        ui,
                                        p,
                                        "暂无速报",
                                        "已启用数据源时将自动填入；也可在设置中检查连接状态。",
                                    );
                                }
                                for (i, r) in snap.records.iter().enumerate() {
                                    let selected = sel
                                        == Some(ListSelection {
                                            tab: SidebarTab::Records,
                                            index: i,
                                        });
                                    let mut place = place_with_agency(&r.place, &r.agency.0);
                                    if r.is_auto {
                                        place.push_str(" [自动测定]");
                                    }
                                    let (_, _, col, _) = intensity_chip(
                                        r.intensity_kind,
                                        r.intensity_level,
                                        &r.intensity_text,
                                        scale,
                                    );
                                    if list_row(
                                        ui,
                                        p,
                                        icons,
                                        &place,
                                        &r.time_text,
                                        &r.magnitude_label(),
                                        r.intensity_kind,
                                        r.intensity_level,
                                        &r.intensity_text,
                                        scale,
                                        col,
                                        selected,
                                        egui::Id::new(("rec", i)),
                                    ) == RowClick::Primary
                                    {
                                        clicked = Some(SideAction::List(SidebarTab::Records, i));
                                    }
                                }
                            }
                            SidebarTab::Eew => {
                                let sidebar = eew_sidebar_entries(&snap.eew_list);
                                if sidebar.is_empty() {
                                    empty_hint(
                                        ui,
                                        p,
                                        "暂无预警",
                                        "连接正常后，紧急地震速报会出现在此列表。",
                                    );
                                }
                                for (real_i, e) in sidebar {
                                    let selected = sel
                                        == Some(ListSelection {
                                            tab: SidebarTab::Eew,
                                            index: real_i,
                                        });
                                    let place = place_with_agency(&e.place, &e.agency.0);
                                    let time = format_list_time(e.origin_ms);
                                    let (_, _, col, _) = intensity_chip(
                                        e.intensity_kind,
                                        e.intensity_level,
                                        &e.max_intensity_text,
                                        scale,
                                    );
                                    if list_row(
                                        ui,
                                        p,
                                        icons,
                                        &place,
                                        &format!("{time}  #{}", e.serial),
                                        &format!("M{:.1}", e.magnitude),
                                        e.intensity_kind,
                                        e.intensity_level,
                                        &e.max_intensity_text,
                                        scale,
                                        col,
                                        selected,
                                        egui::Id::new(("eew", real_i)),
                                    ) == RowClick::Primary
                                    {
                                        clicked = Some(SideAction::List(SidebarTab::Eew, real_i));
                                    }
                                }
                            }
                            SidebarTab::Station => {}
                            SidebarTab::Volcano => {
                                if snap.volcanoes.is_empty() {
                                    empty_hint(
                                        ui,
                                        p,
                                        "暂无火山情报",
                                        "启用 Pancakes JMA-Atom 后，降灰预报等会出现在此列表。",
                                    );
                                }
                                for (i, v) in snap.volcanoes.iter().enumerate() {
                                    let selected = sel
                                        == Some(ListSelection {
                                            tab: SidebarTab::Volcano,
                                            index: i,
                                        });
                                    let place = place_with_agency(&v.name, &v.agency.0);
                                    if volcano_list_row(
                                        ui,
                                        p,
                                        &place,
                                        &v.time_text,
                                        &v.title,
                                        selected,
                                        show_volcano_icons,
                                        egui::Id::new(("vol", i)),
                                    ) {
                                        clicked = Some(SideAction::List(SidebarTab::Volcano, i));
                                    }
                                }
                            }
                        }
                    });
            }
        });
    clicked
}

fn empty_hint(ui: &mut Ui, p: &ThemePalette, title: &str, detail: &str) {
    ui.add_space(20.0);
    Frame::NONE
        .fill(p.chip_bg)
        .stroke(Stroke::new(1.0_f32, p.panel_border))
        .inner_margin(Margin::symmetric(12, 14))
        .corner_radius(10.0)
        .show(ui, |ui| {
            ui.label(RichText::new(title).color(p.text_muted).strong().size(14.0));
            ui.add_space(4.0);
            ui.label(RichText::new(detail).color(p.text_dim).size(12.5));
        });
}

fn draw_side_tabs(ui: &mut Ui, p: &ThemePalette, cur: &mut SidebarTab) {
    ui.horizontal(|ui| {
        for (tab, label) in [
            (SidebarTab::Records, "速报"),
            (SidebarTab::Eew, "预警"),
            (SidebarTab::Station, "测站"),
            (SidebarTab::Volcano, "火山"),
        ] {
            let selected = *cur == tab;
            let text = if selected {
                RichText::new(label).strong().size(14.0).color(p.text)
            } else {
                RichText::new(label).size(14.0).color(p.text_dim)
            };
            let resp = ui.add(
                egui::Button::new(text)
                    .fill(if selected {
                        p.accent_soft
                    } else {
                        Color32::TRANSPARENT
                    })
                    .stroke(Stroke::NONE)
                    .corner_radius(6.0)
                    .min_size(Vec2::new(60.0, 30.0)),
            );
            if selected {
                let r = resp.rect;
                ui.painter().hline(
                    r.left() + 8.0..=r.right() - 8.0,
                    r.bottom() - 1.0,
                    Stroke::new(2.0_f32, p.accent),
                );
            }
            if resp.clicked() {
                *cur = tab;
            }
        }
    });
}

fn volcano_list_row(
    ui: &mut Ui,
    p: &ThemePalette,
    place: &str,
    sub: &str,
    title: &str,
    selected: bool,
    show_icon: bool,
    id: egui::Id,
) -> bool {
    let mut clicked = false;
    let bg = if selected {
        p.row_selected
    } else {
        Color32::TRANSPARENT
    };
    let accent = Color32::from_rgb(0xF9, 0x73, 0x16);

    let resp = Frame::NONE
        .fill(bg)
        .inner_margin(Margin::symmetric(8, 7))
        .corner_radius(8.0)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                let (stripe, painter) =
                    ui.allocate_painter(Vec2::new(4.0, 50.0), Sense::hover());
                painter.rect_filled(stripe.rect, 2.0, accent);

                ui.add_space(8.0);
                if show_icon {
                    ui.label(
                        RichText::new("\u{25b2}")
                            .color(accent)
                            .size(16.0)
                            .strong(),
                    );
                    ui.add_space(4.0);
                }
                ui.vertical(|ui| {
                    let max_w = (ui.available_width() - 4.0).max(40.0);
                    ui.set_max_width(max_w);
                    ui.add(
                        egui::Label::new(RichText::new(place).color(p.text).size(15.0))
                            .truncate(),
                    );
                    ui.add(
                        egui::Label::new(RichText::new(sub).color(p.text_dim).size(13.0))
                            .truncate(),
                    );
                    ui.add(
                        egui::Label::new(RichText::new(title).color(p.text_muted).size(12.5))
                            .wrap(),
                    );
                });
            });
        })
        .response;

    let full = ui
        .interact(resp.rect, id, Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if full.hovered() && !selected {
        ui.painter().rect_stroke(
            resp.rect,
            8.0,
            Stroke::new(1.0_f32, p.panel_border),
            egui::StrokeKind::Outside,
        );
    }
    if full.clicked() {
        clicked = true;
    }
    ui.add_space(3.0);
    clicked
}

fn list_row(
    ui: &mut Ui,
    p: &ThemePalette,
    icons: &IntensityIcons,
    place: &str,
    sub: &str,
    mag: &str,
    kind: IntensityKind,
    level: u8,
    intensity_text: &str,
    scale: &str,
    accent: Color32,
    selected: bool,
    id: egui::Id,
) -> RowClick {
    let bg = if selected {
        p.row_selected
    } else {
        Color32::TRANSPARENT
    };

    let resp = Frame::NONE
        .fill(bg)
        .inner_margin(Margin::symmetric(8, 8))
        .corner_radius(8.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (stripe, painter) =
                    ui.allocate_painter(Vec2::new(4.0, 42.0), Sense::hover());
                painter.rect_filled(stripe.rect, 2.0, accent);

                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.set_max_width(ui.available_width() - 48.0);
                    ui.label(RichText::new(place).color(p.text).size(15.0));
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(sub).color(p.text_dim).size(13.0));
                        if !mag.is_empty() {
                            ui.label(RichText::new(mag).color(p.text_muted).size(13.0));
                        }
                    });
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    paint_intensity(ui, icons, kind, level, intensity_text, scale, 34.0, false);
                });
            });
        })
        .response;

    let full = ui
        .interact(resp.rect, id, Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if full.hovered() && !selected {
        ui.painter().rect_stroke(
            resp.rect,
            8.0,
            Stroke::new(1.0_f32, p.panel_border),
            egui::StrokeKind::Outside,
        );
    }
    ui.add_space(3.0);
    if full.clicked() {
        RowClick::Primary
    } else {
        RowClick::None
    }
}
