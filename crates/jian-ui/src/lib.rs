//! 主界面分区：地图视口 + 浮层信息头 / 图例 / 倒计时 + 右栏列表。

mod intensity_icons;
mod settings;
mod theme;

use chrono::{FixedOffset, Local};
use egui::{Color32, FontId, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2};
use intensity_icons::IntensityIcons;
use jian_core::palette::color_for;
use jian_core::{
    cn_intensity_text, jma_shindo_text, place_with_agency, prefer_intensity_scale, AppSnapshot,
    IntensityKind, ListSelection, OverlayMode, SidebarTab,
};
use jian_map::{BasemapLoadState, BasemapMode, MapViewport, VectorStyle};
use jian_travel::TravelEngine;
use std::path::Path;
use theme::{ThemePalette, UiTheme};

pub use settings::{SettingsAction, SettingsSession, SourceHealthView};
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
        let elapsed = if let Some(ev) = snap.active.as_ref() {
            if ev.origin_ms > 0 {
                let now = chrono::Utc::now().timestamp_millis();
                ((now - ev.origin_ms) as f64 / 1000.0).max(0.0)
            } else {
                self.map_elapsed % 120.0
            }
        } else {
            0.0
        };
        let cancelled = snap.active.as_ref().is_some_and(|e| e.is_cancel);
        let show_waves =
            snap.overlay_mode == OverlayMode::Wave && snap.show_wave_rings && !cancelled;
        let scale = self.intensity_scale.as_str();

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

                ui.scope_builder(egui::UiBuilder::new().max_rect(map_rect), |ui| {
                    let home = match (self.local_lon, self.local_lat) {
                        (Some(lon), Some(lat)) => Some((lon, lat)),
                        _ => None,
                    };
                    self.map.ui(
                        ui,
                        snap.active.as_ref(),
                        travel,
                        elapsed,
                        show_waves,
                        &snap.stations,
                        &snap.records,
                        home,
                    );
                    draw_map_load_banner(ui, &p, self.map.load_state(), self.map.load_status());
                    draw_event_header(ui, &p, snap, scale, &self.intensity_icons);
                    draw_legend_bar(ui, &p);
                    draw_countdown_hud(ui, &p, snap, self.show_nied_clock);
                });

                ui.scope_builder(egui::UiBuilder::new().max_rect(side_rect), |ui| {
                    if let Some(act) = draw_sidebar(ui, &p, snap, scale, &self.intensity_icons) {
                        match act {
                            SideAction::List(tab, idx) => {
                                if let Some((lon, lat, zoom)) = snap.select_list_item(tab, idx) {
                                    self.map.center_on(lon, lat, Some(zoom));
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

        let health = SourceHealthView {
            jian: snap.health_jian,
            wolfx: snap.health_wolfx,
            p2p: snap.health_p2p,
        };
        self.settings.ui(ctx, cfg, &health)
    }
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
    let is_station = snap.overlay_mode == OverlayMode::MarkerOnly;

    egui::Area::new(egui::Id::new("event_header"))
        .fixed_pos(ui.max_rect().left_top() + Vec2::new(14.0, 14.0))
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            glass_frame(p).show(ui, |ui| {
                ui.set_max_width(420.0);
                ui.horizontal(|ui| {
                    paint_intensity(
                        ui,
                        icons,
                        ev.intensity_kind,
                        ev.intensity_level,
                        &ev.max_intensity_text,
                        scale,
                        48.0,
                    );

                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        let place = if is_station {
                            ev.place.clone()
                        } else {
                            place_with_agency(&ev.place, &ev.agency.0)
                        };
                        let title = if ev.is_cancel {
                            format!("{place}  · 取消")
                        } else if ev.serial > 0 {
                            let tag = if ev.is_final { "最终" } else { "" };
                            if tag.is_empty() {
                                format!("{place}  #{}", ev.serial)
                            } else {
                                format!("{place}  #{}  {tag}", ev.serial, tag = tag)
                            }
                        } else if is_station {
                            format!("{place}  · 测站")
                        } else {
                            place
                        };
                        ui.label(RichText::new(title).color(p.text).strong().size(17.0));
                        let sub = if is_station {
                            format!("测站 {}", ev.event_id.trim_start_matches("station:"))
                        } else {
                            format!(
                                "M{:.1}    {:.0} km    {}",
                                ev.magnitude,
                                ev.depth_km,
                                format_origin(ev.origin_ms)
                            )
                        };
                        ui.label(RichText::new(sub).color(p.text_muted).size(13.0));
                    });
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
                    RichText::new("等待地震情报… 可在右栏查看速报 / 预警 / 测站")
                        .color(p.text_muted)
                        .size(12.0),
                );
            });
        });
}

fn draw_legend_bar(ui: &mut Ui, p: &ThemePalette) {
    const ROW: f32 = 20.0;
    const LEFT_W: f32 = 28.0;
    const BAR_W: f32 = 16.0;
    const RIGHT_W: f32 = 32.0;
    const GAP: f32 = 5.0;
    const WIDTH: f32 = LEFT_W + GAP + BAR_W + GAP + RIGHT_W;
    const JMA_ON_CN: [Option<u8>; 12] = [
        Some(9),
        Some(8),
        Some(7),
        Some(6),
        Some(5),
        Some(4),
        Some(3),
        Some(2),
        Some(1),
        None,
        None,
        None,
    ];
    egui::Area::new(egui::Id::new("legend_bar"))
        .pivot(egui::Align2::LEFT_BOTTOM)
        .fixed_pos(ui.max_rect().left_bottom() + Vec2::new(4.0, -4.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ui.ctx(), |ui| {
            ui.spacing_mut().window_margin = Margin::ZERO;
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            ui.set_min_size(Vec2::new(WIDTH, ROW * 12.0 + 22.0));
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
                ui.allocate_painter(Vec2::new(WIDTH, ROW * 12.0), Sense::hover());
            let r = resp.rect;
            let bar_left = r.left() + LEFT_W + GAP;
            let bar_right = bar_left + BAR_W;
            for i in 0..12u8 {
                let lv = 12 - i;
                let y0 = r.top() + ROW * i as f32;
                painter.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(bar_left, y0 + 1.0),
                        egui::pos2(bar_right, y0 + ROW - 1.0),
                    ),
                    1.5,
                    rgb(color_for(IntensityKind::CnIntensity, lv)),
                );
                painter.text(
                    egui::pos2(bar_left - GAP, y0 + ROW * 0.5),
                    egui::Align2::RIGHT_CENTER,
                    cn_intensity_text(lv),
                    FontId::proportional(13.0),
                    p.text_dim,
                );
                if let Some(jma) = JMA_ON_CN[i as usize] {
                    painter.text(
                        egui::pos2(bar_right + GAP, y0 + ROW * 0.5),
                        egui::Align2::LEFT_CENTER,
                        jma_shindo_text(jma),
                        FontId::proportional(13.0),
                        p.text_muted,
                    );
                }
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
                                let place = place_with_agency(&r.place, &r.agency.0);
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
                            for (i, s) in snap.stations.iter().enumerate() {
                                let selected = sel
                                    == Some(ListSelection {
                                        tab: SidebarTab::Station,
                                        index: i,
                                    });
                                let (_, _, col, _) = intensity_chip(
                                    s.intensity_kind,
                                    s.intensity_level,
                                    &s.intensity_text,
                                    scale,
                                );
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
