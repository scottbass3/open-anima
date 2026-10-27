// SPDX-License-Identifier: AGPL-3.0-or-later
//! Look of the control panel: colors, fonts, spacing.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke,
    TextStyle, Vec2,
};

/// Window background, behind everything.
pub const BG: Color32 = Color32::from_rgb(0x10, 0x12, 0x17);
/// Side panels and the title bar.
pub const SURFACE: Color32 = Color32::from_rgb(0x16, 0x19, 0x20);
/// Cards and inputs.
pub const CARD: Color32 = Color32::from_rgb(0x1d, 0x21, 0x2a);
pub const CARD_HOVER: Color32 = Color32::from_rgb(0x25, 0x2a, 0x35);
pub const BORDER: Color32 = Color32::from_rgb(0x2a, 0x30, 0x3c);
pub const ACCENT: Color32 = Color32::from_rgb(0x7c, 0x4d, 0xff);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0x93, 0x6d, 0xff);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(0x2b, 0x22, 0x4d);
pub const TEXT: Color32 = Color32::from_rgb(0xe8, 0xea, 0xf0);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x8b, 0x91, 0xa1);
pub const SUCCESS: Color32 = Color32::from_rgb(0x3d, 0xd6, 0x8c);
pub const DANGER: Color32 = Color32::from_rgb(0xff, 0x5c, 0x70);
pub const WARNING: Color32 = Color32::from_rgb(0xff, 0xb4, 0x4d);

pub const RADIUS: u8 = 8;
pub const CARD_RADIUS: u8 = 12;

/// Name of the semibold font family, for titles.
pub const SEMIBOLD: &str = "semibold";

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, visuals());
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(11.5)),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, FontId::proportional(14.0)),
            (TextStyle::Heading, semibold(20.0)),
            (TextStyle::Monospace, FontId::monospace(12.5)),
        ]
        .into();
        let s = &mut style.spacing;
        s.item_spacing = Vec2::new(8.0, 8.0);
        s.button_padding = Vec2::new(14.0, 6.0);
        s.interact_size = Vec2::new(40.0, 30.0);
        s.slider_width = 150.0;
        s.slider_rail_height = 4.0;
        s.combo_width = 180.0;
        s.window_margin = Margin::same(16);
        s.menu_margin = Margin::same(6);
        s.scroll.bar_width = 6.0;
        s.scroll.floating = true;
    });
}

/// Segoe UI from Windows when available (it isn't redistributed), egui's fonts otherwise.
/// egui's own fonts stay as fallbacks for the icons and emojis, then Windows' CJK fonts.
fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let defaults = fonts.families[&FontFamily::Proportional].clone();
    let mut regular = defaults.clone();
    let mut bold = defaults;

    let windir = std::env::var_os("WINDIR").map(std::path::PathBuf::from);
    let load = |file: &str| -> Option<Vec<u8>> {
        std::fs::read(windir.as_ref()?.join("Fonts").join(file)).ok()
    };
    if let Some(data) = load("segoeui.ttf") {
        fonts
            .font_data
            .insert("segoe".into(), Arc::new(FontData::from_owned(data)));
        regular.insert(0, "segoe".into());
    }
    if let Some(data) = load("seguisb.ttf") {
        fonts
            .font_data
            .insert("segoe-semibold".into(), Arc::new(FontData::from_owned(data)));
        bold.insert(0, "segoe-semibold".into());
    } else {
        bold = regular.clone();
    }
    // Workshop titles are often Japanese or Korean.
    for (name, file) in [("cjk-ja", "msgothic.ttc"), ("cjk-ko", "malgun.ttf")] {
        if let Some(data) = load(file) {
            fonts
                .font_data
                .insert(name.into(), Arc::new(FontData::from_owned(data)));
            regular.push(name.into());
            bold.push(name.into());
        }
    }
    fonts.families.insert(FontFamily::Proportional, regular);
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), bold);
    fonts
}

fn visuals() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    let radius = CornerRadius::same(RADIUS);
    v.panel_fill = BG;
    v.window_fill = SURFACE;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = CornerRadius::same(CARD_RADIUS);
    v.menu_corner_radius = radius;
    v.extreme_bg_color = Color32::from_rgb(0x0c, 0x0e, 0x12);
    v.text_edit_bg_color = Some(CARD);
    v.faint_bg_color = CARD;
    v.code_bg_color = Color32::from_rgb(0x0c, 0x0e, 0x12);
    v.hyperlink_color = ACCENT_HOVER;
    v.warn_fg_color = WARNING;
    v.error_fg_color = DANGER;
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, TEXT);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.collapsing_header_frame = false;
    v.indent_has_left_vline = false;
    v.interact_cursor = Some(egui::CursorIcon::PointingHand);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = SURFACE;
    w.noninteractive.weak_bg_fill = SURFACE;
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_DIM);
    w.noninteractive.corner_radius = radius;

    w.inactive.bg_fill = CARD_HOVER;
    w.inactive.weak_bg_fill = CARD;
    w.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.corner_radius = radius;
    w.inactive.expansion = 0.0;

    w.hovered.bg_fill = Color32::from_rgb(0x30, 0x36, 0x44);
    w.hovered.weak_bg_fill = CARD_HOVER;
    w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x3a, 0x41, 0x50));
    w.hovered.fg_stroke = Stroke::new(1.5, TEXT);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = ACCENT;
    w.active.weak_bg_fill = Color32::from_rgb(0x2c, 0x32, 0x3f);
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    w.active.fg_stroke = Stroke::new(1.5, TEXT);
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;

    w.open.bg_fill = CARD_HOVER;
    w.open.weak_bg_fill = CARD_HOVER;
    w.open.bg_stroke = Stroke::new(1.0, ACCENT);
    w.open.fg_stroke = Stroke::new(1.0, TEXT);
    w.open.corner_radius = radius;
    v
}
