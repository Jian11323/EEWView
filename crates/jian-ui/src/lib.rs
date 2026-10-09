//! eewcn 风格主界面分区。

mod theme;

use egui::{Color32, FontId, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2};
use jian_core::palette::color_for;
use jian_core::{AppSnapshot, ListSelection, OverlayMode, SidebarTab};
use jian_map::MapViewport;
use jian_travel::TravelEngine;
use theme::DARK_BG;

pub struct MainShell {
    pub map: MapViewport,
    /// 地图波圈用经过时间（秒）
    pub map_elapsed: f64,
    /// Home / 复位用默认视口
    pub home_lon: f64,
    pub home_lat: f64,
    pub home_zoom: f64,
}

impl Default for MainShell {
    fn default() -> Self {
        Self {
            map: MapViewport::default(),
            map_elapsed: 0.0,
            home_lon: 135.0,
            home_lat: 35.0,
            home_zoom: 5.0,
        }
    }
}

impl MainShell {
    pub fn configure_map(
        &mut self,
        tile_base: impl Into<String>,
        tile_source: impl Into<String>,
        lon: f64,
        lat: f64,
        zoom: f64,
    ) {
        self.map.configure_tiles(tile_base, tile_source);
        self.map.center_lon = lon;
        self.map.center_lat = lat;
        self.map.zoom = zoom;
        self.home_lon = lon;
        self.home_lat = lat;
        self.home_zoom = zoom;
    }

    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        snap: &mut AppSnapshot,
        travel: Option<&TravelEngine>,
    ) {
        self.map_elapsed += ctx.input(|i| i.stable_dt) as f64;
        let elapsed = self.map_elapsed % 120.0;
        let show_waves = snap.overlay_mode == OverlayMode::Wave;

        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(DARK_BG))
            .show(ctx, |ui| {
                let full = ui.available_rect_before_wrap();
                let sidebar_w = 280.0_f32.min(full.width() * 0.32);
                let map_rect = egui::Rect::from_min_max(
                    full.min,
                    egui::pos2(full.max.x - sidebar_w, full.max.y),
                );
                let side_rect = egui::Rect::from_min_max(
                    egui::pos2(full.max.x - sidebar_w, full.min.y),
                    full.max,
                );

                ui.scope_builder(egui::UiBuilder::new().max_rect(map_rect), |ui| {
                    self.map.ui(
                        ui,
                        snap.active.as_ref(),
                        travel,
                        elapsed,
                        show_waves,
                    );
                    draw_event_header(ui, snap);
                    if let Some(action) = draw_legend_bar(ui) {
                        match action {
                            LegendAction::Home => {
                                self.map.center_on(
                                    self.home_lon,
                                    self.home_lat,
                                    Some(self.home_zoom),
                                );
                            }
                            LegendAction::JumpTab(tab) => {
                                snap.tab = tab;
                            }
                        }
                    }
                    draw_countdown_hud(ui, snap);
                });

                ui.scope_builder(egui::UiBuilder::new().max_rect(side_rect), |ui| {
                    if let Some((tab, idx)) = draw_sidebar(ui, snap) {
                        if let Some((lon, lat, zoom)) = snap.select_list_item(tab, idx) {
                            self.map.center_on(lon, lat, Some(zoom));
                            if snap.overlay_mode == OverlayMode::Wave {
                                self.map_elapsed = 0.0;
                            }
                        }
                    }
                });
            });
    }
}

enum LegendAction {
    Home,
    JumpTab(SidebarTab),
}

fn rgb(c: jian_core::palette::Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

fn draw_event_header(ui: &mut Ui, snap: &AppSnapshot) {
    let Some(ev) = &snap.active else { return };
    let col = rgb(color_for(ev.intensity_kind, ev.intensity_level));
    let is_station = snap.overlay_mode == OverlayMode::MarkerOnly;

    egui::Area::new(egui::Id::new("event_header"))
        .fixed_pos(ui.max_rect().left_top() + Vec2::new(12.0, 12.0))
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(102, 102, 102, 0x80))
                .inner_margin(Margin::same(8))
                .corner_radius(4.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (resp, painter) =
                            ui.allocate_painter(Vec2::splat(42.0), Sense::hover());
                        painter.rect_filled(resp.rect, 2.0, col);
                        painter.rect_stroke(
                            resp.rect,
                            2.0,
                            Stroke::new(1.0_f32, Color32::WHITE),
                            egui::StrokeKind::Outside,
                        );
                        painter.text(
                            resp.rect.center(),
                            egui::Align2::CENTER_CENTER,
                            &ev.max_intensity_text,
                            FontId::proportional(if ev.max_intensity_text.chars().count() > 2 {
                                14.0
                            } else {
                                22.0
                            }),
                            Color32::BLACK,
                        );

                        ui.vertical(|ui| {
                            let title = if ev.serial > 0 {
                                format!("{}  #{}", ev.place, ev.serial)
                            } else if is_station {
                                format!("{}  · 测站", ev.place)
                            } else {
                                ev.place.clone()
                            };
                            ui.label(
                                RichText::new(title)
                                    .color(Color32::WHITE)
                                    .strong()
                                    .size(18.0),
                            );
                            let sub = if is_station {
                                format!("测站 {}", ev.event_id.trim_start_matches("station:"))
                            } else {
                                format!(
                                    "M{:.1}   {:.0}km   发震时刻 —",
                                    ev.magnitude, ev.depth_km
                                )
                            };
                            ui.label(RichText::new(sub).color(Color32::WHITE).size(14.0));
                        });
                    });
                });
        });
}

fn draw_legend_bar(ui: &mut Ui) -> Option<LegendAction> {
    let mut action = None;
    egui::Area::new(egui::Id::new("legend_bar"))
        .fixed_pos(ui.max_rect().left_bottom() + Vec2::new(12.0, -40.0))
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(40, 40, 40, 200))
                .inner_margin(Margin::symmetric(10, 6))
                .corner_radius(3.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Intensity")
                                .color(Color32::LIGHT_GRAY)
                                .size(12.0),
                        );
                        ui.add_space(8.0);
                        if legend_chip(ui, "Home").clicked() {
                            action = Some(LegendAction::Home);
                        }
                        ui.add_space(8.0);
                        if legend_chip(ui, "Record").clicked() {
                            action = Some(LegendAction::JumpTab(SidebarTab::Records));
                        }
                        ui.add_space(8.0);
                        if legend_chip(ui, "EEW").clicked() {
                            action = Some(LegendAction::JumpTab(SidebarTab::Eew));
                        }
                        ui.add_space(8.0);
                        ui.label(RichText::new("P-S").color(Color32::LIGHT_GRAY).size(12.0));
                    });
                });
        });
    action
}

fn legend_chip(ui: &mut Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).color(Color32::LIGHT_GRAY).size(12.0))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE)
            .frame(false),
    )
}

fn draw_countdown_hud(ui: &mut Ui, snap: &AppSnapshot) {
    let Some(sec) = snap.countdown_s else { return };
    egui::Area::new(egui::Id::new("countdown_hud"))
        .fixed_pos(ui.max_rect().right_bottom() + Vec2::new(-200.0, -110.0))
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("S Wave arriving in...")
                        .color(Color32::LIGHT_GRAY)
                        .size(14.0),
                );
                ui.label(
                    RichText::new(format!("{sec}"))
                        .color(Color32::from_rgb(220, 40, 40))
                        .strong()
                        .size(64.0),
                );
                ui.label(
                    RichText::new("EEW 时钟 · NIED 时钟")
                        .color(Color32::GRAY)
                        .size(12.0),
                );
            });
        });
}

/// 返回被点击的 (tab, index)
fn draw_sidebar(ui: &mut Ui, snap: &mut AppSnapshot) -> Option<(SidebarTab, usize)> {
    let mut clicked = None;
    ui.set_min_width(ui.available_width());
    Frame::NONE
        .fill(Color32::from_rgb(28, 28, 30))
        .inner_margin(Margin::same(8))
        .show(ui, |ui| {
            ui.label(
                RichText::new("地震情报实况")
                    .color(Color32::LIGHT_GRAY)
                    .strong()
                    .size(12.0),
            );
            ui.label(
                RichText::new(format!(
                    "Jian {} · Wolfx {} · P2P {}",
                    snap.health_jian.as_zh(),
                    snap.health_wolfx.as_zh(),
                    snap.health_p2p.as_zh()
                ))
                .color(Color32::GRAY)
                .size(11.0),
            );
            ui.separator();

            egui::ScrollArea::vertical()
                .max_height(ui.available_height() - 36.0)
                .show(ui, |ui| {
                    let tab = snap.tab;
                    let sel = snap.selection;
                    match tab {
                        SidebarTab::Records => {
                            for (i, r) in snap.records.iter().enumerate() {
                                let selected = sel
                                    == Some(ListSelection {
                                        tab: SidebarTab::Records,
                                        index: i,
                                    });
                                if list_row(
                                    ui,
                                    &r.place,
                                    &r.time_text,
                                    &format!("M{:.1}", r.magnitude),
                                    rgb(color_for(r.intensity_kind, r.intensity_level)),
                                    &r.intensity_text,
                                    selected,
                                    egui::Id::new(("rec", i)),
                                ) {
                                    clicked = Some((SidebarTab::Records, i));
                                }
                            }
                        }
                        SidebarTab::Eew => {
                            for (i, e) in snap.eew_list.iter().enumerate() {
                                let selected = sel
                                    == Some(ListSelection {
                                        tab: SidebarTab::Eew,
                                        index: i,
                                    });
                                if list_row(
                                    ui,
                                    &e.place,
                                    &format!("#{}", e.serial),
                                    &format!("M{:.1}", e.magnitude),
                                    rgb(color_for(e.intensity_kind, e.intensity_level)),
                                    &e.max_intensity_text,
                                    selected,
                                    egui::Id::new(("eew", i)),
                                ) {
                                    clicked = Some((SidebarTab::Eew, i));
                                }
                            }
                        }
                        SidebarTab::Station => {
                            for (i, s) in snap.stations.iter().enumerate() {
                                let selected = sel
                                    == Some(ListSelection {
                                        tab: SidebarTab::Station,
                                        index: i,
                                    });
                                if list_row(
                                    ui,
                                    &s.name,
                                    &s.id,
                                    "",
                                    rgb(color_for(s.intensity_kind, s.intensity_level)),
                                    &s.intensity_text,
                                    selected,
                                    egui::Id::new(("st", i)),
                                ) {
                                    clicked = Some((SidebarTab::Station, i));
                                }
                            }
                        }
                    }
                });

            ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                ui.horizontal(|ui| {
                    tab_btn(ui, &mut snap.tab, SidebarTab::Records, "Records");
                    tab_btn(ui, &mut snap.tab, SidebarTab::Eew, "EEW");
                    tab_btn(ui, &mut snap.tab, SidebarTab::Station, "Station");
                });
            });
        });
    clicked
}

fn tab_btn(ui: &mut Ui, cur: &mut SidebarTab, tab: SidebarTab, label: &str) {
    let selected = *cur == tab;
    let text = if selected {
        RichText::new(label).strong().color(Color32::BLACK)
    } else {
        RichText::new(label).color(Color32::DARK_GRAY)
    };
    let fill = if selected {
        Color32::WHITE
    } else {
        Color32::from_rgb(200, 200, 200)
    };
    if ui
        .add(egui::Button::new(text).fill(fill).min_size(Vec2::new(70.0, 22.0)))
        .clicked()
    {
        *cur = tab;
    }
}

fn list_row(
    ui: &mut Ui,
    place: &str,
    sub: &str,
    mag: &str,
    color: Color32,
    intensity: &str,
    selected: bool,
    id: egui::Id,
) -> bool {
    let bg = if selected {
        Color32::from_rgb(48, 52, 64)
    } else {
        Color32::TRANSPARENT
    };
    let mut clicked = false;
    Frame::NONE
        .fill(bg)
        .inner_margin(Margin::symmetric(4, 4))
        .corner_radius(3.0)
        .show(ui, |ui| {
            let sense = Sense::click();
            let resp = ui
                .horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_max_width(ui.available_width() - 36.0);
                        ui.label(RichText::new(place).color(Color32::WHITE).size(13.0));
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(sub).color(Color32::GRAY).size(11.0));
                            if !mag.is_empty() {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(mag)
                                                .color(Color32::LIGHT_GRAY)
                                                .size(12.0),
                                        );
                                    },
                                );
                            }
                        });
                    });
                    let (r, painter) = ui.allocate_painter(Vec2::splat(28.0), Sense::hover());
                    painter.rect_filled(r.rect, 2.0, color);
                    painter.text(
                        r.rect.center(),
                        egui::Align2::CENTER_CENTER,
                        intensity,
                        FontId::proportional(11.0),
                        Color32::BLACK,
                    );
                })
                .response
                .interact(sense)
                .on_hover_cursor(egui::CursorIcon::PointingHand);

            // 用稳定 Id 扩大整行点击区
            let full = ui.interact(ui.min_rect(), id, Sense::click());
            if resp.clicked() || full.clicked() {
                clicked = true;
            }
        });
    ui.add_space(2.0);
    clicked
}
