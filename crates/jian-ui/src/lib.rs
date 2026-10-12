//! 主界面分区：地图视口 + 浮层信息头 / 图例 / 倒计时 + 右栏列表。

mod brand;
mod intensity_icons;
mod settings;
mod theme;

pub use brand::{app_icon_data, logo_texture, APP_DISPLAY_NAME};

use chrono::{FixedOffset, Local};
use egui::{Color32, FontFamily, FontId, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2};
use intensity_icons::IntensityIcons;
use jian_core::palette::color_for;
use jian_core::{
    agency_bracket, cn_intensity_text, instrumental_band_rgb, eew_map_markers_spread, eew_sidebar_entries, eew_wave_active_spread, live_wave_eews_spread, place_with_agency,
    prefer_intensity_scale, AppSnapshot, IntensityKind, ListSelection, OverlayMode,
    SidebarTab,
};
use jian_map::{zoom_to_fit_radius_km, BasemapLoadState, BasemapMode, MapViewport, VectorStyle, ProjectionMode};
use jian_travel::{max_s_wave_spread_km, TravelEngine, Wave};
use std::path::Path;
use theme::{ThemePalette, UiTheme};

pub use settings::{MapPickKind, SettingsAction, SettingsSession, SourceHealthView};
pub use theme::UiTheme as AppUiTheme;

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
    ) {
        self.map.set_projection_mode(projection);
        self.map
            .configure(basemap, tile_base, tile_source, geodata_dir);
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
    ) {
        let lon = self.map.center_lon;
        let lat = self.map.center_lat;
        let zoom = self.map.zoom;
        self.map.set_projection_mode(projection);
        self.map
            .configure(basemap, tile_base, tile_source, geodata_dir);
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
        self.map.set_ocean_fill(p.map_bg);
        self.map
            .set_vector_style(VectorStyle::for_theme(&cfg.ui.theme));

        snap.prune_finished_eews();
        self.intensity_icons.ensure(ctx);
        self.map_elapsed += ctx.input(|i| i.stable_dt) as f64;
        let now_ms = chrono::Utc::now().timestamp_millis();
        snap.show_wave_rings = cfg.ui.show_wave_rings;
        // 多震：只要仍有未取消的传播中事件就画波圈（不因当前选中项取消而全灭）
        let show_waves = snap.overlay_mode == OverlayMode::Wave && snap.show_wave_rings;
        let scale = self.intensity_scale.clone();

        let live_refs = live_wave_eews_spread(&snap.eew_list, now_ms, &snap.wave_spread_km);
        let wave_owned: Vec<_> = live_refs.into_iter().cloned().collect();
        let marker_refs = eew_map_markers_spread(&snap.eew_list, now_ms, &snap.wave_spread_km);
        let marker_owned: Vec<_> = marker_refs.into_iter().cloned().collect();
        // 演示样本：无有效发震时刻时用 map_elapsed 驱动波圈
        let sample_elapsed = self.map_elapsed % 120.0;
        let mut map_click = None;

        // 波圈动画锁定约 60fps，避免 uncapped request_repaint 拉高 CPU
        if show_waves && !wave_owned.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }

        // SidePanel 保证侧栏命中；勿再用双 scope_builder（易被地图 Sense 吞点击）
        let screen_w = ctx.screen_rect().width();
        let sidebar_w = 280.0_f32.min(screen_w * 0.32).max(234.0);
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
                let mut wave_for_map = wave_owned.clone();
                for ev in &mut wave_for_map {
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
                map_click = self.map.ui(
                    ui,
                    snap.active.as_ref(),
                    &wave_for_map,
                    &marker_owned,
                    travel,
                    now_ms,
                    show_waves,
                    &snap.stations,
                    &snap.records,
                    volcanoes_for_map,
                    selected_volcano,
                    home,
                    snap.tsunami.as_ref(),
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
                            .max(35.0);
                        let z = zoom_to_fit_radius_km(
                            ev.latitude,
                            r,
                            map_rect.width().min(map_rect.height()),
                            0.72,
                        )
                        .clamp(7.2, 11.8);
                        self.map.apply_linear_zoom_out(z, 0.10);
                        ctx.request_repaint_after(std::time::Duration::from_millis(16));
                    }
                }
                draw_map_load_banner(ui, &p, self.map.load_state(), self.map.load_status());
                if let Some(kind) = self.settings.map_pick {
                    draw_map_pick_banner(ui, &p, kind);
                }
                draw_event_header(ui, &p, snap, scale.as_str(), &self.intensity_icons);
                draw_tsunami_banner(ui, &p, snap);
                draw_realtime_intensity_card(ui, &p, snap, &self.intensity_icons);
                if cfg.ui.show_legend {
                    draw_legend_bar(ui, &p);
                }
                draw_countdown_hud(ui, &p, snap, self.show_nied_clock);
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
                                        let live = eew_wave_active_spread(
                                            e,
                                            now_ms,
                                            &snap.wave_spread_km,
                                        );
                                        if !live {
                                            return 90.0;
                                        }
                                        let elapsed = if e.origin_ms > 0 {
                                            ((now_ms - e.origin_ms) as f64 / 1000.0).max(0.0)
                                        } else {
                                            0.0
                                        };
                                        let s_km = if let Some(eng) = travel {
                                            eng.surface_distance_for_elapsed(
                                                Wave::S,
                                                e.depth_km,
                                                elapsed,
                                            )
                                        } else {
                                            elapsed * 5.0
                                        };
                                        s_km.min(max_s_wave_spread_km(e.magnitude)).max(35.0)
                                    })
                                } else {
                                    Some(80.0)
                                }
                                .unwrap_or(80.0);
                                let end_z = zoom_to_fit_radius_km(
                                    lat,
                                    r_km,
                                    map_w.min(map_h),
                                    0.72,
                                )
                                .clamp(7.2, 11.8);
                                let dur = if tab == SidebarTab::Eew { 1.0 } else { 0.85 };
                                self.map.fly_to(lon, lat, Some(end_z), dur);
                            }
                            SidebarTab::Station => {
                                self.map.fly_to(lon, lat, Some(zoom.max(8.5)), 0.7);
                            }
                            SidebarTab::Volcano => {
                                self.map.fly_to(lon, lat, Some(zoom.max(7.5)), 0.85);
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

        if let Some((lon, lat)) = map_click {
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
    egui::Area::new(egui::Id::new("map_pick_banner"))
        .fixed_pos(ui.max_rect().center_top() + Vec2::new(-140.0, 44.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ui.ctx(), |ui| {
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
    egui::Area::new(egui::Id::new("map_load_banner"))
        .fixed_pos(ui.max_rect().center_top() + Vec2::new(-120.0, 10.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ui.ctx(), |ui| {
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

/// 左上预警条：GQ 只保留当前焦点一条（镜头切换）；其余数据源仍向下堆叠。
fn header_live_eews(snap: &AppSnapshot) -> Vec<&jian_core::EewReport> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let live = live_wave_eews_spread(&snap.eew_list, now_ms, &snap.wave_spread_km);
    let mut gq = Vec::new();
    let mut others = Vec::new();
    for e in live {
        if jian_core::agency_family(&e.agency.0) == jian_core::AgencyFamily::Gq {
            gq.push(e);
        } else {
            others.push(e);
        }
    }
    let mut out = Vec::new();
    if !gq.is_empty() {
        let focused = snap.active.as_ref().and_then(|a| {
            gq.iter()
                .copied()
                .find(|e| e.agency.0 == a.agency.0 && e.event_id == a.event_id)
        });
        out.push(focused.unwrap_or(gq[0]));
    }
    out.extend(others);
    out
}

/// 左上角预警条总高度（海啸条避让）。
fn eew_header_stack_height(snap: &AppSnapshot) -> f32 {
    let n = header_live_eews(snap).len();
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
) {
    let live = header_live_eews(snap);
    if live.is_empty() {
        return;
    }

    const LEFT_PANEL_X: f32 = 12.0;
    const LEFT_PANEL_W: f32 = 340.0;
    const ROW_GAP: f32 = 6.0;
    egui::Area::new(egui::Id::new("event_header"))
        .fixed_pos(ui.max_rect().left_top() + Vec2::new(LEFT_PANEL_X, OVERLAY_TOP_Y))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ui.ctx(), |ui| {
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
                    let mag_line = format!("M{:.1}", ev.magnitude);
                    let depth_line = format!("{:.0}km", ev.depth_km);
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

fn draw_tsunami_banner(ui: &mut Ui, p: &ThemePalette, snap: &AppSnapshot) {
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
        let h = eew_header_stack_height(snap);
        if h > 0.0 {
            OVERLAY_TOP_Y + h + 8.0
        } else {
            OVERLAY_TOP_Y
        }
    };
    const LEFT_PANEL_X: f32 = 12.0;
    const LEFT_PANEL_W: f32 = 340.0;
    egui::Area::new(egui::Id::new("tsunami_banner"))
        .fixed_pos(ui.max_rect().left_top() + Vec2::new(LEFT_PANEL_X, top_y))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ui.ctx(), |ui| {
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

/// 地图右上角实时震度卡片（对齐要石 sourceMaxCard：网名 / 計測值 / 时刻）。
fn draw_realtime_intensity_card(
    ui: &mut Ui,
    _p: &ThemePalette,
    snap: &AppSnapshot,
    _icons: &IntensityIcons,
) {
    if snap.stations.is_empty() {
        return;
    }
    let mut cards: Vec<(&str, &str, f64, i64)> = Vec::new();
    for (key, label) in [("kmoni", "Kmoni"), ("snet", "S-Net"), ("kma", "KMA")] {
        let mut max_inst = -99.0_f64;
        let mut any = false;
        for s in &snap.stations {
            if !s.id.starts_with(&format!("{key}:")) {
                continue;
            }
            any = true;
            if s.instrumental > max_inst {
                max_inst = s.instrumental;
            }
        }
        if any {
            let ts = snap.station_updated_ms.get(key).copied().unwrap_or(0);
            cards.push((key, label, max_inst, ts));
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
        cards.push(("all", "NIED", max_inst, ts));
    }

    const CARD_W: f32 = 72.0;
    // 与左上预警框（72 烈度块 + 边距）同顶、接近同高
    const CARD_H: f32 = 88.0;
    let gap = 6.0_f32;
    let now_ms = chrono::Utc::now().timestamp_millis();
    // 超过该间隔未刷新视为测站 API 断开（原版 NIED ~0.8s；WS 心跳略宽）
    const STALE_MS: i64 = 8_000;
    let total_w = CARD_W * cards.len() as f32 + gap * (cards.len().saturating_sub(1) as f32);
    let top_right = ui.max_rect().right_top() + Vec2::new(-12.0, OVERLAY_TOP_Y);
    egui::Area::new(egui::Id::new("rt_intensity_card"))
        .pivot(egui::Align2::RIGHT_TOP)
        .fixed_pos(top_right)
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ui.ctx(), |ui| {
            ui.set_min_size(Vec2::new(total_w, CARD_H));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (k, label, inst, ts) in &cards {
                    let value = if *inst >= -3.0 {
                        jian_core::instrumental_half_step_text(*inst)
                    } else {
                        "-".into()
                    };
                    let rgb = jian_core::instrumental_rgb(*inst);
                    let value_color = if *inst >= -3.0 {
                        Color32::from_rgb(rgb.r, rgb.g, rgb.b)
                    } else {
                        Color32::from_rgb(0x66, 0x66, 0x66)
                    };
                    let time_text = if *ts > 0 {
                        format_clock_hms(*ts)
                    } else {
                        "--:--:--".into()
                    };
                    // S-Net 约每分钟一帧，断线阈值放宽到 90s
                    let stale = if *k == "snet" { 90_000_i64 } else { STALE_MS };
                    let connected = *ts > 0 && (now_ms - *ts).abs() <= stale;
                    let time_color = if connected {
                        Color32::WHITE
                    } else {
                        Color32::from_rgb(0xFF, 0x44, 0x44)
                    };

                    let (r, _) =
                        ui.allocate_exact_size(Vec2::new(CARD_W, CARD_H), Sense::hover());
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
                    painter.text(
                        egui::pos2(r.center().x, head.bottom() + 22.0),
                        egui::Align2::CENTER_CENTER,
                        value,
                        FontId::proportional(22.0),
                        value_color,
                    );
                    painter.text(
                        egui::pos2(r.center().x, r.bottom() - 14.0),
                        egui::Align2::CENTER_CENTER,
                        time_text,
                        FontId::proportional(12.0),
                        time_color,
                    );
                }
            });
        });
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
    // 左：中国烈度 I–XII；中：要石計測色带；右：震度/計測 -3～7（-3 深蓝 #0003cf）
    const ROW: f32 = 18.0;
    const LEFT_W: f32 = 28.0;
    const BAR_W: f32 = 16.0;
    const RIGHT_W: f32 = 36.0;
    const GAP: f32 = 5.0;
    const WIDTH: f32 = LEFT_W + GAP + BAR_W + GAP + RIGHT_W;
    const N: usize = 13;
    // (右栏文案, 要石色带档位 0–20)
    const INST_ROWS: [(&str, u8); N] = [
        ("7", 20),
        ("6强", 19),
        ("6弱", 18),
        ("5强", 17),
        ("5弱", 16),
        ("4", 15),
        ("3", 13),
        ("2", 11),
        ("1", 9),
        ("0", 7),
        ("-1", 5),
        ("-2", 3),
        ("-3", 0),
    ];
    egui::Area::new(egui::Id::new("legend_bar"))
        .pivot(egui::Align2::LEFT_BOTTOM)
        .fixed_pos(ui.max_rect().left_bottom() + Vec2::new(4.0, -4.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ui.ctx(), |ui| {
            ui.spacing_mut().window_margin = Margin::ZERO;
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            ui.set_min_size(Vec2::new(WIDTH, ROW * N as f32 + 22.0));
            ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                ui.set_width(WIDTH);
                ui.label(
                    RichText::new("图例")
                        .color(p.text_muted)
                        .strong()
                        .size(15.0),
                );
            });
            let (resp, painter) =
                ui.allocate_painter(Vec2::new(WIDTH, ROW * N as f32), Sense::hover());
            let r = resp.rect;
            let bar_left = r.left() + LEFT_W + GAP;
            let bar_right = bar_left + BAR_W;
            for i in 0..N {
                let y0 = r.top() + ROW * i as f32;
                let (label, band) = INST_ROWS[i];
                painter.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(bar_left, y0 + 1.0),
                        egui::pos2(bar_right, y0 + ROW - 1.0),
                    ),
                    1.5,
                    rgb(instrumental_band_rgb(band)),
                );
                if i < 12 {
                    painter.text(
                        egui::pos2(bar_left - GAP, y0 + ROW * 0.5),
                        egui::Align2::RIGHT_CENTER,
                        cn_intensity_text((12 - i) as u8),
                        FontId::proportional(12.0),
                        p.text_dim,
                    );
                }
                painter.text(
                    egui::pos2(bar_right + GAP, y0 + ROW * 0.5),
                    egui::Align2::LEFT_CENTER,
                    label,
                    FontId::proportional(12.0),
                    p.text_muted,
                );
            }
        });
}

fn draw_countdown_hud(ui: &mut Ui, p: &ThemePalette, snap: &AppSnapshot, show_nied: bool) {
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

    egui::Area::new(egui::Id::new("countdown_hud"))
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .fixed_pos(ui.max_rect().right_bottom() + Vec2::new(-5.0, -2.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ui.ctx(), |ui| {
            ui.spacing_mut().window_margin = Margin::ZERO;
            ui.spacing_mut().item_spacing = Vec2::new(0.0, 1.0);
            ui.with_layout(egui::Layout::top_down(egui::Align::RIGHT), |ui| {
                if let Some(sec) = snap.countdown_s {
                    outlined_label(ui, "S波即将到达", Color32::WHITE, 14.0, false);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                        });
                    });
                    ui.add_space(4.0);
                }
                draw_tz_clock_block(ui, "BST", "北京时间", &clock);
                if let Some(jst) = jst_clock {
                    draw_tz_clock_block(ui, "JST", "日本标准时", &jst);
                }
            });
        });
}

fn draw_tz_clock_block(ui: &mut Ui, code: &str, title: &str, datetime: &str) {
    // right_to_left：整块贴右；先排两行文案再排 BST/JST，视觉仍为「代号 | 标题/时间」
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.with_layout(egui::Layout::top_down(egui::Align::RIGHT), |ui| {
                let f1 = FontId::new(13.0, FontFamily::Name("lxgw_wenkai".into()));
                let f2 = FontId::new(16.0, FontFamily::Name("lxgw_wenkai".into()));
                ui.label(
                    RichText::new(title)
                        .font(f1)
                        .color(Color32::from_rgb(0xCC, 0xCC, 0xCC)),
                );
                ui.label(RichText::new(datetime).font(f2).color(Color32::WHITE));
            });
        });
        let font = FontId::new(28.0, FontFamily::Name("lxgw_wenkai".into()));
        ui.label(RichText::new(code).font(font).color(Color32::WHITE).strong());
    });
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
                for (name, st) in [
                    ("Jian", snap.health_jian),
                    ("Wolfx", snap.health_wolfx),
                    ("P2P", snap.health_p2p),
                    ("Pancakes", snap.health_pancakes),
                ] {
                    let tip = format!("{name}\u{00b7}{}", st.as_zh());
                    let resp = ui
                        .horizontal(|ui| {
                            let (r, painter) =
                                ui.allocate_painter(Vec2::splat(9.0), Sense::hover());
                            painter.circle_filled(r.rect.center(), 3.8, health_dot(p, st));
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
                                let col = if s.instrumental >= -3.0 {
                                    let c = jian_core::instrumental_rgb(s.instrumental);
                                    Color32::from_rgb(c.r, c.g, c.b)
                                } else {
                                    intensity_chip(
                                        s.intensity_kind,
                                        s.intensity_level,
                                        &s.intensity_text,
                                        scale,
                                    )
                                    .2
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
                                ) {
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
                                        &format!("M{:.1}", r.magnitude),
                                        r.intensity_kind,
                                        r.intensity_level,
                                        &r.intensity_text,
                                        scale,
                                        col,
                                        selected,
                                        egui::Id::new(("rec", i)),
                                    ) {
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
                                    ) {
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
) -> bool {
    let mut clicked = false;
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
    if full.clicked() {
        clicked = true;
    }
    ui.add_space(3.0);
    clicked
}
