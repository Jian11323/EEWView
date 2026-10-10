//! 设置页：按 RhythmQuake `design/settings-page.html` + `SettingsControlStyle`
//! 以 egui 重写（统一玻璃面板、左侧分类、扁平行；已获授权参考）。

use egui::{
    Color32, Frame, Margin, Pos2, Rect, RichText, ScrollArea, Sense, Stroke, Ui, Vec2,
};
use jian_config::AppConfig;
use jian_core::HealthStatus;
use jian_net::{exchange_login_key, format_expire_date};

mod filters_ui;


#[derive(Debug, Clone, Copy)]
struct SettingsChrome {
    page_bg: Color32,
    content: Color32,
    field: Color32,
    border: Color32,
    divider: Color32,
    text: Color32,
    text70: Color32,
    muted: Color32,
    accent: Color32,
    ghost_bg: Color32,
    footer_bg: Color32,
    icon_fill: Color32,
    icon_stroke: Color32,
    status_ok: Color32,
    status_warn: Color32,
    status_bad: Color32,
}

impl SettingsChrome {
    fn for_theme(theme: &str) -> Self {
        if theme.eq_ignore_ascii_case("light") {
            Self::light()
        } else {
            Self::dark()
        }
    }

    fn dark() -> Self {
        Self {
            page_bg: Color32::from_rgb(0x02, 0x02, 0x08),
            content: Color32::from_rgba_unmultiplied(36, 36, 38, 102),
            field: Color32::from_rgba_unmultiplied(255, 255, 255, 23),
            border: Color32::from_rgba_unmultiplied(255, 255, 255, 41),
            divider: Color32::from_rgba_unmultiplied(255, 255, 255, 26),
            text: Color32::WHITE,
            text70: Color32::from_rgba_unmultiplied(255, 255, 255, 178),
            muted: Color32::from_rgba_unmultiplied(255, 255, 255, 148),
            accent: Color32::from_rgb(0x82, 0xB1, 0xFF),
            ghost_bg: Color32::from_rgba_unmultiplied(255, 255, 255, 28),
            footer_bg: Color32::from_rgba_unmultiplied(8, 8, 24, 115),
            icon_fill: Color32::from_rgba_unmultiplied(0x82, 0xB1, 0xFF, 36),
            icon_stroke: Color32::from_rgba_unmultiplied(0x82, 0xB1, 0xFF, 72),
            status_ok: Color32::from_rgb(0x72, 0xD6, 0xB1),
            status_warn: Color32::from_rgb(0xFF, 0xC6, 0x6D),
            status_bad: Color32::from_rgb(0xEF, 0x44, 0x44),
        }
    }

    fn light() -> Self {
        Self {
            page_bg: Color32::from_rgb(0xF2, 0xF4, 0xF7),
            content: Color32::from_rgba_unmultiplied(255, 255, 255, 240),
            field: Color32::from_rgb(0xEE, 0xF2, 0xF7),
            border: Color32::from_rgba_unmultiplied(0x90, 0x9A, 0xA8, 120),
            divider: Color32::from_rgba_unmultiplied(0x90, 0x9A, 0xA8, 70),
            text: Color32::from_rgb(0x1F, 0x29, 0x37),
            text70: Color32::from_rgb(0x4B, 0x55, 0x63),
            muted: Color32::from_rgb(0x6B, 0x72, 0x80),
            accent: Color32::from_rgb(0x25, 0x63, 0xEB),
            ghost_bg: Color32::from_rgb(0xE5, 0xE7, 0xEB),
            footer_bg: Color32::from_rgba_unmultiplied(255, 255, 255, 230),
            icon_fill: Color32::from_rgba_unmultiplied(0x25, 0x63, 0xEB, 28),
            icon_stroke: Color32::from_rgba_unmultiplied(0x25, 0x63, 0xEB, 70),
            status_ok: Color32::from_rgb(0x15, 0x80, 0x3D),
            status_warn: Color32::from_rgb(0xCA, 0x8A, 0x04),
            status_bad: Color32::from_rgb(0xDC, 0x26, 0x26),
        }
    }
}

thread_local! {
    static CHROME: std::cell::Cell<SettingsChrome> =
        std::cell::Cell::new(SettingsChrome::dark());
}

fn set_chrome(c: SettingsChrome) {
    CHROME.set(c);
}

fn chrome() -> SettingsChrome {
    CHROME.get()
}

pub(super) fn muted() -> Color32 {
    chrome().muted
}


const CAT_DATA: Color32 = Color32::from_rgb(0x62, 0xC6, 0xFF);
const CAT_VOICE: Color32 = Color32::from_rgb(0xD6, 0xA5, 0xFF);
const CAT_ADV: Color32 = Color32::from_rgb(0xAE, 0xB8, 0xCC);
const CAT_GENERAL: Color32 = Color32::from_rgb(0x9E, 0xD0, 0xFF);
const CAT_ABOUT: Color32 = Color32::from_rgb(0xFF, 0xC6, 0x6D);

const NAV_W: f32 = 228.0;
const HEADER_H: f32 = 78.0;
const NAV_BTN_H: f32 = 58.0;
const FOOTER_H: f32 = 52.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    Sources,
    General,
    Audio,
    Advanced,
    About,
}

impl SettingsTab {
    const ALL: [Self; 5] = [
        Self::Sources,
        Self::General,
        Self::Audio,
        Self::Advanced,
        Self::About,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::Sources => "API与数据源",
            Self::General => "通用设置",
            Self::Audio => "音频",
            Self::Advanced => "高级",
            Self::About => "关于",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Self::Sources => "接口开关、鉴权与连接状态",
            Self::General => "主题、底图、窗口与定位",
            Self::Audio => "音量、静音与音效包",
            Self::Advanced => "走时表、缓存与配置路径",
            Self::About => "版本、许可与署名",
        }
    }

    /// 导航字形（无 Material Icons 时的占位）
    fn glyph(self) -> &'static str {
        match self {
            Self::Sources => "\u{25ce}",
            Self::General => "\u{2630}",
            Self::Audio => "\u{266a}",
            Self::Advanced => "\u{2699}",
            Self::About => "\u{24d8}",
        }
    }

    fn accent(self) -> Color32 {
        match self {
            Self::Sources => CAT_DATA,
            Self::General => CAT_GENERAL,
            Self::Audio => CAT_VOICE,
            Self::Advanced => CAT_ADV,
            Self::About => CAT_ABOUT,
        }
    }
}

pub enum SettingsAction {
    Saved { cfg: AppConfig, close: bool },
    Cancelled,
    TestSound {
        mute: bool,
        master_volume: f32,
        countdown_volume: f32,
        event_pack: String,
        countdown_pack: String,
    },
    ClearTileCache,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapPickKind {
    DefaultViewport,
    LocalLocation,
}

pub struct SourceHealthView {
    pub jian: HealthStatus,
    pub wolfx: HealthStatus,
    pub p2p: HealthStatus,
}

pub struct SettingsSession {
    pub open: bool,
    draft: Option<AppConfig>,
    tab: SettingsTab,
    keep_open: bool,
    save_note: Option<String>,
    login_key: String,
    auth_busy: bool,
    /// 主地图单击选点目标
    pub map_pick: Option<MapPickKind>,
    /// 下一帧用主地图中心写入默认视口
    pub capture_view: bool,
}

impl Default for SettingsSession {
    fn default() -> Self {
        Self {
            open: false,
            draft: None,
            tab: SettingsTab::Sources,
            keep_open: false,
            save_note: None,
            login_key: String::new(),
            auth_busy: false,
            map_pick: None,
            capture_view: false,
        }
    }
}

impl SettingsSession {
    pub fn open_with(&mut self, cfg: &AppConfig) {
        self.open = true;
        self.draft = Some(cfg.clone());
        self.tab = SettingsTab::Sources;
        self.save_note = None;
        self.login_key.clear();
        self.auth_busy = false;
        self.map_pick = None;
        self.capture_view = false;
    }

    pub fn set_save_note(&mut self, note: impl Into<String>) {
        self.save_note = Some(note.into());
    }

    pub fn close_after_save(&mut self) {
        self.open = false;
        self.draft = None;
    }

    /// 用主地图当前中心写入默认视口
    pub fn apply_capture_view(&mut self, lon: f64, lat: f64, zoom: f64) {
        if let Some(draft) = self.draft.as_mut() {
            draft.map.default_lon = lon;
            draft.map.default_lat = lat;
            draft.map.default_zoom = zoom;
            self.save_note = Some("已用当前视野写入默认视口".into());
        }
        self.capture_view = false;
    }

    /// 主地图单击坐标写入对应 `map_pick`。?
    pub fn apply_map_pick_coords(
        &mut self,
        kind: MapPickKind,
        lon: f64,
        lat: f64,
        zoom: f64,
    ) {
        let Some(draft) = self.draft.as_mut() else {
            return;
        };
        match kind {
            MapPickKind::DefaultViewport => {
                draft.map.default_lon = lon;
                draft.map.default_lat = lat;
                draft.map.default_zoom = zoom;
                self.save_note = Some("已用地图选点写入默认视口".into());
            }
            MapPickKind::LocalLocation => {
                draft.local.longitude = Some(lon);
                draft.local.latitude = Some(lat);
                self.save_note = Some("已用地图选点写入本机位置".into());
            }
        }
        self.map_pick = None;
    }

    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        live: &AppConfig,
        health: &SourceHealthView,
    ) -> Option<SettingsAction> {
        if !self.open {
            return None;
        }
        if self.draft.is_none() {
            self.draft = Some(live.clone());
        }

        ctx.set_embed_viewports(false);

        let mut action = None;
        let mut restore = false;
        let mut do_exchange = false;
        let mut close_requested = false;

        let viewport_id = egui::ViewportId::from_hash_of("eewview_settings");
        ctx.show_viewport_immediate(
            viewport_id,
            egui::ViewportBuilder::default()
                .with_title("EEWView \u{00b7} 设置")
                // 对齐 RQ 原型约 1180x660 比例，略收以适应常见桌面
                .with_inner_size([1080.0, 640.0])
                .with_min_inner_size([860.0, 520.0])
                .with_max_inner_size([1280.0, 900.0])
                .with_resizable(true),
            |ctx, class| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    close_requested = true;
                }

                let theme_key = self
                    .draft
                    .as_ref()
                    .map(|d| d.ui.theme.as_str())
                    .unwrap_or("dark");
                let c = SettingsChrome::for_theme(theme_key);
                set_chrome(c);
                let light = theme_key.eq_ignore_ascii_case("light");
                let mut visuals = if light {
                    egui::Visuals::light()
                } else {
                    egui::Visuals::dark()
                };
                visuals.dark_mode = !light;
                visuals.override_text_color = Some(c.text);
                visuals.panel_fill = c.page_bg;
                visuals.window_fill = c.page_bg;
                visuals.extreme_bg_color = c.page_bg;
                visuals.widgets.noninteractive.fg_stroke.color = c.text70;
                visuals.widgets.inactive.fg_stroke.color = c.text70;
                visuals.widgets.hovered.fg_stroke.color = c.text;
                visuals.widgets.active.fg_stroke.color = c.text;
                visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(
                    c.accent.r(),
                    c.accent.g(),
                    c.accent.b(),
                    60,
                );
                ctx.set_visuals(visuals);

                if class == egui::ViewportClass::Embedded {
                    egui::Window::new("设置")
                        .id(egui::Id::new("eewview_settings_embedded"))
                        .default_size([1080.0, 620.0])
                        .min_size([860.0, 500.0])
                        .resizable(true)
                        .collapsible(false)
                        .frame(
                            Frame::NONE
                                .fill(chrome().page_bg)
                                .stroke(Stroke::new(1.0_f32, chrome().border))
                                .inner_margin(Margin::ZERO)
                                .corner_radius(14.0),
                        )
                        .show(ctx, |ui| {
                            draw_shell(
                                ui,
                                self,
                                health,
                                &mut action,
                                &mut restore,
                                &mut do_exchange,
                            );
                        });
                } else {
                    egui::CentralPanel::default()
                        .frame(Frame::NONE.fill(chrome().page_bg).inner_margin(Margin::ZERO))
                        .show(ctx, |ui| {
                            draw_shell(
                                ui,
                                self,
                                health,
                                &mut action,
                                &mut restore,
                                &mut do_exchange,
                            );
                        });
                }
            },
        );

        if do_exchange {
            self.auth_busy = true;
            let key = self.login_key.clone();
            match exchange_login_key(&key) {
                Ok(ok) => {
                    if let Some(draft) = self.draft.as_mut() {
                        draft.sources.jian.refresh_token = Some(ok.token);
                        draft.sources.jian.refresh_expire_at = ok.expire_at;
                    }
                    let expire = format_expire_date(ok.expire_at)
                        .map(|d| format!("，约至 {d}"))
                        .unwrap_or_default();
                    self.save_note = Some(format!("{}{}", ok.message, expire));
                    self.login_key.clear();
                    if let Some(draft) = self.draft.clone() {
                        action = Some(SettingsAction::Saved {
                            cfg: draft,
                            close: false,
                        });
                    }
                }
                Err(e) => {
                    self.save_note = Some(format!("鉴权失败：{e}"));
                }
            }
            self.auth_busy = false;
        }

        if restore {
            let token = live.sources.jian.access_token.clone();
            let refresh = live.sources.jian.refresh_token.clone();
            let refresh_exp = live.sources.jian.refresh_expire_at;
            let mut d = AppConfig::default();
            d.sources.jian.access_token = token;
            d.sources.jian.refresh_token = refresh;
            d.sources.jian.refresh_expire_at = refresh_exp;
            self.draft = Some(d);
            self.save_note = Some("已恢复默认（令牌保留）；点「保存全部」写入".into());
        }

        if close_requested {
            self.open = false;
            self.draft = None;
            return Some(SettingsAction::Cancelled);
        }

        if matches!(
            action,
            Some(SettingsAction::Saved { .. })
                | Some(SettingsAction::TestSound { .. })
                | Some(SettingsAction::ClearTileCache)
        ) {
            return action;
        }
        if matches!(action, Some(SettingsAction::Cancelled)) {
            self.open = false;
            self.draft = None;
            return action;
        }
        None
    }
}

fn draw_shell(
    ui: &mut Ui,
    session: &mut SettingsSession,
    health: &SourceHealthView,
    action: &mut Option<SettingsAction>,
    restore: &mut bool,
    do_exchange: &mut bool,
) {
    // RQ .shell inset
    Frame::NONE
        .inner_margin(Margin::symmetric(28, 18))
        .show(ui, |ui| {
    ui.allocate_ui_with_layout(
        ui.available_size(),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            // 页眉（RQ .header）
            Frame::NONE
                .inner_margin(Margin::symmetric(4, 6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new("设置中心")
                                    .strong()
                                    .size(22.0)
                                    .color(chrome().text),
                            );
                            ui.label(
                                RichText::new("EEWView \u{00b7} 左侧分类与右侧内容合并为同一玻璃面板")
                                    .size(12.0)
                                    .color(chrome().muted),
                            );
                        });
                    });
                });

            let avail = ui.available_size();
            let body_h = (avail.y - FOOTER_H - 8.0).max(280.0);

            // 统一面板（RQ .panel）
            Frame::NONE
                .fill(chrome().content)
                .stroke(Stroke::new(1.0_f32, chrome().border))
                .corner_radius(14.0)
                .inner_margin(Margin::ZERO)
                .show(ui, |ui| {
                    ui.set_min_size(Vec2::new(avail.x.max(100.0), body_h));
                    ui.set_max_height(body_h);

                    ui.allocate_ui_with_layout(
                        Vec2::new(ui.available_width(), body_h),
                        egui::Layout::left_to_right(egui::Align::TOP),
                        |ui| {
                            draw_nav(ui, session, body_h);
                            // 竖分割线
                            let (line, _) =
                                ui.allocate_exact_size(Vec2::new(1.0, body_h), Sense::hover());
                            ui.painter().rect_filled(line, 0.0, chrome().divider);

                            draw_main(ui, session, health, action, do_exchange, body_h);
                        },
                    );
                });

            ui.add_space(6.0);

            // 底栏：EEWView 草稿保存（RQ 即时写入；此处保留显式保存）
            Frame::NONE
                .fill(chrome().footer_bg)
                .stroke(Stroke::new(1.0_f32, chrome().divider))
                .corner_radius(10.0)
                .inner_margin(Margin::symmetric(14, 8))
                .show(ui, |ui| {
                    if let Some(n) = session.save_note.as_deref() {
                        ui.label(RichText::new(n).size(11.5).color(chrome().muted));
                        ui.add_space(2.0);
                    }
                    ui.horizontal(|ui| {
                        if glass_btn(ui, "恢复默认", false).clicked() {
                            *restore = true;
                        }
                        ui.add_space(8.0);
                        ui.checkbox(
                            &mut session.keep_open,
                            RichText::new("保存后不关闭").size(12.0).color(chrome().text70),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let draft = session.draft.as_ref().unwrap();
                            if glass_btn(ui, "保存全部", true).clicked() {
                                *action = Some(SettingsAction::Saved {
                                    cfg: draft.clone(),
                                    close: !session.keep_open,
                                });
                            }
                            ui.add_space(6.0);
                            if glass_btn(ui, "应用", false).clicked() {
                                *action = Some(SettingsAction::Saved {
                                    cfg: draft.clone(),
                                    close: false,
                                });
                            }
                            ui.add_space(6.0);
                            if glass_btn(ui, "取消", false).clicked() {
                                *action = Some(SettingsAction::Cancelled);
                            }
                        });
                    });
                });
        },
    );
        }); // shell inset
}

fn draw_nav(ui: &mut Ui, session: &mut SettingsSession, body_h: f32) {
    ui.allocate_ui_with_layout(
        Vec2::new(NAV_W, body_h),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_min_width(NAV_W);
            ui.set_max_width(NAV_W);

            let head_rect = ui
                .allocate_exact_size(Vec2::new(NAV_W, HEADER_H), Sense::hover())
                .0;
            ui.painter().text(
                Pos2::new(head_rect.left() + 18.0, head_rect.center().y),
                egui::Align2::LEFT_CENTER,
                "应用设置",
                egui::FontId::proportional(15.0),
                chrome().text,
            );
            paint_fade_rule(ui, head_rect, 12.0);

            ui.add_space(8.0);

            for t in SettingsTab::ALL {
                let selected = session.tab == t;
                let accent = t.accent();
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::new(NAV_W - 20.0, NAV_BTN_H), Sense::click());

                    let fill = if selected {
                        Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 36)
                    } else if resp.hovered() {
                        Color32::from_rgba_unmultiplied(255, 255, 255, 16)
                    } else {
                        Color32::TRANSPARENT
                    };
                    let stroke = if selected {
                        Stroke::new(1.0_f32,
                            Color32::from_rgba_unmultiplied(
                                accent.r(),
                                accent.g(),
                                accent.b(),
                                110,
                            ),
                        )
                    } else {
                        Stroke::NONE
                    };
                    // Single fill + border (RQ .nav-btn.active); no split gloss layer.
                    if selected {
                        ui.painter().rect_filled(
                            rect,
                            10.0,
                            Color32::from_rgba_unmultiplied(255, 255, 255, 26),
                        );
                    }
                    ui.painter()
                        .rect(rect, 10.0, fill, stroke, egui::StrokeKind::Inside);
                    if selected {
                        // Thin top inset highlight; keeps the selected fill as one band.
                        let y = rect.min.y + 1.0;
                        ui.painter().line_segment(
                            [
                                Pos2::new(rect.min.x + 10.0, y),
                                Pos2::new(rect.max.x - 10.0, y),
                            ],
                            Stroke::new(
                                1.0_f32,
                                Color32::from_rgba_unmultiplied(255, 255, 255, 46),
                            ),
                        );
                    }

                    let icon_c = if selected { accent } else { chrome().muted };
                    let title_c = if selected { chrome().text } else { chrome().text70 };
                    let cy = rect.center().y;
                    // Icon + two-line meta vertically centered as one group.
                    ui.painter().text(
                        Pos2::new(rect.left() + 14.0, cy),
                        egui::Align2::LEFT_CENTER,
                        t.glyph(),
                        egui::FontId::proportional(16.0),
                        icon_c,
                    );
                    ui.painter().text(
                        Pos2::new(rect.left() + 38.0, cy - 8.0),
                        egui::Align2::LEFT_CENTER,
                        t.title(),
                        egui::FontId::proportional(13.5),
                        title_c,
                    );
                    ui.painter().text(
                        Pos2::new(rect.left() + 38.0, cy + 9.0),
                        egui::Align2::LEFT_CENTER,
                        t.subtitle(),
                        egui::FontId::proportional(10.5),
                        chrome().muted,
                    );

                    if resp.clicked() {
                        session.tab = t;
                    }
                });
                ui.add_space(4.0);
            }
        },
    );
}

fn draw_main(
    ui: &mut Ui,
    session: &mut SettingsSession,
    health: &SourceHealthView,
    action: &mut Option<SettingsAction>,
    do_exchange: &mut bool,
    body_h: f32,
) {
    let w = ui.available_width();
    ui.allocate_ui_with_layout(
        Vec2::new(w, body_h),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            let tab = session.tab;
            let accent = tab.accent();

            // 内容头（RQ .content-head）
            let head = ui
                .allocate_exact_size(Vec2::new(ui.available_width(), HEADER_H), Sense::hover())
                .0;
            let icon_rect = Rect::from_center_size(
                Pos2::new(head.left() + 36.0, head.center().y),
                Vec2::splat(36.0),
            );
            ui.painter().rect(
                icon_rect,
                8.0,
                Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 34),
                Stroke::NONE,
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                icon_rect.center(),
                egui::Align2::CENTER_CENTER,
                tab.glyph(),
                egui::FontId::proportional(18.0),
                accent,
            );
            ui.painter().text(
                Pos2::new(head.left() + 62.0, head.center().y - 9.0),
                egui::Align2::LEFT_CENTER,
                tab.title(),
                egui::FontId::proportional(16.0),
                chrome().text,
            );
            ui.painter().text(
                Pos2::new(head.left() + 62.0, head.center().y + 10.0),
                egui::Align2::LEFT_CENTER,
                tab.subtitle(),
                egui::FontId::proportional(11.5),
                chrome().muted,
            );
            paint_fade_rule(ui, head, 14.0);

            ScrollArea::vertical()
                .auto_shrink([false, false])
                .id_salt("settings_body")
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.add_space(8.0);
                    Frame::NONE
                        .inner_margin(Margin::symmetric(16, 8))
                        .show(ui, |ui| {
                            draw_content(ui, session, health, action, do_exchange);
                        });
                    ui.add_space(16.0);
                });
        },
    );
}

fn paint_fade_rule(ui: &Ui, head: Rect, inset: f32) {
    // RQ .panel-nav-head / .content-head ::after ? mid band with faded ends.
    let y = head.bottom() - 1.0;
    let left = head.left() + inset;
    let right = head.right() - inset;
    let span = (right - left).max(1.0);
    let stops = [
        (0.00, 0),
        (0.18, 30),
        (0.50, 30),
        (0.82, 30),
        (1.00, 0),
    ];
    for w in stops.windows(2) {
        let (t0, a0) = w[0];
        let (t1, a1) = w[1];
        let x0 = left + span * t0 as f32;
        let x1 = left + span * t1 as f32;
        let a = ((a0 + a1) / 2).max(1) as u8;
        ui.painter().line_segment(
            [Pos2::new(x0, y), Pos2::new(x1, y)],
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, a)),
        );
    }
}

fn draw_content(
    ui: &mut Ui,
    session: &mut SettingsSession,
    health: &SourceHealthView,
    action: &mut Option<SettingsAction>,
    do_exchange: &mut bool,
) {
    let tab = session.tab;
    let auth_busy = session.auth_busy;
    let mut map_pick = session.map_pick;
    let mut capture_view = session.capture_view;
    {
        let draft = session.draft.as_mut().unwrap();
        let login_key = &mut session.login_key;
        match tab {
            SettingsTab::Sources => {
                if draw_sources(ui, draft, login_key, auth_busy, health) {
                    *do_exchange = true;
                }
            }
            SettingsTab::General => draw_general(ui, draft, &mut map_pick, &mut capture_view),
            SettingsTab::Audio => {
                if draw_audio(ui, draft) {
                    *action = Some(SettingsAction::TestSound {
                        mute: draft.audio.mute,
                        master_volume: draft.audio.master_volume,
                        countdown_volume: draft.audio.countdown_volume,
                        event_pack: draft.audio.event_pack.clone(),
                        countdown_pack: draft.audio.countdown_pack.clone(),
                    });
                }
            }
            SettingsTab::Advanced => draw_advanced(ui, draft, action),
            SettingsTab::About => draw_about(ui),
        }
    }
    session.map_pick = map_pick;
    session.capture_view = capture_view;
}

pub(super) fn section(ui: &mut Ui, title: &str, glyph: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(glyph).size(16.0).color(chrome().accent));
        ui.label(RichText::new(title).strong().size(15.0).color(chrome().text));
    });
    ui.add_space(12.0);
    add(ui);
    ui.add_space(18.0);
}

pub(super) fn row_divider(ui: &mut Ui) {
    ui.add_space(10.0);
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(Vec2::new(w, 1.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, chrome().divider);
    ui.add_space(10.0);
}

/// RQ SettingsControlRow：左侧图标盒 + 标题副标题，右侧控件
pub(super) fn control_row(
    ui: &mut Ui,
    title: &str,
    subtitle: &str,
    glyph: &str,
    right: impl FnOnce(&mut Ui),
) {
    ui.horizontal(|ui| {
        let (icon_r, _) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::hover());
        ui.painter().rect(
            icon_r,
            8.0,
            chrome().icon_fill,
            Stroke::new(1.0_f32,
                chrome().icon_stroke,
            ),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            icon_r.center(),
            egui::Align2::CENTER_CENTER,
            glyph,
            egui::FontId::proportional(14.0),
            chrome().accent,
        );
        ui.add_space(10.0);

        ui.vertical(|ui| {
            ui.set_max_width((ui.available_width() - 280.0).max(140.0));
            ui.label(RichText::new(title).size(14.0).strong().color(chrome().text));
            if !subtitle.is_empty() {
                ui.label(RichText::new(subtitle).size(12.0).color(chrome().muted));
            }
        });

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.set_min_width(220.0);
            ui.set_max_width(330.0);
            right(ui);
        });
    });
}

fn glass_btn(ui: &mut Ui, text: &str, emphasized: bool) -> egui::Response {
    let fill = if emphasized {
        Color32::from_rgba_unmultiplied(chrome().accent.r(), chrome().accent.g(), chrome().accent.b(), 90)
    } else {
        chrome().ghost_bg
    };
    let stroke = if emphasized {
        Stroke::new(1.0_f32,
            Color32::from_rgba_unmultiplied(chrome().accent.r(), chrome().accent.g(), chrome().accent.b(), 200),
        )
    } else {
        Stroke::new(1.0_f32, chrome().border)
    };
    ui.add(
        egui::Button::new(RichText::new(text).size(13.0).strong().color(chrome().text))
            .fill(fill)
            .stroke(stroke)
            .corner_radius(10.0)
            .min_size(Vec2::new(if emphasized { 108.0 } else { 76.0 }, 40.0)),
    )
}

pub(super) fn toggle(ui: &mut Ui, on: &mut bool) -> egui::Response {
    let size = Vec2::new(46.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
    }
    let track = if *on {
        Color32::from_rgba_unmultiplied(chrome().accent.r(), chrome().accent.g(), chrome().accent.b(), 97)
    } else {
        chrome().ghost_bg
    };
    ui.painter().rect_filled(rect, 14.0, track);
    let thumb_x = if *on {
        rect.right() - 14.0
    } else {
        rect.left() + 14.0
    };
    let thumb_c = if *on {
        chrome().accent
    } else {
        chrome().text70
    };
    ui.painter()
        .circle_filled(Pos2::new(thumb_x, rect.center().y), 10.0, thumb_c);
    // toggle hover ring
    if resp.hovered() {
        ui.painter().rect_stroke(
            rect,
            14.0,
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(chrome().accent.r(), chrome().accent.g(), chrome().accent.b(), 90)),
            egui::StrokeKind::Outside,
        );
    }
    resp
}

pub(super) fn framed_edit(ui: &mut Ui, text: &mut String, width: f32) {
    Frame::NONE
        .fill(chrome().field)
        .stroke(Stroke::new(1.0_f32, chrome().border))
        .corner_radius(10.0)
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(text)
                    .desired_width(width)
                    .text_color(chrome().text),
            );
        });
}

fn draw_sources(
    ui: &mut Ui,
    draft: &mut AppConfig,
    login_key: &mut String,
    auth_busy: bool,
    health: &SourceHealthView,
) -> bool {
    let mut exchange = false;

    section(ui, "服务器状态", "\u{25c9}", |ui| {
        status_row(ui, "Jian Project", health.jian);
        row_divider(ui);
        status_row(ui, "Wolfx", health.wolfx);
        row_divider(ui);
        status_row(ui, "P2PQuake", health.p2p);
    });

    section(ui, "Jian Project", "\u{25ce}", |ui| {
        control_row(ui, "启用 /all", "预警与速报（需令牌）", "\u{26a1}", |ui| {
            toggle(ui, &mut draft.sources.jian.enabled);
        });
        row_divider(ui);
        control_row(ui, "登录密钥 lk_", "换取长期 Token rt_ 并写入用户配置", "\u{25ce}", |ui| {
            framed_edit(ui, login_key, 200.0);
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let label = if auth_busy { "换票中…" } else { "换票并保存" };
            if glass_btn(ui, label, true).clicked() && !auth_busy {
                exchange = true;
            }
        });
        ui.add_space(6.0);
        if let Some(rt) = draft.sources.jian.refresh_token.as_deref() {
            let short = if rt.len() > 12 {
                format!("{}…{}", &rt[..6], &rt[rt.len() - 4..])
            } else {
                rt.to_string()
            };
            let exp = format_expire_date(draft.sources.jian.refresh_expire_at)
                .map(|d| format!("（约至 {d}）"))
                .unwrap_or_default();
            ui.label(
                RichText::new(format!("长期 Token：{short}{exp}"))
                    .size(11.5)
                    .color(chrome().muted),
            );
        } else {
            ui.label(
                RichText::new("尚未保存长期 Token。也可设环境变量 JIAN_REFRESH_TOKEN。")
                    .size(11.5)
                    .color(chrome().muted),
            );
        }
        if draft.sources.jian.access_token.is_some() {
            ui.label(
                RichText::new("短期 at_ 已由 JIAN_ACCESS_TOKEN 注入（不写入文件）。")
                    .size(11.5)
                    .color(chrome().muted),
            );
        }
    });

    section(ui, "Wolfx", "\u{25ce}", |ui| {
        control_row(ui, "启用 Wolfx", "总开关", "\u{26a1}", |ui| {
            toggle(ui, &mut draft.sources.wolfx.enabled);
        });
        row_divider(ui);
        control_row(ui, "JMA EEW", "Wolfx 预警通道", "\u{26a1}", |ui| {
            toggle(ui, &mut draft.sources.wolfx.eew_enabled);
        });
        row_divider(ui);
        control_row(ui, "CENC 速报列表", "Wolfx 台网速报", "\u{25ce}", |ui| {
            toggle(ui, &mut draft.sources.wolfx.eqlist_enabled);
        });
    });

    section(ui, "P2PQuake", "\u{25ce}", |ui| {
        control_row(ui, "启用 P2PQuake", "气象厅地震情报", "\u{26a1}", |ui| {
            toggle(ui, &mut draft.sources.p2pquake.enabled);
        });
        row_divider(ui);
        control_row(ui, "轮询间隔（秒）", "保存后立即重连", "\u{25ce}", |ui| {
            ui.add(
                egui::DragValue::new(&mut draft.sources.p2pquake.poll_secs)
                    .speed(1.0)
                    .range(10..=300),
            );
        });
    });


    section(ui, "实时测站", "\u{25ce}", |ui| {
        control_row(ui, "日本 JMA/NIED", "总开关", "\u{26a1}", |ui| {
            toggle(ui, &mut draft.sources.stations.jma_enabled);
        });
        if draft.sources.stations.jma_enabled {
            row_divider(ui);
            control_row(ui, "JMA 测站源", "原版 kmoni/lmoni 或 Jian 中转", "\u{25ce}", |ui| {
                egui::ComboBox::from_id_salt("set_jma_station_src")
                    .selected_text(jma_station_src_label(&draft.sources.stations.jma_source))
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut draft.sources.stations.jma_source,
                            "jian".into(),
                            "Jian 中转 /kmoni",
                        );
                        ui.selectable_value(
                            &mut draft.sources.stations.jma_source,
                            "kmoni".into(),
                            "原版 NIED kmoni",
                        );
                        ui.selectable_value(
                            &mut draft.sources.stations.jma_source,
                            "lmoni".into(),
                            "原版 NIED lmoni",
                        );
                    });
            });
        }
        row_divider(ui);
        control_row(ui, "S-Net", "海上强震网（Jian 中转）", "\u{25ce}", |ui| {
            toggle(ui, &mut draft.sources.stations.snet_enabled);
        });
        if draft.sources.stations.snet_enabled {
            row_divider(ui);
            control_row(ui, "S-Net 源", "目前原版需 pixels.csv，暂用中转", "\u{25ce}", |ui| {
                egui::ComboBox::from_id_salt("set_snet_station_src")
                    .selected_text(snet_station_src_label(&draft.sources.stations.snet_source))
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut draft.sources.stations.snet_source,
                            "jian".into(),
                            "Jian 中转 /s-net",
                        );
                        ui.selectable_value(
                            &mut draft.sources.stations.snet_source,
                            "original".into(),
                            "原版（开发中）",
                        );
                    });
            });
        }
        row_divider(ui);
        control_row(ui, "韩国 KMA", "PEWS 测站（Jian API）", "\u{25ce}", |ui| {
            toggle(ui, &mut draft.sources.stations.kma_enabled);
        });
    });


    filters_ui::draw_eew_and_record_filters(ui, draft);

    exchange
}

fn status_row(ui: &mut Ui, name: &str, status: HealthStatus) {
    let (color, label) = match status {
        HealthStatus::Normal => (chrome().status_ok, "正常"),
        HealthStatus::Fluctuating => (chrome().status_warn, "波动"),
        HealthStatus::Abnormal => (chrome().status_bad, "异常"),
    };
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 4.5, color);
        ui.label(RichText::new(name).size(14.0).color(chrome().text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(label).strong().size(13.0).color(color));
        });
    });
}


fn is_basemap_auto(draft: &AppConfig) -> bool {
    matches!(draft.map.basemap.trim().to_ascii_lowercase().as_str(), "auto" | "")
}

fn basemap_preset_label(draft: &AppConfig) -> String {
    match draft.map.basemap.trim().to_ascii_lowercase().as_str() {
        "vector" => "矢量地图".into(),
        "tiles" | "both" => "地形地图".into(),
        _ => "跟随主题（推荐）".into(),
    }
}

fn tab_display(s: &str) -> String {
    match s.to_ascii_lowercase().as_str() {
        "records" | "record" => "速报".into(),
        "station" => "测站".into(),
        _ => "预警".into(),
    }
}

fn theme_display(s: &str) -> String {
    if s.eq_ignore_ascii_case("light") {
        "亮色".into()
    } else {
        "暗色".into()
    }
}

fn scale_display(s: &str) -> String {
    match s.to_ascii_lowercase().as_str() {
        "jma" => "气象厅震度".into(),
        "cn" => "中国烈度".into(),
        _ => "自动".into(),
    }
}

fn draw_audio(ui: &mut Ui, draft: &mut AppConfig) -> bool {
    let mut test = false;
    section(ui, "音频设置", "\u{266a}", |ui| {
        control_row(ui, "静音", "", "\u{25ce}", |ui| {
            toggle(ui, &mut draft.audio.mute);
        });
        row_divider(ui);
        control_row(ui, "主音量", "", "\u{25ce}", |ui| {
            ui.add(egui::Slider::new(&mut draft.audio.master_volume, 0.0..=1.0).show_value(true));
        });
        row_divider(ui);
        control_row(ui, "倒计时音量", "", "\u{25ce}", |ui| {
            ui.add(
                egui::Slider::new(&mut draft.audio.countdown_volume, 0.0..=1.0).show_value(true),
            );
        });
        row_divider(ui);
        control_row(ui, "试听", "按草稿音量 / 事件包立即试听（无需先保存）", "\u{25b6}", |ui| {
            if glass_btn(ui, "试听首报音", true).clicked() {
                test = true;
            }
        });
    });

    section(ui, "音效包", "\u{266a}", |ui| {
        control_row(ui, "事件包", "默认 srev（CC BY-SA 2.0）", "\u{25ce}", |ui| {
            framed_edit(ui, &mut draft.audio.event_pack, 140.0);
        });
        row_divider(ui);
        control_row(ui, "倒计时包", "countdown（非自由，不随仓分发）", "\u{25ce}", |ui| {
            framed_edit(ui, &mut draft.audio.countdown_pack, 140.0);
        });
    });
    test
}

fn draw_general(
    ui: &mut Ui,
    draft: &mut AppConfig,
    map_pick: &mut Option<MapPickKind>,
    capture_view: &mut bool,
) {
    section(ui, "外观", "\u{25d0}", |ui| {
        control_row(ui, "界面主题", "切换后设置页立即跟随；保存后应用到主界面", "\u{25d0}", |ui| {
            egui::ComboBox::from_id_salt("set_theme")
                .selected_text(theme_display(&draft.ui.theme))
                .width(120.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut draft.ui.theme, "dark".into(), "暗色");
                    ui.selectable_value(&mut draft.ui.theme, "light".into(), "亮色");
                });
        });
        row_divider(ui);
        control_row(ui, "地图底图", "跟随主题，或固定矢量 / 地形地图", "\u{25a3}", |ui| {
            egui::ComboBox::from_id_salt("set_basemap_preset")
                .selected_text(basemap_preset_label(draft))
                .width(200.0)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(is_basemap_auto(draft), "跟随主题（推荐）")
                        .clicked()
                    {
                        draft.map.basemap = "auto".into();
                    }
                    if ui
                        .selectable_label(draft.map.basemap == "vector", "矢量地图")
                        .clicked()
                    {
                        draft.map.basemap = "vector".into();
                        draft.ui.theme = "dark".into();
                    }
                    if ui
                        .selectable_label(
                            draft.map.basemap == "tiles" || draft.map.basemap == "both",
                            "地形地图",
                        )
                        .clicked()
                    {
                        draft.map.basemap = "tiles".into();
                        draft.ui.theme = "light".into();
                    }
                });
        });
    });

    section(ui, "窗口", "\u{25a3}", |ui| {
        control_row(ui, "窗口置顶", "主窗口始终保持在其他窗口之上", "\u{25ce}", |ui| {
            toggle(ui, &mut draft.ui.always_on_top);
        });
    });

    section(ui, "地图显示", "\u{25a3}", |ui| {
        control_row(ui, "预警自动跟焦", "新预警或报数更新时地图移到震中", "\u{25ce}", |ui| {
            toggle(ui, &mut draft.ui.auto_follow_eew);
        });
        row_divider(ui);
        control_row(ui, "波圈自动缩放", "随 P/S 波传播自动调整地图缩放", "\u{25ce}", |ui| {
            toggle(ui, &mut draft.ui.auto_zoom_waves);
        });
        row_divider(ui);
        control_row(ui, "P/S 波圈", "在地图上绘制走时波阵面", "\u{25ce}", |ui| {
            toggle(ui, &mut draft.ui.show_wave_rings);
        });
        row_divider(ui);
        control_row(ui, "烈度图例", "地图左下角显示烈度色标", "\u{25ce}", |ui| {
            toggle(ui, &mut draft.ui.show_legend);
        });
    });

    section(ui, "界面偏好", "\u{2630}", |ui| {
        control_row(ui, "JST 时钟", "主界面额外显示日本标准时", "\u{25ce}", |ui| {
            toggle(ui, &mut draft.ui.show_nied_clock);
        });
        row_divider(ui);
        control_row(ui, "启动侧栏", "仅下次启动生效", "\u{25ce}", |ui| {
            egui::ComboBox::from_id_salt("gen_default_tab")
                .selected_text(tab_display(&draft.ui.sidebar_default_tab))
                .width(120.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut draft.ui.sidebar_default_tab, "eew".into(), "预警");
                    ui.selectable_value(
                        &mut draft.ui.sidebar_default_tab,
                        "records".into(),
                        "速报",
                    );
                    ui.selectable_value(
                        &mut draft.ui.sidebar_default_tab,
                        "station".into(),
                        "测站",
                    );
                });
        });
        row_divider(ui);
        control_row(ui, "烈度标度", "列表与色标优先使用的标度", "\u{25ce}", |ui| {
            egui::ComboBox::from_id_salt("gen_intensity_scale")
                .selected_text(scale_display(&draft.ui.intensity_scale))
                .width(160.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut draft.ui.intensity_scale, "auto".into(), "自动");
                    ui.selectable_value(&mut draft.ui.intensity_scale, "jma".into(), "气象厅震度");
                    ui.selectable_value(&mut draft.ui.intensity_scale, "cn".into(), "中国烈度");
                });
        });
    });

    section(ui, "默认视口", "\u{25ce}", |ui| {
        control_row(ui, "经度 / 纬度 / 缩放", "Home 无本机位置时使用", "\u{25ce}", |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut draft.map.default_zoom)
                        .speed(0.1)
                        .range(2.0..=12.0)
                        .prefix("Z "),
                );
                ui.add(
                    egui::DragValue::new(&mut draft.map.default_lat)
                        .speed(0.1)
                        .range(-90.0..=90.0)
                        .prefix("\u{03c6} "),
                );
                ui.add(
                    egui::DragValue::new(&mut draft.map.default_lon)
                        .speed(0.1)
                        .range(-180.0..=180.0)
                        .prefix("\u{03bb} "),
                );
            });
        });
        row_divider(ui);
        let picking = matches!(*map_pick, Some(MapPickKind::DefaultViewport));
        let sub = if picking {
            "请点击主窗口地图选点，Esc 取消"
        } else {
            "在主地图单击选点，或采用当前视野中心"
        };
        control_row(ui, "地图选点", sub, "\u{25ce}", |ui| {
            ui.horizontal(|ui| {
                if glass_btn(ui, if picking { "选点中…" } else { "选点" }, picking).clicked() {
                    *map_pick = Some(MapPickKind::DefaultViewport);
                }
                if glass_btn(ui, "当前视野", false).clicked() {
                    *capture_view = true;
                }
            });
        });
    });

    section(ui, "本机位置", "\u{25ce}", |ui| {
        let mut has_local = draft.local.latitude.is_some() && draft.local.longitude.is_some();
        control_row(ui, "启用本机经纬", "倒计时与 Home 标记", "\u{25ce}", |ui| {
            if toggle(ui, &mut has_local).changed() {
                if has_local {
                    draft.local.latitude.get_or_insert(35.68);
                    draft.local.longitude.get_or_insert(139.76);
                } else {
                    draft.local.latitude = None;
                    draft.local.longitude = None;
                }
            }
        });
        if has_local {
            row_divider(ui);
            let mut lat = draft.local.latitude.unwrap_or(35.68);
            let mut lon = draft.local.longitude.unwrap_or(139.76);
            control_row(ui, "纬度 / 经度", "", "\u{25ce}", |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut lon).speed(0.01).range(-180.0..=180.0));
                    ui.add(egui::DragValue::new(&mut lat).speed(0.01).range(-90.0..=90.0));
                });
            });
            draft.local.latitude = Some(lat);
            draft.local.longitude = Some(lon);
            row_divider(ui);
            let picking = matches!(*map_pick, Some(MapPickKind::LocalLocation));
            let sub = if picking {
                "请点击主窗口地图选点，Esc 取消"
            } else {
                "在主地图单击设置本机位置"
            };
            control_row(ui, "地图选点", sub, "\u{25ce}", |ui| {
                if glass_btn(ui, if picking { "选点中…" } else { "选点" }, picking).clicked() {
                    *map_pick = Some(MapPickKind::LocalLocation);
                }
            });
        }
    });
}

fn draw_advanced(ui: &mut Ui, draft: &mut AppConfig, action: &mut Option<SettingsAction>) {
    section(ui, "走时表", "\u{25ce}", |ui| {
        control_row(ui, "主表（近距）", "默认 JMA2001", "\u{25ce}", |ui| {
            egui::ComboBox::from_id_salt("adv_travel_primary")
                .selected_text(draft.travel.primary.clone())
                .width(140.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut draft.travel.primary, "jma2001".into(), "jma2001");
                    ui.selectable_value(&mut draft.travel.primary, "ak135".into(), "ak135");
                });
        });
        row_divider(ui);
        control_row(ui, "远距回退", "主表插值失败时使用", "\u{25ce}", |ui| {
            egui::ComboBox::from_id_salt("adv_travel_fallback")
                .selected_text(draft.travel.fallback.clone())
                .width(140.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut draft.travel.fallback, "ak135".into(), "ak135");
                    ui.selectable_value(&mut draft.travel.fallback, "jma2001".into(), "jma2001");
                });
        });
        row_divider(ui);
        control_row(ui, "常速直线兜底", "插值失败时启用", "\u{25ce}", |ui| {
            toggle(ui, &mut draft.travel.linear_last_resort);
        });
    });

    section(ui, "瓦片缓存", "\u{25ce}", |ui| {
        control_row(ui, "内存瓦片缓存", "清除后将重新下载可见瓦片", "\u{25ce}", |ui| {
            if glass_btn(ui, "一键清除", false).clicked() {
                *action = Some(SettingsAction::ClearTileCache);
            }
        });
    });

    section(ui, "配置文件", "\u{25ce}", |ui| {
        if let Some(path) = AppConfig::user_config_path() {
            ui.label(
                RichText::new(format!("用户覆盖：{}", path.display()))
                    .size(12.0)
                    .color(chrome().muted),
            );
        } else {
            ui.label(
                RichText::new("未能解析用户配置目录。")
                    .size(12.0)
                    .color(chrome().muted),
            );
        }
        ui.add_space(6.0);
        ui.label(
            RichText::new("WebSocket 等开发向端点请直接编辑配置文件；访问令牌不会写入覆盖文件。")
                .size(12.0)
                .color(chrome().muted),
        );
    });
}

fn jma_station_src_label(s: &str) -> String {
    match s.trim().to_ascii_lowercase().as_str() {
        "kmoni" => "原版 NIED kmoni".into(),
        "lmoni" => "原版 NIED lmoni".into(),
        _ => "Jian 中转 /kmoni".into(),
    }
}

fn snet_station_src_label(s: &str) -> String {
    match s.trim().to_ascii_lowercase().as_str() {
        "original" => "原版（开发中）".into(),
        _ => "Jian 中转 /s-net".into(),
    }
}

fn draw_about(ui: &mut Ui) {
    section(ui, "EEWView \u{00b7} 地震视监器", "\u{24d8}", |ui| {
        ui.label(
            RichText::new(format!("版本 {}", env!("CARGO_PKG_VERSION")))
                .size(14.0)
                .color(chrome().text),
        );
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "跨平台地震预警与速报视监客户端。聚合多源实时情报，在地图上展示震中、\
走时波圈与测站烈度，并提供本机倒计时与音效播报。",
            )
            .size(13.0)
            .color(chrome().text),
        );
        ui.add_space(10.0);
        ui.label(
            RichText::new("数据源")
                .size(13.0)
                .strong()
                .color(chrome().text),
        );
        ui.label(
            RichText::new(
                "\u{00b7} Jian Project API（预警 / 速报 / 测站中转）\n\
\u{00b7} Wolfx（JMA EEW、台网速报）\n\
\u{00b7} P2PQuake（气象厅地震情报）\n\
\u{00b7} NIED kmoni / lmoni 原版测站图像（可选）",
            )
            .size(12.0)
            .color(chrome().muted),
        );
        ui.add_space(10.0);
        ui.label(
            RichText::new("致谢与授权")
                .size(13.0)
                .strong()
                .color(chrome().text),
        );
        ui.label(
            RichText::new(
                "界面布局参考 RhythmQuake（已获授权）。地图标记 SVG 与部分交互参考 eewcn（已获授权）。\
音效包 srev 采用 CC BY-SA 2.0。底图与走时表等第三方数据按其各自许可使用。",
            )
            .size(12.0)
            .color(chrome().muted),
        );
        ui.add_space(10.0);
        ui.label(
            RichText::new("免责声明")
                .size(13.0)
                .strong()
                .color(chrome().text),
        );
        ui.label(
            RichText::new(
                "本软件仅供信息展示与学习研究，不构成官方预警渠道。请以各国气象/地震主管部门发布为准；\
紧急情况下请遵从当地防灾指引。开发者不保证数据的实时性、完整性与准确性。",
            )
            .size(12.0)
            .color(chrome().muted),
        );
    });
}
