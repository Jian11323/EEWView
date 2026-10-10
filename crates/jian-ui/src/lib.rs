//! 主界面分区：地图视口 + 浮层信息头 / 图例 / 倒计时 + 右栏列表。

mod intensity_icons;
mod settings;
mod theme;

use chrono::{FixedOffset, Local};
use egui::{Color32, FontId, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2};
use intensity_icons::IntensityIcons;
use jian_core::palette::color_for;
use jian_core::{
    agency_bracket, cn_intensity_text, instrumental_band_rgb, latest_eew_per_source,
    live_wave_eews, place_with_agency, prefer_intensity_scale, AppSnapshot, IntensityKind,
    ListSelection, OverlayMode, SidebarTab,
};
use jian_map::{fit_events, BasemapLoadState, BasemapMode, MapViewport, VectorStyle};
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
    ) {
        self.map
            .configure(basemap, tile_base, tile_source, geodata_dir);
        self.map.center_lon = lon;
        self.map.center_lat = lat;
        self.map.zoom = zoom;
        self.home_lon = lon;
        self.home_lat = lat;
        self.home_zoom = zoom;
    }

    /// 仅换底图/瓦片，不改当前视口中心。
    pub fn reload_basemap(
        &mut self,
        basemap: BasemapMode,
        tile_base: impl Into<String>,
        tile_source: impl Into<String>,
        geodata_dir: Option<&Path>,
    ) {
        let lon = self.map.center_lon;
        let lat = self.map.center_lat;
        let zoom = self.map.zoom;
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

        self.intensity_icons.ensure(ctx);
        self.map_elapsed += ctx.input(|i| i.stable_dt) as f64;
        let now_ms = chrono::Utc::now().timestamp_millis();
        snap.show_wave_rings = cfg.ui.show_wave_rings;
        // 多震：只要仍有未取消的传播中事件就画波圈（不因当前选中项取消而全灭）
        let show_waves = snap.overlay_mode == OverlayMode::Wave && snap.show_wave_rings;
        let scale = self.intensity_scale.clone();

        let live_refs = live_wave_eews(&snap.eew_list, now_ms, 30 * 60 * 1000);
        let wave_owned: Vec<_> = live_refs.into_iter().cloned().collect();
        let marker_refs = latest_eew_per_source(&snap.eew_list);
        let marker_owned: Vec<_> = marker_refs.into_iter().cloned().collect();
        // 演示样本：无有效发震时刻时用 map_elapsed 驱动波圈
        let sample_elapsed = self.map_elapsed % 120.0;
        let mut map_click = None;

        // 有传播中波圈时每帧重绘，保证 P/S 圈持续扩张
        if show_waves && !wave_owned.is_empty() {
            ctx.request_repaint();
        }

        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(p.map_bg))
            .show(ctx, |ui| {
                let full = ui.available_rect_before_wrap();
                // 对齐 RhythmQuake 桌面壳：右信息区约 234–280
                let sidebar_w = 280.0_f32.min(full.width() * 0.32).max(234.0);
                let map_rect = egui::Rect::from_min_max(
                    full.min,
                    egui::pos2(full.max.x - sidebar_w, full.max.y),
                );
                let side_rect = egui::Rect::from_min_max(
                    egui::pos2(full.max.x - sidebar_w, full.min.y),
                    full.max,
                );

                // 自动缩放：按多震 S 波适配；飞跃中/落地冷却期内让路，避免弹跳
                if show_waves
                    && cfg.ui.auto_zoom_waves
                    && !self.map.blocks_auto_zoom()
                {
                    let mut pts = Vec::new();
                    for ev in &wave_owned {
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
                        let max_km = max_s_wave_spread_km(ev.magnitude);
                        let r = s_km.min(max_km).max(60.0);
                        pts.push((ev.longitude, ev.latitude, r));
                    }
                    if pts.is_empty() {
                        if let Some(ev) = snap.active.as_ref().filter(|e| {
                            !e.event_id.starts_with("station:")
                                && !e.event_id.starts_with("record:")
                        }) {
                            pts.push((ev.longitude, ev.latitude, 120.0));
                        }
                    }
                    if let Some((_lon, _lat, z)) =
                        fit_events(&pts, map_rect.width(), map_rect.height())
                    {
                        self.map.apply_auto_zoom(z, 0.08);
                        ctx.request_repaint();
                    }
                }

                ui.scope_builder(egui::UiBuilder::new().max_rect(map_rect), |ui| {
                    let home = match (self.local_lon, self.local_lat) {
                        (Some(lon), Some(lat)) => Some((lon, lat)),
                        _ => None,
                    };
                    // 样本事件：把 origin_ms=0 的 elapsed 写回临时列表供波圈用
                    let mut wave_for_map = wave_owned.clone();
                    for ev in &mut wave_for_map {
                        if ev.origin_ms <= 0 {
                            // 用负 origin 编码不可行；map 侧对 origin<=0 会得到 0。
                            // 改为临时写入 now - sample_elapsed*1000
                            ev.origin_ms = now_ms - (sample_elapsed * 1000.0) as i64;
                        }
                    }
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
                        home,
                    );
                    draw_map_load_banner(ui, &p, self.map.load_state(), self.map.load_status());
                    if let Some(kind) = self.settings.map_pick {
                        draw_map_pick_banner(ui, &p, kind);
                    }
                    draw_event_header(ui, &p, snap, scale.as_str(), &self.intensity_icons);
                    draw_tsunami_banner(ui, &p, snap);
                    if cfg.ui.show_legend {
                        draw_legend_bar(ui, &p);
                    }
                    draw_countdown_hud(ui, &p, snap, self.show_nied_clock);
                });

                ui.scope_builder(egui::UiBuilder::new().max_rect(side_rect), |ui| {
                    if let Some(act) = draw_sidebar(ui, &p, snap, scale.as_str(), &self.intensity_icons)
                    {
                        match act {
                            SideAction::List(tab, idx) => {
                                if let Some((lon, lat, zoom)) = snap.select_list_item(tab, idx) {
                                    // 预警 / 速报：抛物线飞跃；测站：瞬时居中
                                    match tab {
                                        SidebarTab::Eew | SidebarTab::Records => {
                                            let end_z = fit_events(
                                                &[(lon, lat, 120.0)],
                                                map_rect.width(),
                                                map_rect.height(),
                                            )
                                            .map(|(_, _, z)| z)
                                            .unwrap_or(zoom);
                                            let dur = if tab == SidebarTab::Eew {
                                                0.85
                                            } else {
                                                0.75
                                            };
                                            self.map.fly_to(lon, lat, Some(end_z), dur);
                                        }
                                        SidebarTab::Station => {
                                            self.map.center_on(lon, lat, Some(zoom));
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
                });

            });

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

fn format_origin(origin_ms: i64) -> String {
    if origin_ms <= 0 {
        return "发震时刻 —".into();
    }
    let Some(dt) = chrono::DateTime::from_timestamp_millis(origin_ms) else {
        return "发震时刻 —".into();
    };
    let cst = FixedOffset::east_opt(8 * 3600).unwrap();
    format!(
        "发震 {}",
        dt.with_timezone(&cst).format("%m-%d %H:%M:%S")
    )
}

fn format_list_time(origin_ms: i64) -> String {
    if origin_ms <= 0 {
        return "—".into();
    }
    let Some(dt) = chrono::DateTime::from_timestamp_millis(origin_ms) else {
        return "—".into();
    };
    let cst = FixedOffset::east_opt(8 * 3600).unwrap();
    dt.with_timezone(&cst).format("%m-%d %H:%M").to_string()
}

fn rgb(c: jian_core::palette::Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

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
) {
    let (k, lv, col, label) = intensity_chip(kind, level, text, scale);
    let (resp, painter) = ui.allocate_painter(Vec2::splat(size), Sense::hover());
    if let Some(tex) = icons.get(k, lv) {
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

fn draw_event_header(
    ui: &mut Ui,
    p: &ThemePalette,
    snap: &AppSnapshot,
    scale: &str,
    icons: &IntensityIcons,
) {
    let Some(ev) = &snap.active else {
        draw_idle_header(ui, p);
        return;
    };
    // 左上角仅展示预警；速报 / 测站选中不占信息头
    let is_station = snap.overlay_mode == OverlayMode::MarkerOnly
        || ev.event_id.starts_with("station:");
    let is_record = ev.event_id.starts_with("record:");
    if is_record || is_station {
        draw_idle_header(ui, p);
        return;
    }
    let (_, _, int_col, _) = intensity_chip(
        ev.intensity_kind,
        ev.intensity_level,
        &ev.max_intensity_text,
        scale,
    );

    egui::Area::new(egui::Id::new("event_header"))
        .fixed_pos(ui.max_rect().left_top() + Vec2::new(12.0, 12.0))
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            // 对齐 eewcn 旧版预警头：烈度色左边条 + 深底白边 + 三行信息
            Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(17, 17, 34, 210))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0xCC, 0xCC, 0xDD)))
                .inner_margin(Margin::ZERO)
                .corner_radius(4.0)
                .show(ui, |ui| {
                    ui.set_max_width(440.0);
                    ui.horizontal(|ui| {
                        // 左侧烈度色条
                        let (bar_resp, bar_painter) =
                            ui.allocate_painter(Vec2::new(5.0, 72.0), Sense::hover());
                        bar_painter.rect_filled(bar_resp.rect, 0.0, int_col);

                        ui.add_space(8.0);
                        paint_intensity(
                            ui,
                            icons,
                            ev.intensity_kind,
                            ev.intensity_level,
                            &ev.max_intensity_text,
                            scale,
                            64.0,
                        );
                        ui.add_space(10.0);
                        ui.vertical(|ui| {
                            ui.add_space(2.0);
                            let place = ev.place.clone();
                            ui.label(
                                RichText::new(if ev.is_cancel {
                                    format!("{place}  · 取消")
                                } else {
                                    place
                                })
                                .color(Color32::WHITE)
                                .strong()
                                .size(17.0),
                            );

                            let agency = agency_bracket(&ev.agency.0);
                            let mid = if ev.serial > 0 {
                                if ev.is_final {
                                    format!("[{agency}]  #{}  最终报", ev.serial)
                                } else {
                                    format!("[{agency}]  #{}  第{}报", ev.serial, ev.serial)
                                }
                            } else {
                                format!("[{agency}]")
                            };
                            ui.label(
                                RichText::new(mid)
                                    .color(Color32::from_rgb(0xDD, 0xEE, 0xFF))
                                    .size(13.0),
                            );

                            let sub = format!(
                                "M{:.1}    {:.0} km    {}",
                                ev.magnitude,
                                ev.depth_km,
                                format_origin(ev.origin_ms)
                            );
                            ui.label(
                                RichText::new(sub)
                                    .color(Color32::from_rgb(0xBB, 0xBB, 0xCC))
                                    .size(13.0),
                            );
                        });
                        ui.add_space(10.0);
                    });
                });
        });
}

fn draw_idle_header(ui: &mut Ui, p: &ThemePalette) {
    egui::Area::new(egui::Id::new("event_header_idle"))
        .fixed_pos(ui.max_rect().left_top() + Vec2::new(14.0, 14.0))
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            glass_frame(p).show(ui, |ui| {
                ui.set_max_width(360.0);
                ui.label(
                    RichText::new("EEWView · 地震视监器")
                        .color(p.text)
                        .strong()
                        .size(16.0),
                );
                ui.label(
                    RichText::new("等待预警… 速报与测站请在右栏查看")
                        .color(p.text_muted)
                        .size(12.0),
                );
            });
        });
}

fn draw_tsunami_banner(ui: &mut Ui, p: &ThemePalette, snap: &AppSnapshot) {
    let Some(t) = snap.tsunami.as_ref() else {
        return;
    };
    if t.cancelled {
        return;
    }
    let accent = match t.grade.as_str() {
        "MajorWarning" | "Major" => Color32::from_rgb(0xB9, 0x1C, 0x1C),
        "Warning" => Color32::from_rgb(0xDC, 0x26, 0x26),
        "Watch" => Color32::from_rgb(0xD9, 0x77, 0x06),
        _ => Color32::from_rgb(0x25, 0x63, 0xEB),
    };
    egui::Area::new(egui::Id::new("tsunami_banner"))
        .fixed_pos(ui.max_rect().left_top() + Vec2::new(14.0, 96.0))
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 220))
                .corner_radius(8.0)
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_max_width(440.0);
                    ui.label(
                        RichText::new(&t.title)
                            .color(Color32::WHITE)
                            .strong()
                            .size(15.0),
                    );
                    ui.label(
                        RichText::new(format!("影响区域：{}", t.areas))
                            .color(Color32::from_rgb(0xF8, 0xFA, 0xFC))
                            .size(12.0),
                    );
                });
            let _ = p;
        });
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
    let jst_line = if show_nied {
        let jst = FixedOffset::east_opt(9 * 3600).unwrap();
        Some(format!(
            "JST {}",
            local.with_timezone(&jst).format("%H:%M:%S")
        ))
    } else {
        None
    };

    egui::Area::new(egui::Id::new("countdown_hud"))
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .fixed_pos(ui.max_rect().right_bottom() + Vec2::new(-6.0, -4.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ui.ctx(), |ui| {
            ui.spacing_mut().window_margin = Margin::ZERO;
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            ui.with_layout(egui::Layout::top_down(egui::Align::RIGHT), |ui| {
                if let Some(sec) = snap.countdown_s {
                    glass_frame(p).show(ui, |ui| {
                        ui.with_layout(egui::Layout::top_down(egui::Align::RIGHT), |ui| {
                            ui.label(RichText::new("S 波到达").color(p.text_muted).size(13.0));
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("{sec}"))
                                        .color(p.danger)
                                        .strong()
                                        .size(56.0),
                                );
                                ui.label(RichText::new("秒").color(p.text_dim).size(18.0));
                            });
                        });
                    });
                    ui.add_space(8.0);
                }
                ui.label(
                    RichText::new(clock)
                        .color(p.text)
                        .strong()
                        .size(22.0)
                        .monospace(),
                );
                if let Some(jst) = jst_line {
                    ui.label(RichText::new(jst).color(p.text_dim).size(12.0));
                }
            });
        });
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
                        .size(14.0),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("设置").size(12.0).color(p.text_muted))
                                .fill(p.tab_idle)
                                .corner_radius(6.0)
                                .min_size(Vec2::new(48.0, 24.0)),
                        )
                        .clicked()
                    {
                        clicked = Some(SideAction::Settings);
                    }
                });
            });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                for (name, st) in [
                    ("Jian", snap.health_jian),
                    ("Wolfx", snap.health_wolfx),
                    ("P2P", snap.health_p2p),
                ] {
                    let (r, painter) = ui.allocate_painter(Vec2::splat(8.0), Sense::hover());
                    painter.circle_filled(r.rect.center(), 3.5, health_dot(p, st));
                    ui.label(
                        RichText::new(format!("{} {}", name, st.as_zh()))
                            .size(11.0)
                            .color(p.text_dim),
                    );
                    ui.add_space(6.0);
                }
            });

            ui.add_space(8.0);
            draw_side_tabs(ui, p, &mut snap.tab);
            ui.add_space(6.0);

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let tab = snap.tab;
                    let sel = snap.selection;
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
                            if snap.eew_list.is_empty() {
                                empty_hint(
                                    ui,
                                    p,
                                    "暂无预警",
                                    "连接正常后，紧急地震速报会出现在此列表。",
                                );
                            }
                            for (i, e) in snap.eew_list.iter().enumerate() {
                                let selected = sel
                                    == Some(ListSelection {
                                        tab: SidebarTab::Eew,
                                        index: i,
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
                                    egui::Id::new(("eew", i)),
                                ) {
                                    clicked = Some(SideAction::List(SidebarTab::Eew, i));
                                }
                            }
                        }
                        SidebarTab::Station => {
                            if snap.stations.is_empty() {
                                empty_hint(
                                    ui,
                                    p,
                                    "暂无测站",
                                    "启用 Jian 测站 /kmoni 后，实时震度点将显示于此。",
                                );
                            }
                            // 侧栏列出全部测站（含計測 -3～0）
                            for (i, s) in snap.stations.iter().enumerate() {
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
                        }
                    }
                });
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
            ui.label(RichText::new(title).color(p.text_muted).strong().size(13.0));
            ui.add_space(4.0);
            ui.label(RichText::new(detail).color(p.text_dim).size(11.0));
        });
}

fn draw_side_tabs(ui: &mut Ui, p: &ThemePalette, cur: &mut SidebarTab) {
    ui.horizontal(|ui| {
        for (tab, label) in [
            (SidebarTab::Records, "速报"),
            (SidebarTab::Eew, "预警"),
            (SidebarTab::Station, "测站"),
        ] {
            let selected = *cur == tab;
            let text = if selected {
                RichText::new(label).strong().size(12.0).color(p.text)
            } else {
                RichText::new(label).size(12.0).color(p.text_dim)
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
                    .min_size(Vec2::new(72.0, 28.0)),
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
        .inner_margin(Margin::symmetric(8, 7))
        .corner_radius(8.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (stripe, painter) =
                    ui.allocate_painter(Vec2::new(4.0, 36.0), Sense::hover());
                painter.rect_filled(stripe.rect, 2.0, accent);

                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.set_max_width(ui.available_width() - 40.0);
                    ui.label(RichText::new(place).color(p.text).size(13.0));
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(sub).color(p.text_dim).size(11.0));
                        if !mag.is_empty() {
                            ui.label(RichText::new(mag).color(p.text_muted).size(11.0));
                        }
                    });
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    paint_intensity(ui, icons, kind, level, intensity_text, scale, 30.0);
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
