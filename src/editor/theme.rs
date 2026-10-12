//! Colors and global style: dark hardware look with an orange accent.

use nih_plug_egui::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle};

pub const BG: Color32 = Color32::from_rgb(0x1e, 0x20, 0x24);
pub const PANEL: Color32 = Color32::from_rgb(0x2a, 0x2d, 0x32);
pub const PANEL_DARK: Color32 = Color32::from_rgb(0x16, 0x18, 0x1b);
pub const KNOB_BODY: Color32 = Color32::from_rgb(0x3a, 0x3e, 0x45);
pub const TRACK: Color32 = Color32::from_rgb(0x15, 0x16, 0x19);
pub const TEXT: Color32 = Color32::from_rgb(0xdc, 0xde, 0xe3);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x8a, 0x8f, 0x98);
pub const ACCENT: Color32 = Color32::from_rgb(0xf0, 0x8a, 0x3c);
pub const WARN: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);
pub const WAVE: Color32 = Color32::from_rgb(0x9f, 0xd6, 0xc9);
pub const MARKER_START: Color32 = Color32::from_rgb(0x6c, 0xd0, 0x7a);
pub const MARKER_END: Color32 = Color32::from_rgb(0xe8, 0x6a, 0x5a);

/// One color per pad row, like the colored pad banks on classic drum machines.
const ROW_COLORS: [Color32; 4] = [
    Color32::from_rgb(0xe0, 0x62, 0x3a),
    Color32::from_rgb(0xe8, 0xb2, 0x3a),
    Color32::from_rgb(0x4f, 0xb3, 0xa9),
    Color32::from_rgb(0x6c, 0x8e, 0xe8),
];

pub fn pad_color(pad: usize) -> Color32 {
    ROW_COLORS[(pad / 4) % ROW_COLORS.len()]
}

pub fn small() -> FontId {
    FontId::new(10.0, FontFamily::Proportional)
}

pub fn label() -> FontId {
    FontId::new(11.0, FontFamily::Proportional)
}

pub fn heading() -> FontId {
    FontId::new(15.0, FontFamily::Proportional)
}

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    let v = &mut style.visuals;
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = PANEL_DARK;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0f32, Color32::BLACK);
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(4);
    }
    v.widgets.inactive.weak_bg_fill = KNOB_BODY;
    v.widgets.inactive.bg_fill = KNOB_BODY;
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x48, 0x4d, 0x55);
    v.widgets.active.weak_bg_fill = ACCENT;

    style.spacing.item_spacing = egui::vec2(6.0, 6.0);
    style.spacing.button_padding = egui::vec2(8.0, 3.0);
    style.text_styles.insert(
        TextStyle::Button,
        FontId::new(12.0, FontFamily::Proportional),
    );
    style
        .text_styles
        .insert(TextStyle::Body, FontId::new(12.0, FontFamily::Proportional));
    ctx.set_style(style);
}
