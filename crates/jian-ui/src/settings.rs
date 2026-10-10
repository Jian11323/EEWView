//! 设置页：按 RhythmQuake `design/settings-page.html` + `SettingsControlStyle`
//! 以 egui 重写（统一玻璃面板、左侧分类、扁平行；已获授权参考）。

use egui::{
    Color32, Frame, Margin, Pos2, Rect, RichText, ScrollArea, Sense, Stroke, Ui, Vec2,
};
use jian_config::AppConfig;
use jian_core::HealthStatus;
use jian_net::{exchange_login_key, format_expire_date};
use std::sync::LazyLock;

mod filters_ui;

// —— RhythmQuake 令牌（settings-page.html :root / SettingsControlStyle） ——
const PAGE_BG: Color32 = Color32::from_rgb(0x02, 0x02, 0x08);
static CONTENT: LazyLock<Color32> =
    LazyLock::new(|| Color32::from_rgba_unmultiplied(36, 36, 38, 102));
pub(super) static FIELD: LazyLock<Color32> =
    LazyLock::new(|| Color32::from_rgba_unmultiplied(255, 255, 255, 23));
static BORDER: LazyLock<Color32> =
    LazyLock::new(|| Color32::from_rgba_unmultiplied(255, 255, 255, 41));
static DIVIDER: LazyLock<Color32> =
    LazyLock::new(|| Color32::from_rgba_unmultiplied(255, 255, 255, 26));
const TEXT: Color32 = Color32::WHITE;
static TEXT70: LazyLock<Color32> =
    LazyLock::new(|| Color32::from_rgba_unmultiplied(255, 255, 255, 178));
pub(super) static MUTED: LazyLock<Color32> =
    LazyLock::new(|| Color32::from_rgba_unmultiplied(255, 255, 255, 148));
const ACCENT: Color32 = Color32::from_rgb(0x82, 0xB1, 0xFF);
static GHOST_BG: LazyLock<Color32> =
    LazyLock::new(|| Color32::from_rgba_unmultiplied(255, 255, 255, 28));
const STATUS_OK: Color32 = Color32::from_rgb(0x72, 0xD6, 0xB1);
const STATUS_WARN: Color32 = Color32::from_rgb(0xFF, 0xC6, 0x6D);
const STATUS_BAD: Color32 = Color32::from_rgb(0xEF, 0x44, 0x44);
const DECL_RED: Color32 = Color32::from_rgb(0xFF, 0x8A, 0x80);

const CAT_DATA: Color32 = Color32::from_rgb(0x62, 0xC6, 0xFF);
const CAT_MAP: Color32 = Color32::from_rgb(0x72, 0xD6, 0xB1);
const CAT_VOICE: Color32 = Color32::from_rgb(0xD6, 0xA5, 0xFF);
const CAT_ADV: Color32 = Color32::from_rgb(0xAE, 0xB8, 0xCC);
const CAT_ABOUT: Color32 = Color32::from_rgb(0xFF, 0xC6, 0x6D);

const NAV_W: f32 = 228.0;
const HEADER_H: f32 = 78.0;
const NAV_BTN_H: f32 = 58.0;
const FOOTER_H: f32 = 52.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    Sources,
    MapDisplay,
    Audio,
    Advanced,
    About,
}

impl SettingsTab {
    const ALL: [Self; 5] = [
        Self::Sources,
        Self::MapDisplay,
        Self::Audio,
        Self::Advanced,
        Self::About,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::Sources => "API与数据源",
            Self::MapDisplay => "地图与显示",
            Self::Audio => "音频",
            Self::Advanced => "高级",
            Self::About => "关于",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Self::Sources => "接口开关、鉴权与连接状态",
            Self::MapDisplay => "底图、主题、定位与烈度",
            Self::Audio => "音量、静音与音效包",
            Self::Advanced => "走时表、端点与配置路径",
            Self::About => "版本、许可与署名",
        }
    }

    /// 导航字形（无 Material Icons 时的占位）
    fn glyph(self) -> &'static str {
        match self {
            Self::Sources => "\u{25ce}",
            Self::MapDisplay => "\u{25a3}",
            Self::Audio => "\u{266a}",
            Self::Advanced => "\u{2699}",
            Self::About => "\u{24d8}",
        }
    }

    fn accent(self) -> Color32 {
        match self {
            Self::Sources => CAT_DATA,
            Self::MapDisplay => CAT_MAP,
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
    }

    pub fn set_save_note(&mut self, note: impl Into<String>) {
        self.save_note = Some(note.into());
    }

    pub fn close_after_save(&mut self) {
        self.open = false;
        self.draft = None;
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
                .with_title("EEWView · 设置")
                // 对齐 RQ 原型约 1180×660 比例，略收以适应常见桌面
                .with_inner_size([1080.0, 640.0])
                .with_min_inner_size([860.0, 520.0])
                .with_max_inner_size([1280.0, 900.0])
                .with_resizable(true),
            |ctx, class| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    close_requested = true;
                }

                let mut visuals = ctx.style().visuals.clone();
                visuals.dark_mode = true;
                visuals.override_text_color = Some(TEXT);
                visuals.panel_fill = PAGE_BG;
                visuals.window_fill = PAGE_BG;
                visuals.extreme_bg_color = PAGE_BG;
                visuals.widgets.noninteractive.fg_stroke.color = *TEXT70;
                visuals.widgets.inactive.fg_stroke.color = *TEXT70;
                visuals.widgets.hovered.fg_stroke.color = TEXT;
                visuals.widgets.active.fg_stroke.color = TEXT;
                visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(0x82, 0xB1, 0xFF, 60);
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
                                .fill(PAGE_BG)
                                .stroke(Stroke::new(1.0_f32, *BORDER))
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
                        .frame(Frame::NONE.fill(PAGE_BG).inner_margin(Margin::ZERO))
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
            Some(SettingsAction::Saved { .. }) | Some(SettingsAction::TestSound { .. })
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
    ui.allocate_ui_with_layout(
        ui.available_size(),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            // 页眉（RQ .header）
            Frame::NONE
                .inner_margin(Margin::symmetric(22, 14))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new("设置中心")
                                    .strong()
                                    .size(22.0)
                                    .color(TEXT),
                            );
                            ui.label(
                                RichText::new("EEWView · 左侧分类与右侧内容合并为同一玻璃面板")
                                    .size(12.0)
                                    .color(*MUTED),
                            );
                        });
                    });
                });

            let avail = ui.available_size();
            let body_h = (avail.y - FOOTER_H - 8.0).max(280.0);

            // 统一面板（RQ .panel）
            Frame::NONE
                .fill(*CONTENT)
                .stroke(Stroke::new(1.0_f32, *BORDER))
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
                            ui.painter().rect_filled(line, 0.0, *DIVIDER);

                            draw_main(ui, session, health, action, do_exchange, body_h);
                        },
                    );
                });

            ui.add_space(6.0);

            // 底栏：EEWView 草稿保存（RQ 即时写入；此处保留显式保存）
            Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(8, 8, 24, 115))
                .stroke(Stroke::new(1.0_f32, *DIVIDER))
                .corner_radius(10.0)
                .inner_margin(Margin::symmetric(14, 8))
                .show(ui, |ui| {
                    if let Some(n) = session.save_note.as_deref() {
                        ui.label(RichText::new(n).size(11.5).color(*MUTED));
                        ui.add_space(2.0);
                    }
                    ui.horizontal(|ui| {
                        if glass_btn(ui, "恢复默认", false).clicked() {
                            *restore = true;
                        }
                        ui.add_space(8.0);
                        ui.checkbox(
                            &mut session.keep_open,
                            RichText::new("保存后不关闭").size(12.0).color(*TEXT70),
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
                TEXT,
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
                        Stroke::new(
                            1.0,
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
                    ui.painter()
                        .rect(rect, 10.0, fill, stroke, egui::StrokeKind::Inside);

                    if selected {
                        let top = Rect::from_min_max(
                            rect.min,
                            Pos2::new(rect.max.x, rect.min.y + rect.height() * 0.55),
                        );
                        ui.painter().rect_filled(
                            top,
                            10.0,
                            Color32::from_rgba_unmultiplied(255, 255, 255, 18),
                        );
                    }

                    let icon_c = if selected { accent } else { *MUTED };
                    let title_c = if selected { TEXT } else { *TEXT70 };
                    ui.painter().text(
                        Pos2::new(rect.left() + 14.0, rect.center().y - 8.0),
                        egui::Align2::LEFT_CENTER,
                        t.glyph(),
                        egui::FontId::proportional(16.0),
                        icon_c,
                    );
                    ui.painter().text(
                        Pos2::new(rect.left() + 38.0, rect.center().y - 9.0),
                        egui::Align2::LEFT_CENTER,
                        t.title(),
                        egui::FontId::proportional(13.5),
                        title_c,
                    );
                    ui.painter().text(
                        Pos2::new(rect.left() + 38.0, rect.center().y + 10.0),
                        egui::Align2::LEFT_CENTER,
                        t.subtitle(),
                        egui::FontId::proportional(10.5),
                        *MUTED,
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
                TEXT,
            );
            ui.painter().text(
                Pos2::new(head.left() + 62.0, head.center().y + 10.0),
                egui::Align2::LEFT_CENTER,
                tab.subtitle(),
                egui::FontId::proportional(11.5),
                *MUTED,
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
    let y = head.bottom() - 1.0;
    let left = head.left() + inset;
    let right = head.right() - inset;
    let mid_l = left + (right - left) * 0.18;
    let mid_r = left + (right - left) * 0.82;
    let c = Color32::from_rgba_unmultiplied(255, 255, 255, 30);
    ui.painter().line_segment(
        [Pos2::new(mid_l, y), Pos2::new(mid_r, y)],
        Stroke::new(1.0, c),
    );
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
    {
        let draft = session.draft.as_mut().unwrap();
        let login_key = &mut session.login_key;
        match tab {
            SettingsTab::Sources => {
                if draw_sources(ui, draft, login_key, auth_busy, health) {
                    *do_exchange = true;
                }
            }
            SettingsTab::MapDisplay => draw_map_display(ui, draft),
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
            SettingsTab::Advanced => draw_advanced(ui, draft),
            SettingsTab::About => draw_about(ui),
        }
    }
}

pub(super) fn section(ui: &mut Ui, title: &str, glyph: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(glyph).size(16.0).color(ACCENT));
        ui.label(RichText::new(title).strong().size(15.0).color(TEXT));
    });
    ui.add_space(12.0);
    add(ui);
    ui.add_space(18.0);
}

pub(super) fn row_divider(ui: &mut Ui) {
    ui.add_space(10.0);
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(Vec2::new(w, 1.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, *DIVIDER);
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
            Color32::from_rgba_unmultiplied(0x82, 0xB1, 0xFF, 36),
            Stroke::new(
                1.0,
                Color32::from_rgba_unmultiplied(0x82, 0xB1, 0xFF, 72),
            ),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            icon_r.center(),
            egui::Align2::CENTER_CENTER,
            glyph,
            egui::FontId::proportional(14.0),
            ACCENT,
        );
        ui.add_space(10.0);

        ui.vertical(|ui| {
            ui.set_max_width((ui.available_width() - 280.0).max(140.0));
            ui.label(RichText::new(title).size(14.0).strong().color(TEXT));
            if !subtitle.is_empty() {
                ui.label(RichText::new(subtitle).size(12.0).color(*MUTED));
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
        Color32::from_rgba_unmultiplied(0x82, 0xB1, 0xFF, 90)
    } else {
        *GHOST_BG
    };
    let stroke = if emphasized {
        Stroke::new(
            1.0,
            Color32::from_rgba_unmultiplied(0x82, 0xB1, 0xFF, 200),
        )
    } else {
        Stroke::new(1.0, *BORDER)
    };
    ui.add(
        egui::Button::new(RichText::new(text).size(13.0).strong().color(TEXT))
            .fill(fill)
            .stroke(stroke)
            .corner_radius(10.0)
            .min_size(Vec2::new(if emphasized { 108.0 } else { 76.0 }, 36.0)),
    )
}

pub(super) fn toggle(ui: &mut Ui, on: &mut bool) -> egui::Response {
    let size = Vec2::new(46.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
    }
    let track = if *on {
        Color32::from_rgba_unmultiplied(0x82, 0xB1, 0xFF, 97)
    } else {
        Color32::from_rgba_unmultiplied(255, 255, 255, 38)
    };
    ui.painter().rect_filled(rect, 14.0, track);
    let thumb_x = if *on {
        rect.right() - 14.0
    } else {
        rect.left() + 14.0
    };
    let thumb_c = if *on {
        ACCENT
    } else {
        Color32::from_rgba_unmultiplied(255, 255, 255, 178)
    };
    ui.painter()
        .circle_filled(Pos2::new(thumb_x, rect.center().y), 10.0, thumb_c);
    resp
}

pub(super) fn framed_edit(ui: &mut Ui, text: &mut String, width: f32) {
    Frame::NONE
        .fill(*FIELD)
        .stroke(Stroke::new(1.0_f32, *BORDER))
        .corner_radius(10.0)
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(text)
                    .desired_width(width)
                    .text_color(TEXT),
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
        control_row(ui, "启用测站 /kmoni", "无需令牌", "\u{25ce}", |ui| {
            toggle(ui, &mut draft.sources.jian.station_enabled);
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
                "已保存".into()
            };
            let exp = format_expire_date(draft.sources.jian.refresh_expire_at)
                .map(|d| format!("（约至 {d}）"))
                .unwrap_or_default();
            ui.label(
                RichText::new(format!("长期 Token：{short}{exp}"))
                    .size(11.5)
                    .color(*MUTED),
            );
        } else {
            ui.label(
                RichText::new("尚未保存长期 Token。也可设环境变量 JIAN_REFRESH_TOKEN。")
                    .size(11.5)
                    .color(*MUTED),
            );
        }
        if draft.sources.jian.access_token.is_some() {
            ui.label(
                RichText::new("短期 at_ 已由 JIAN_ACCESS_TOKEN 注入（不写入文件）。")
                    .size(11.5)
                    .color(*MUTED),
            );
        }
        ui.add_space(8.0);
        row_divider(ui);
        control_row(ui, "/all 地址", "", "\u{25ce}", |ui| {
            framed_edit(ui, &mut draft.sources.jian.ws_url, 240.0);
        });
        row_divider(ui);
        control_row(ui, "测站地址", "", "\u{25ce}", |ui| {
            framed_edit(ui, &mut draft.sources.jian.station_ws_url, 240.0);
        });
    });

    section(ui, "Wolfx", "\u{25ce}", |ui| {
        control_row(ui, "启用 Wolfx", "总开关", "\u{26a1}", |ui| {
            toggle(ui, &mut draft.sources.wolfx.enabled);
        });
        row_divider(ui);
        control_row(ui, "JMA EEW", "ws-api.wolfx.jp/jma_eew", "\u{26a1}", |ui| {
            toggle(ui, &mut draft.sources.wolfx.eew_enabled);
        });
        row_divider(ui);
        control_row(ui, "CENC 速报列表", "api.wolfx.jp/cenc_eqlist.json", "\u{25ce}", |ui| {
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

    filters_ui::draw_eew_and_record_filters(ui, draft);

    exchange
}

fn status_row(ui: &mut Ui, name: &str, status: HealthStatus) {
    let (color, label) = match status {
        HealthStatus::Normal => (STATUS_OK, "正常"),
        HealthStatus::Fluctuating => (STATUS_WARN, "波动"),
        HealthStatus::Abnormal => (STATUS_BAD, "异常"),
    };
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 4.5, color);
        ui.label(RichText::new(name).size(14.0).color(TEXT));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(label).strong().size(13.0).color(color));
        });
    });
}

fn draw_map_display(ui: &mut Ui, draft: &mut AppConfig) {
    section(ui, "地图底图", "\u{25a3}", |ui| {
        control_row(ui, "底图模式", "auto：暗色要石 / 亮色地形", "\u{25a3}", |ui| {
            egui::ComboBox::from_id_salt("set_basemap")
                .selected_text(basemap_label(&draft.map.basemap))
                .width(200.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut draft.map.basemap,
                        "auto".into(),
                        "auto（暗色要石 / 亮色地形）",
                    );
                    ui.selectable_value(&mut draft.map.basemap, "vector".into(), "vector");
                    ui.selectable_value(&mut draft.map.basemap, "tiles".into(), "tiles");
                    ui.selectable_value(&mut draft.map.basemap, "both".into(), "both");
                });
        });
        row_divider(ui);
        control_row(ui, "瓦片源", "", "\u{25a3}", |ui| {
            framed_edit(ui, &mut draft.map.tile_source, 160.0);
        });
        row_divider(ui);
        control_row(ui, "瓦片基址", "", "\u{25ce}", |ui| {
            framed_edit(ui, &mut draft.map.tile_base, 220.0);
        });
        row_divider(ui);
        control_row(ui, "geodata 目录", "", "\u{25ce}", |ui| {
            framed_edit(ui, &mut draft.map.geodata_dir, 160.0);
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
    });

    section(ui, "显示", "\u{25a3}", |ui| {
        control_row(ui, "JST 时钟", "时区显示日本标准时", "\u{25ce}", |ui| {
            toggle(ui, &mut draft.ui.show_nied_clock);
        });
        row_divider(ui);
        control_row(ui, "烈度标度", "", "\u{25ce}", |ui| {
            egui::ComboBox::from_id_salt("set_intensity_scale")
                .selected_text(scale_display(&draft.ui.intensity_scale))
                .width(160.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut draft.ui.intensity_scale, "auto".into(), "自动");
                    ui.selectable_value(&mut draft.ui.intensity_scale, "jma".into(), "气象厅震度");
                    ui.selectable_value(&mut draft.ui.intensity_scale, "cn".into(), "中国烈度");
                });
        });
        row_divider(ui);
        control_row(ui, "启动侧栏", "仅下次启动生效", "\u{25ce}", |ui| {
            egui::ComboBox::from_id_salt("set_default_tab")
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
        control_row(ui, "界面主题", "保存后立即切换主界面与 auto 底图", "\u{25d0}", |ui| {
            egui::ComboBox::from_id_salt("set_theme")
                .selected_text(theme_display(&draft.ui.theme))
                .width(120.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut draft.ui.theme, "dark".into(), "暗色");
                    ui.selectable_value(&mut draft.ui.theme, "light".into(), "亮色");
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
        }
    });
}

fn basemap_label(s: &str) -> String {
    match s.trim().to_ascii_lowercase().as_str() {
        "auto" | "" => "auto".into(),
        "tiles" => "tiles".into(),
        "both" => "both".into(),
        "vector" => "vector".into(),
        other => other.to_string(),
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

fn draw_advanced(ui: &mut Ui, draft: &mut AppConfig) {
    section(ui, "走时表", "\u{2699}", |ui| {
        control_row(ui, "主表", "近距 JMA2001", "\u{25ce}", |ui| {
            framed_edit(ui, &mut draft.travel.primary, 120.0);
        });
        row_divider(ui);
        control_row(ui, "远距回退", "ak135", "\u{25ce}", |ui| {
            framed_edit(ui, &mut draft.travel.fallback, 120.0);
        });
        row_divider(ui);
        control_row(ui, "常速直线兜底", "插值失败时启用", "\u{25ce}", |ui| {
            toggle(ui, &mut draft.travel.linear_last_resort);
        });
    });

    section(ui, "P2PQuake 端点", "\u{25ce}", |ui| {
        control_row(ui, "HTTP history", "", "\u{25ce}", |ui| {
            framed_edit(ui, &mut draft.sources.p2pquake.http_history, 240.0);
        });
        row_divider(ui);
        control_row(ui, "WebSocket（预留）", "", "\u{25ce}", |ui| {
            framed_edit(ui, &mut draft.sources.p2pquake.ws_url, 240.0);
        });
    });

    section(ui, "配置文件", "\u{25ce}", |ui| {
        if let Some(path) = AppConfig::user_config_path() {
            ui.label(
                RichText::new(format!("用户覆盖：{}", path.display()))
                    .size(12.0)
                    .color(*MUTED),
            );
        } else {
            ui.label(
                RichText::new("未能解析用户配置目录。")
                    .size(12.0)
                    .color(*MUTED),
            );
        }
        ui.label(
            RichText::new("访问令牌不会写入覆盖文件。")
                .size(12.0)
                .color(*MUTED),
        );
    });
}

fn draw_about(ui: &mut Ui) {
    section(ui, "EEWView · 地震视监器", "\u{24d8}", |ui| {
        ui.label(
            RichText::new(format!("版本 {}", env!("CARGO_PKG_VERSION")))
                .size(14.0)
                .color(TEXT),
        );
        ui.add_space(8.0);
        ui.label(
            RichText::new("本软件按「原样」提供，仅供研究、教学、应急演练与减灾等合法用途；不对可用性、正确性作保证。")
                .size(12.0)
                .color(DECL_RED),
        );
    });

    section(ui, "数据源", "\u{25ce}", |ui| {
        ui.label(RichText::new("· Jian Project API").size(13.0).color(TEXT));
        ui.label(RichText::new("· Wolfx（JMA EEW / CENC）").size(13.0).color(TEXT));
        ui.label(RichText::new("· P2PQuake").size(13.0).color(TEXT));
    });

    section(ui, "许可与署名", "\u{00a9}", |ui| {
        ui.label(RichText::new("代码：Apache License 2.0").size(12.0).color(*TEXT70));
        ui.label(RichText::new("预警音效：SREV（CC BY-SA 2.0）").size(12.0).color(*TEXT70));
        ui.label(RichText::new("倒计时：地牛 Wake Up!（非自由；发行前须授权或替换）").size(12.0).color(*TEXT70));
        ui.label(RichText::new("暗色矢量底图：要石 kanameishi（AGPL-3.0，见 geodata/kanameishi/）").size(12.0).color(*TEXT70));
        ui.label(RichText::new("地图标记 SVG：eewcn（已获开发者授权，见 assets/ATTRIBUTION.md）").size(12.0).color(*TEXT70));
        ui.add_space(8.0);
        ui.label(
            RichText::new("设置中心与监视器玻璃 UI 布局参考 RhythmQuake（flutterRhythmQuake），已获开发者授权在 EEWView 中以 egui 重写实现。")
                .size(12.0)
                .color(*MUTED),
        );
        ui.hyperlink_to(
            RichText::new("https://github.com/yuelinyayun-star/flutterRhythmQuake")
                .size(12.0)
                .color(ACCENT),
            "https://github.com/yuelinyayun-star/flutterRhythmQuake",
        );
        ui.add_space(4.0);
        ui.hyperlink_to(
            RichText::new("https://github.com/Jian11323/EEWView")
                .size(12.0)
                .color(ACCENT),
            "https://github.com/Jian11323/EEWView",
        );
    });
}
