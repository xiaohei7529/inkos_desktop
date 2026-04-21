//! 现代暗色主题：调色板、视觉与控件样式（参考当下流行的开发者工具风格）。

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle, Visuals,
};

#[allow(dead_code)]
pub mod color {
    use eframe::egui::Color32;

    pub const BG: Color32 = Color32::from_rgb(0x0f, 0x11, 0x16);
    pub const PANEL: Color32 = Color32::from_rgb(0x14, 0x17, 0x1d);
    pub const SURFACE: Color32 = Color32::from_rgb(0x1a, 0x1e, 0x26);
    pub const SURFACE_HI: Color32 = Color32::from_rgb(0x22, 0x27, 0x31);
    pub const BORDER: Color32 = Color32::from_rgb(0x2a, 0x30, 0x3c);
    pub const BORDER_HI: Color32 = Color32::from_rgb(0x3a, 0x42, 0x52);

    pub const TEXT: Color32 = Color32::from_rgb(0xe7, 0xea, 0xf1);
    pub const TEXT_DIM: Color32 = Color32::from_rgb(0x9a, 0xa3, 0xb2);
    pub const TEXT_FAINT: Color32 = Color32::from_rgb(0x6a, 0x73, 0x82);

    pub const ACCENT: Color32 = Color32::from_rgb(0x7c, 0x8a, 0xff);
    pub const ACCENT_HI: Color32 = Color32::from_rgb(0x95, 0xa1, 0xff);
    pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x3a, 0x44, 0x82);

    pub const SUCCESS: Color32 = Color32::from_rgb(0x4a, 0xde, 0x80);
    pub const WARNING: Color32 = Color32::from_rgb(0xf5, 0x9e, 0x0b);
    pub const DANGER: Color32 = Color32::from_rgb(0xef, 0x44, 0x44);
}

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.global_style()).clone();

    style.text_styles = [
        (TextStyle::Heading, FontId::new(20.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
        (TextStyle::Button, FontId::new(13.5, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
    ]
    .into();

    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.window_margin = Margin::same(0);
    style.spacing.menu_margin = Margin::same(8);
    style.spacing.indent = 18.0;
    style.spacing.interact_size.y = 28.0;

    let mut v = Visuals::dark();
    v.dark_mode = true;
    v.override_text_color = Some(color::TEXT);
    v.window_fill = color::SURFACE;
    v.panel_fill = color::BG;
    v.faint_bg_color = color::SURFACE;
    v.extreme_bg_color = color::PANEL;
    v.code_bg_color = color::PANEL;
    v.window_stroke = Stroke::new(1.0, color::BORDER);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.selection.bg_fill = color::ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0, color::ACCENT);
    v.hyperlink_color = color::ACCENT_HI;

    let r = CornerRadius::same(8);
    v.widgets.noninteractive.bg_fill = color::SURFACE;
    v.widgets.noninteractive.weak_bg_fill = color::SURFACE;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, color::BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, color::TEXT_DIM);
    v.widgets.noninteractive.corner_radius = r;

    v.widgets.inactive.bg_fill = color::SURFACE_HI;
    v.widgets.inactive.weak_bg_fill = color::SURFACE_HI;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, color::BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, color::TEXT);
    v.widgets.inactive.corner_radius = r;

    v.widgets.hovered.bg_fill = color::BORDER_HI;
    v.widgets.hovered.weak_bg_fill = color::BORDER_HI;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, color::ACCENT);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, color::TEXT);
    v.widgets.hovered.corner_radius = r;

    v.widgets.active.bg_fill = color::ACCENT_DIM;
    v.widgets.active.weak_bg_fill = color::ACCENT_DIM;
    v.widgets.active.bg_stroke = Stroke::new(1.0, color::ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    v.widgets.active.corner_radius = r;

    v.widgets.open.bg_fill = color::SURFACE_HI;
    v.widgets.open.weak_bg_fill = color::SURFACE_HI;
    v.widgets.open.bg_stroke = Stroke::new(1.0, color::ACCENT);
    v.widgets.open.fg_stroke = Stroke::new(1.0, color::TEXT);
    v.widgets.open.corner_radius = r;

    style.visuals = v;
    ctx.set_global_style(style);
}

pub fn card_frame() -> egui::Frame {
    egui::Frame::default()
        .fill(color::SURFACE)
        .stroke(Stroke::new(1.0, color::BORDER))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(16, 14))
}

pub fn pill(ui: &mut egui::Ui, text: &str, fg: Color32, bg: Color32) {
    egui::Frame::default()
        .fill(bg)
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).color(fg).size(11.5));
        });
}

pub fn dim_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(color::TEXT_DIM).size(12.5));
}

pub fn page_header(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.add_space(2.0);
    ui.label(egui::RichText::new(title).size(22.0).strong().color(color::TEXT));
    if !subtitle.is_empty() {
        ui.label(egui::RichText::new(subtitle).size(13.0).color(color::TEXT_DIM));
    }
    ui.add_space(10.0);
}

pub fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    let mut spaced = String::new();
    let upper = text.to_uppercase();
    let mut chars = upper.chars().peekable();
    while let Some(c) = chars.next() {
        spaced.push(c);
        if chars.peek().is_some() {
            spaced.push(' ');
        }
    }
    ui.label(
        egui::RichText::new(spaced)
            .size(11.0)
            .color(color::TEXT_FAINT),
    );
    ui.add_space(2.0);
}

pub fn empty_state(ui: &mut egui::Ui, title: &str, hint: &str) {
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        ui.label(egui::RichText::new(title).size(15.0).color(color::TEXT));
        ui.add_space(4.0);
        ui.label(egui::RichText::new(hint).size(12.5).color(color::TEXT_DIM));
    });
}
