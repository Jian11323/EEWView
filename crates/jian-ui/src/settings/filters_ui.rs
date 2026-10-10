//! RhythmQuake 风格：预警机构开关 + 信息事件震级过滤。

use super::{control_row, framed_edit, row_divider, section, toggle, MUTED};
use egui::{RichText, Ui};
use jian_config::AppConfig;
use jian_core::{AgencyFamily, EEW_FILTER_FAMILIES, RECORD_FILTER_FAMILIES};

const MAG_OPTS: [f64; 12] = [
    -1.0, 0.0, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0, 5.5,
];

fn mag_label(v: f64) -> String {
    if v < 0.0 {
        "不接收".into()
    } else if v == 0.0 {
        "不过滤".into()
    } else {
        format!("M {v:.1}")
    }
}

pub fn draw_eew_and_record_filters(ui: &mut Ui, draft: &mut AppConfig) {
    section(ui, "预警机构开关", "\u{26a1}", |ui| {
        ui.label(
            RichText::new("关闭后对应机构预警不进列表。")
                .size(12.0)
                .color(*MUTED),
        );
        ui.add_space(10.0);
        let mut first = true;
        for fam in EEW_FILTER_FAMILIES {
            if !first {
                row_divider(ui);
            }
            first = false;
            let key = fam.as_filter_key();
            let mut on = draft.filters.eew_agency_on(key);
            control_row(
                ui,
                fam.display_name(),
                &format!("[{}]", fam_bracket(fam)),
                "\u{25ce}",
                |ui| {
                    if toggle(ui, &mut on).changed() {
                        draft.filters.set_eew_agency(key, on);
                    }
                },
            );
        }
    });

    section(ui, "信息事件震级过滤", "\u{25ce}", |ui| {
        ui.label(
            RichText::new("低于阈值的速报不显示。白名单可绕过震级，不能绕过「不接收」。")
                .size(12.0)
                .color(*MUTED),
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
            let subtitle = if fam == AgencyFamily::Other {
                "未登记机构；默认不接收"
            } else {
                "低于阈值不显示"
            };
            control_row(ui, fam.display_name(), subtitle, "\u{25ce}", |ui| {
                egui::ComboBox::from_id_salt(format!("mag_filter_{key}"))
                    .selected_text(mag_label(val))
                    .width(110.0)
                    .show_ui(ui, |ui| {
                        for opt in MAG_OPTS {
                            ui.selectable_value(&mut val, opt, mag_label(opt));
                        }
                    });
            });
            draft.filters.set_record_mag(key, val);
        }
        row_divider(ui);
        control_row(ui, "地点白名单", "关键词用 | 分隔", "\u{25ce}", |ui| {
            framed_edit(ui, &mut draft.filters.place_whitelist, 200.0);
        });
    });
}

fn fam_bracket(fam: AgencyFamily) -> &'static str {
    match fam {
        AgencyFamily::Jma => "JMA",
        AgencyFamily::Cn => "CN",
        AgencyFamily::Usgs => "USGS",
        AgencyFamily::Cwa => "CWA",
        AgencyFamily::Kma => "KMA",
        AgencyFamily::EarlyEst => "Early-est",
        AgencyFamily::Ningxia => "宁夏地震局",
        AgencyFamily::Yunnan => "云南地震局",
        AgencyFamily::Beijing => "北京地震局",
        AgencyFamily::Hko => "HKO",
        AgencyFamily::Emsc => "EMSC",
        AgencyFamily::Sa => "SA",
        AgencyFamily::Other => "其他",
    }
}
