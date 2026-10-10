# -*- coding: utf-8 -*-
"""Polish RhythmQuake settings shell details in settings.rs (UTF-8 safe)."""
from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PATH = ROOT / "crates/jian-ui/src/settings.rs"


def main() -> None:
    text = PATH.read_text(encoding="utf-8")
    assert "设置中心" in text, "settings.rs lost UTF-8 Chinese; abort"

    # All Stroke::new(1.0, ...) -> 1.0_f32 (including multiline).
    text, n_float = re.subn(
        r"Stroke::new\(\s*1\.0\s*,",
        "Stroke::new(1.0_f32,",
        text,
    )

    new_fade = """fn paint_fade_rule(ui: &Ui, head: Rect, inset: f32) {
    // RQ .panel-nav-head / .content-head ::after — mid band with faded ends.
    let y = head.bottom() - 1.0;
    let left = head.left() + inset;
    let right = head.right() - inset;
    let span = (right - left).max(1.0);
    let stops: [(f32, i32); 5] = [
        (0.00, 0),
        (0.18, 30),
        (0.50, 30),
        (0.82, 30),
        (1.00, 0),
    ];
    for w in stops.windows(2) {
        let (t0, a0) = w[0];
        let (t1, a1) = w[1];
        let x0 = left + span * t0;
        let x1 = left + span * t1;
        let a = ((a0 + a1) / 2).max(1) as u8;
        ui.painter().line_segment(
            [Pos2::new(x0, y), Pos2::new(x1, y)],
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, a)),
        );
    }
}"""
    text, n_fade = re.subn(
        r"fn paint_fade_rule\(ui: &Ui, head: Rect, inset: f32\) \{.*?\n\}",
        new_fade,
        text,
        count=1,
        flags=re.S,
    )
    if n_fade != 1:
        raise SystemExit(f"paint_fade_rule replace failed ({n_fade})")

    # Wrap shell body with inset margin (idempotent).
    if "shell inset" not in text:
        text = text.replace(
            "fn draw_shell(\n"
            "    ui: &mut Ui,\n"
            "    session: &mut SettingsSession,\n"
            "    health: &SourceHealthView,\n"
            "    action: &mut Option<SettingsAction>,\n"
            "    restore: &mut bool,\n"
            "    do_exchange: &mut bool,\n"
            ") {\n"
            "    ui.allocate_ui_with_layout(\n",
            "fn draw_shell(\n"
            "    ui: &mut Ui,\n"
            "    session: &mut SettingsSession,\n"
            "    health: &SourceHealthView,\n"
            "    action: &mut Option<SettingsAction>,\n"
            "    restore: &mut bool,\n"
            "    do_exchange: &mut bool,\n"
            ") {\n"
            "    // RQ .shell inset\n"
            "    Frame::NONE\n"
            "        .inner_margin(Margin::symmetric(28, 18))\n"
            "        .show(ui, |ui| {\n"
            "    ui.allocate_ui_with_layout(\n",
            1,
        )
        text = text.replace(
            "                });\n"
            "        },\n"
            "    );\n"
            "}\n"
            "\n"
            "fn draw_nav(",
            "                });\n"
            "        },\n"
            "    );\n"
            "        }); // shell inset\n"
            "}\n"
            "\n"
            "fn draw_nav(",
            1,
        )
        if "shell inset" not in text:
            raise SystemExit("shell inset wrap failed")

    # Soften page-header margins after inset wrap.
    text = text.replace(
        ".inner_margin(Margin::symmetric(22, 14))\n"
        "                .show(ui, |ui| {\n"
        "                    ui.horizontal(|ui| {\n"
        "                        ui.vertical(|ui| {\n"
        '                            ui.label(\n'
        '                                RichText::new("设置中心")',
        ".inner_margin(Margin::symmetric(4, 6))\n"
        "                .show(ui, |ui| {\n"
        "                    ui.horizontal(|ui| {\n"
        "                        ui.vertical(|ui| {\n"
        "                            ui.label(\n"
        '                                RichText::new("设置中心")',
        1,
    )

    # Nav selected: white glass underlay (idempotent marker).
    if "nav-btn.active base" not in text:
        m = re.search(
            r"(let stroke = if selected \{.*?Stroke::NONE\n                    \};\n)",
            text,
            flags=re.S,
        )
        if not m:
            raise SystemExit("nav stroke block not found")
        inject = (
            m.group(1)
            + "                    // nav-btn.active base\n"
            + "                    if selected {\n"
            + "                        ui.painter().rect_filled(\n"
            + "                            rect,\n"
            + "                            10.0,\n"
            + "                            Color32::from_rgba_unmultiplied(255, 255, 255, 26),\n"
            + "                        );\n"
            + "                    }\n"
        )
        text = text[: m.start(1)] + inject + text[m.end(1) :]

    # glass button height 40 like RQ .btn
    text, n_btn = re.subn(
        r"\.min_size\(Vec2::new\(if emphasized \{ 108\.0 \} else \{ 76\.0 \}, 36\.0\)\)",
        ".min_size(Vec2::new(if emphasized { 108.0 } else { 76.0 }, 40.0))",
        text,
        count=1,
    )

    # Hover ring on toggle (idempotent).
    if "toggle hover ring" not in text:
        old = """    ui.painter()
        .circle_filled(Pos2::new(thumb_x, rect.center().y), 10.0, thumb_c);
    resp
}

pub(super) fn framed_edit"""
        new = """    ui.painter()
        .circle_filled(Pos2::new(thumb_x, rect.center().y), 10.0, thumb_c);
    // toggle hover ring
    if resp.hovered() {
        ui.painter().rect_stroke(
            rect,
            14.0,
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(0x82, 0xB1, 0xFF, 90)),
            egui::StrokeKind::Outside,
        );
    }
    resp
}

pub(super) fn framed_edit"""
        if old not in text:
            raise SystemExit("toggle tail not found")
        text = text.replace(old, new, 1)

    assert "设置中心" in text
    PATH.write_text(text, encoding="utf-8", newline="\n")
    print(
        f"polished {PATH.relative_to(ROOT)}: "
        f"floats={n_float} fade={n_fade} btn={n_btn} "
        f"inset={'yes' if 'shell inset' in text else 'no'}"
    )


if __name__ == "__main__":
    main()
