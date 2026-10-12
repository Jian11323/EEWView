//! 信息事件（速报）源开关与震级过滤。

use super::{control_field_below, control_row, framed_edit, muted, row_divider, section, toggle};
use egui::{RichText, Ui};
use jian_config::AppConfig;
use jian_core::{AgencyFamily, RECORD_FILTER_FAMILIES};

/// 各机构速报开关（挂在 Jian Project 区内，不再单独成块）。
pub fn draw_record_source_switches(ui: &mut Ui, draft: &mut AppConfig) {
    ui.label(
        RichText::new("各机构速报独立开关；关闭后该源整条丢弃。与下方震级阈值无关。")
            .size(12.0)
            .color(muted()),
    );
    ui.add_space(10.0);
    let mut first = true;
    for fam in RECORD_FILTER_FAMILIES {
        if !first {
            row_divider(ui);
        }
        first = false;
        let key = fam.as_filter_key();
        let mut on = draft.filters.record_source_enabled(key);
        let subtitle = if *fam == AgencyFamily::Other {
            "未登记机构的信息事件；默认关闭"
        } else {
            fam.as_filter_key()
        };
        control_row(ui, fam.display_name(), subtitle, "\u{25ce}", |ui| {
            toggle(ui, &mut on);
        });
        draft.filters.set_record_source_enabled(key, on);
    }
}

pub fn draw_record_mag_filters(ui: &mut Ui, draft: &mut AppConfig) {
    section(ui, "信息事件震级过滤", "\u{25ce}", |ui| {
        ui.label(
            RichText::new("手动填写最低震级；0=不过滤。白名单可绕过震级，不能绕过已关闭的源开关。")
                .size(12.0)
                .color(muted()),
        );
        ui.add_space(10.0);
        let mut first = true;
        for fam in RECORD_FILTER_FAMILIES {
            if !first {
                row_divider(ui);
            }
            first = false;
            let key = fam.as_filter_key();
            let mut val = draft.filters.record_mag_threshold(key);
            let subtitle = if *fam == AgencyFamily::Other {
                "未登记机构；仅在上方开关打开时生效"
            } else {
                "0=不过滤；低于阈值不显示"
            };
            control_row(ui, fam.display_name(), subtitle, "\u{25ce}", |ui| {
                ui.add(
                    egui::DragValue::new(&mut val)
                        .speed(0.1)
                        .range(0.0..=9.9)
                        .fixed_decimals(1)
                        .prefix("M "),
                );
            });
            draft.filters.set_record_mag(key, val);
        }
        row_divider(ui);
        control_field_below(
            ui,
            "信息事件地点白名单",
            "关键词用 | 分隔；命中后可绕过震级，但关闭的源开关仍优先",
            "\u{25ce}",
            |ui| {
                let w = (ui.available_width() - 8.0).clamp(200.0, 420.0);
                framed_edit(ui, &mut draft.filters.place_whitelist, w);
            },
        );
    });
}
