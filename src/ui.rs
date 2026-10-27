// SPDX-License-Identifier: AGPL-3.0-or-later
//! The control panel.

use std::path::PathBuf;
use std::time::Duration;

use eframe::egui::text::{LayoutJob, TextWrapping};
use eframe::egui::{
    self, Align, Align2, Button, Color32, CornerRadius, FontId, Frame, Id, Layout, Margin, Pos2,
    Rect, Response, RichText, Sense, Stroke, StrokeKind, TextFormat, Vec2, ViewportCommand,
};

use crate::app::{App, ItemStatus, Tab};
use crate::config::{ColorKey, Item, SpriteSheet};
use crate::i18n::Lang;
use crate::media::{self, decode::MediaKind};
use crate::theme::*;

const MEDIA_EXTENSIONS: &[&str] = &[
    "gif", "png", "apng", "webp", "jpg", "jpeg", "bmp", "tga", "tif", "tiff", "qoi", "mp4", "m4v",
    "webm", "mov", "mkv", "avi", "oapack",
];
const AUDIO_EXTENSIONS: &[&str] = &["mp3", "wav", "ogg", "flac", "m4a", "aac", "opus"];

const TITLE_BAR_HEIGHT: f32 = 46.0;
const INSPECTOR_WIDTH: f32 = 360.0;
const CARD_MIN_WIDTH: f32 = 170.0;
const GAP: f32 = 14.0;
const CODE_BG: Color32 = Color32::from_rgb(0x0c, 0x0e, 0x12);
const THUMB_BG: Color32 = Color32::from_rgb(0x13, 0x16, 0x1c);

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let dropped: Vec<PathBuf> =
        ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
    if !dropped.is_empty() {
        app.add_files(&ctx, dropped);
    }

    title_bar(app, ui);
    if !app.jobs.is_empty() || app.notice.is_some() {
        egui::Panel::bottom("status")
            .frame(Frame::new().fill(SURFACE).inner_margin(Margin::symmetric(16, 6)))
            .show(ui, |ui| status_bar(app, ui));
    }

    match app.tab {
        Tab::Library => library(app, ui),
        Tab::Behaviors => behaviors(app, ui),
        Tab::Settings => settings(app, ui),
    }

    confirm_dialog(app, &ctx);
    drop_overlay(app, &ctx);
    resize_handles(ui);
}

/// The window has no system frame, so its edges are resized by hand.
fn resize_handles(ui: &mut egui::Ui) {
    use egui::{CursorIcon, ResizeDirection as Dir};
    let ctx = ui.ctx().clone();
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let r = ctx.content_rect();
    let (e, c) = (5.0, 12.0);
    let x = |a: f32, b: f32| egui::Rangef::new(a, b);
    let handles = [
        (Dir::NorthWest, x(r.left(), r.left() + c), x(r.top(), r.top() + c), CursorIcon::ResizeNwSe),
        (Dir::NorthEast, x(r.right() - c, r.right()), x(r.top(), r.top() + c), CursorIcon::ResizeNeSw),
        (Dir::SouthWest, x(r.left(), r.left() + c), x(r.bottom() - c, r.bottom()), CursorIcon::ResizeNeSw),
        (Dir::SouthEast, x(r.right() - c, r.right()), x(r.bottom() - c, r.bottom()), CursorIcon::ResizeNwSe),
        (Dir::North, x(r.left() + c, r.right() - c), x(r.top(), r.top() + e), CursorIcon::ResizeVertical),
        (Dir::South, x(r.left() + c, r.right() - c), x(r.bottom() - e, r.bottom()), CursorIcon::ResizeVertical),
        (Dir::West, x(r.left(), r.left() + e), x(r.top() + c, r.bottom() - c), CursorIcon::ResizeHorizontal),
        (Dir::East, x(r.right() - e, r.right()), x(r.top() + c, r.bottom() - c), CursorIcon::ResizeHorizontal),
    ];
    for (dir, xs, ys, cursor) in handles {
        let rect = Rect::from_x_y_ranges(xs, ys);
        let response = ui.interact(rect, Id::new(("resize", dir as u8)), Sense::drag());
        if response.hovered() || response.dragged() {
            ctx.set_cursor_icon(cursor);
        }
        if response.drag_started_by(egui::PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
        }
    }
}

// -------------------------------------------------------------------------------------------
// Building blocks

fn primary_button(text: impl Into<String>) -> Button<'static> {
    Button::new(RichText::new(text).font(semibold(14.0)).color(Color32::WHITE))
        .fill(ACCENT)
        .corner_radius(RADIUS)
        .min_size(Vec2::new(0.0, 34.0))
}

fn ghost_button(text: impl Into<String>) -> Button<'static> {
    Button::new(RichText::new(text).color(TEXT))
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(RADIUS)
        .min_size(Vec2::new(0.0, 34.0))
}

fn danger_button(text: impl Into<String>) -> Button<'static> {
    Button::new(RichText::new(text).color(DANGER))
        .fill(CARD)
        .stroke(Stroke::new(1.0, DANGER.gamma_multiply(0.45)))
        .corner_radius(RADIUS)
        .min_size(Vec2::new(0.0, 34.0))
}

fn wide(ui: &mut egui::Ui, button: Button<'static>) -> Response {
    ui.add_sized([ui.available_width(), 38.0], button)
}

/// Small grey title above a group of controls.
fn caption(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text.to_uppercase()).font(semibold(11.5)).color(TEXT_DIM));
}

fn card_frame() -> Frame {
    Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CARD_RADIUS)
        .inner_margin(Margin::same(14))
}

/// A collapsible card of settings.
fn section(ui: &mut egui::Ui, title: &str, open: bool, add: impl FnOnce(&mut egui::Ui)) {
    card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::CollapsingHeader::new(RichText::new(title).font(semibold(14.5)).color(TEXT))
            .id_salt(title)
            .default_open(open)
            .show_unindented(ui, |ui| {
                ui.add_space(4.0);
                add(ui);
            });
    });
    ui.add_space(6.0);
}

/// iOS-style switch.
fn toggle(ui: &mut egui::Ui, on: &mut bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(38.0, 22.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(response.id, *on);
    let off = Color32::from_rgb(0x3a, 0x40, 0x4e);
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(11), lerp_color(off, ACCENT, t));
    let x = egui::lerp(rect.left() + 11.0..=rect.right() - 11.0, t);
    painter.circle_filled(Pos2::new(x, rect.center().y), 8.0, Color32::WHITE);
    response
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Label on the left, control on the right.
fn row<R>(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(TEXT));
        ui.with_layout(Layout::right_to_left(Align::Center), add).inner
    })
    .inner
}

fn toggle_row(ui: &mut egui::Ui, label: &str, value: &mut bool) -> bool {
    row(ui, label, |ui| toggle(ui, value).changed())
}

fn slider_row(ui: &mut egui::Ui, label: &str, slider: egui::Slider<'_>) -> Response {
    ui.label(RichText::new(label).color(TEXT));
    ui.spacing_mut().slider_width = (ui.available_width() - 72.0).max(60.0);
    ui.add(slider)
}

fn percent(v: f64, _: std::ops::RangeInclusive<usize>) -> String {
    format!("{:.0} %", v * 100.0)
}

fn parse_percent(s: &str) -> Option<f64> {
    s.trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0)
}

fn checkerboard(painter: &egui::Painter, rect: Rect, radius: u8) {
    painter.rect_filled(rect, radius, Color32::from_rgb(0x2a, 0x2e, 0x38));
    let cell = 10.0;
    let cols = (rect.width() / cell).ceil() as i32;
    let rows = (rect.height() / cell).ceil() as i32;
    let clipped = painter.with_clip_rect(rect.shrink(radius as f32 / 2.0));
    for y in 0..rows {
        for x in 0..cols {
            if (x + y) % 2 == 0 {
                let min = rect.min + Vec2::new(x as f32 * cell, y as f32 * cell);
                clipped.rect_filled(
                    Rect::from_min_size(min, Vec2::splat(cell)),
                    0.0,
                    Color32::from_rgb(0x22, 0x26, 0x2f),
                );
            }
        }
    }
}

/// Paints `tex` (of natural size `size`) centered and fitted inside `rect`.
fn paint_fit(painter: &egui::Painter, rect: Rect, tex: egui::TextureId, size: Vec2, tint: Color32) {
    if size.x <= 0.0 || size.y <= 0.0 {
        return;
    }
    let scale = (rect.size() / size).min_elem().min(2.0);
    let target = Rect::from_center_size(rect.center(), size * scale);
    painter.image(tex, target, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), tint);
}

fn paint_logo(painter: &egui::Painter, center: Pos2, radius: f32) {
    painter.circle_filled(center, radius, ACCENT);
    let r = radius * 0.5;
    let points = vec![
        center + Vec2::new(-r * 0.6, -r),
        center + Vec2::new(r * 1.1, 0.0),
        center + Vec2::new(-r * 0.6, r),
    ];
    painter.add(egui::Shape::convex_polygon(points, Color32::WHITE, Stroke::NONE));
}

fn truncated(
    painter: &egui::Painter,
    text: &str,
    font: FontId,
    color: Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::single_section(
        text.to_owned(),
        TextFormat {
            font_id: font,
            color,
            ..Default::default()
        },
    );
    job.wrap = TextWrapping::truncate_at_width(width);
    painter.layout_job(job)
}

/// Green "on desktop" / "running" badge in the corner of a card.
fn badge(painter: &egui::Painter, at: Pos2, text: &str) {
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(11.0), SUCCESS);
    let pill = Rect::from_min_size(at, galley.size() + Vec2::new(22.0, 6.0));
    painter.rect_filled(pill, CornerRadius::same(10), Color32::from_black_alpha(170));
    painter.circle_filled(Pos2::new(pill.left() + 9.0, pill.center().y), 3.0, SUCCESS);
    painter.galley(Pos2::new(pill.left() + 16.0, pill.top() + 3.0), galley, SUCCESS);
}

/// Background, border, thumbnail area and texts shared by the library and behavior cards.
/// Returns the thumbnail area.
fn paint_card(
    painter: &egui::Painter,
    rect: Rect,
    thumb_h: f32,
    hovered: bool,
    selected: bool,
    title: &str,
    subtitle: &str,
) -> Rect {
    painter.rect_filled(rect, CARD_RADIUS, if hovered { CARD_HOVER } else { CARD });
    let stroke = if selected {
        Stroke::new(2.0, ACCENT)
    } else {
        Stroke::new(1.0, BORDER)
    };
    painter.rect_stroke(rect, CARD_RADIUS, stroke, StrokeKind::Inside);
    let thumb = Rect::from_min_size(
        rect.min + Vec2::splat(8.0),
        Vec2::new(rect.width() - 16.0, thumb_h - 8.0),
    );
    painter.rect_filled(thumb, RADIUS, THUMB_BG);
    let width = rect.width() - 24.0;
    let galley = truncated(painter, title, semibold(13.5), TEXT, width);
    painter.galley(Pos2::new(rect.left() + 12.0, thumb.bottom() + 9.0), galley, TEXT);
    let galley = truncated(painter, subtitle, FontId::proportional(11.5), TEXT_DIM, width - 48.0);
    painter.galley(Pos2::new(rect.left() + 12.0, thumb.bottom() + 30.0), galley, TEXT_DIM);
    thumb
}

fn empty_state(ui: &mut egui::Ui, icon: &str, title: &str, hint: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.25);
        ui.label(RichText::new(icon).size(40.0).color(TEXT_DIM));
        ui.add_space(4.0);
        ui.label(RichText::new(title).font(semibold(16.0)).color(TEXT));
        if !hint.is_empty() {
            ui.label(RichText::new(hint).color(TEXT_DIM));
        }
    });
}

fn side_panel_frame() -> Frame {
    Frame::new()
        .fill(SURFACE)
        .inner_margin(Margin::symmetric(16, 14))
        .stroke(Stroke::new(1.0, BORDER))
}

fn content_frame() -> Frame {
    Frame::new().fill(BG).inner_margin(Margin {
        left: 20,
        right: 20,
        top: 16,
        bottom: 0,
    })
}

// -------------------------------------------------------------------------------------------
// Window chrome

fn title_bar(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    egui::Panel::top("title_bar")
        .exact_size(TITLE_BAR_HEIGHT)
        .frame(Frame::new().fill(SURFACE))
        .show(ui, |ui| {
            let bar = ui.max_rect();
            // Registered first so that the widgets on top of it get the clicks.
            let drag = ui.interact(bar, Id::new("title_bar_drag"), Sense::click_and_drag());
            if drag.drag_started_by(egui::PointerButton::Primary) {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            }
            if drag.double_clicked() {
                let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
            }
            ui.painter()
                .hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, BORDER));

            ui.horizontal_centered(|ui| {
                ui.add_space(16.0);
                let (logo, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                paint_logo(ui.painter(), logo.center(), 11.0);
                ui.add_space(2.0);
                ui.label(RichText::new("Open Anima").font(semibold(15.0)).color(TEXT));
                ui.add_space(28.0);

                for (tab, key) in [
                    (Tab::Library, "tab_library"),
                    (Tab::Behaviors, "tab_behaviors"),
                    (Tab::Settings, "tab_settings"),
                ] {
                    if tab_button(ui, app.tab == tab, app.t(key), bar.bottom()).clicked() {
                        app.tab = tab;
                    }
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    window_buttons(app, ui, &ctx, bar.height());
                });
            });
        });
}

fn tab_button(ui: &mut egui::Ui, selected: bool, text: &str, bottom: f32) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), semibold(14.0), Color32::PLACEHOLDER);
    let size = galley.size() + Vec2::new(24.0, 16.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let color = if selected || response.hovered() {
        TEXT
    } else {
        TEXT_DIM
    };
    if response.hovered() && !selected {
        ui.painter().rect_filled(rect, RADIUS, CARD);
    }
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, color);
    if selected {
        let line = Rect::from_min_max(
            Pos2::new(rect.left() + 10.0, bottom - 3.0),
            Pos2::new(rect.right() - 10.0, bottom),
        );
        ui.painter().rect_filled(line, CornerRadius::same(2), ACCENT);
    }
    response
}

#[derive(Clone, Copy)]
enum WindowButton {
    Close,
    Maximize,
    Minimize,
}

fn window_buttons(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context, height: f32) {
    // Laid out right to left.
    for kind in [WindowButton::Close, WindowButton::Maximize, WindowButton::Minimize] {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(46.0, height), Sense::click());
        let hovered = response.hovered();
        if hovered {
            let fill = match kind {
                WindowButton::Close => Color32::from_rgb(0xc4, 0x2b, 0x1c),
                _ => CARD_HOVER,
            };
            ui.painter().rect_filled(rect, 0.0, fill);
        }
        let stroke = Stroke::new(1.0, if hovered { Color32::WHITE } else { TEXT_DIM });
        let c = rect.center();
        let p = ui.painter();
        match kind {
            WindowButton::Close => {
                let d = 5.0;
                p.line_segment([c + Vec2::new(-d, -d), c + Vec2::new(d, d)], stroke);
                p.line_segment([c + Vec2::new(-d, d), c + Vec2::new(d, -d)], stroke);
            }
            WindowButton::Maximize => {
                p.rect_stroke(
                    Rect::from_center_size(c, Vec2::splat(10.0)),
                    1.0,
                    stroke,
                    StrokeKind::Inside,
                );
            }
            WindowButton::Minimize => {
                p.hline(c.x - 5.0..=c.x + 5.0, c.y, stroke);
            }
        }
        let tooltip = match kind {
            WindowButton::Close => app.t("close"),
            WindowButton::Maximize => app.t("maximize"),
            WindowButton::Minimize => app.t("minimize"),
        };
        if response.on_hover_text(tooltip).clicked() {
            match kind {
                WindowButton::Close => ctx.send_viewport_cmd(ViewportCommand::Close),
                WindowButton::Maximize => {
                    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                }
                WindowButton::Minimize => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
            }
        }
    }
}

fn status_bar(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        for job in &app.jobs {
            ui.spinner();
            ui.label(RichText::new(&job.label).color(TEXT));
            ui.add(
                egui::ProgressBar::new(job.progress())
                    .desired_width(140.0)
                    .desired_height(8.0)
                    .fill(ACCENT)
                    .corner_radius(4),
            );
            ui.add_space(12.0);
        }
        if let Some(notice) = &app.notice {
            if notice.at.elapsed() < Duration::from_secs(8) {
                let (icon, color) = if notice.error {
                    ("⚠", DANGER)
                } else {
                    ("✔", SUCCESS)
                };
                ui.label(RichText::new(icon).color(color));
                ui.label(RichText::new(&notice.text).color(TEXT));
                ui.ctx().request_repaint_after(Duration::from_secs(1));
            } else {
                app.notice = None;
            }
        }
    });
}

fn drop_overlay(app: &App, ctx: &egui::Context) {
    if ctx.input(|i| i.raw.hovered_files.is_empty()) {
        return;
    }
    let rect = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, "drop".into()));
    painter.rect_filled(rect, 0.0, Color32::from_black_alpha(200));
    let zone = rect.shrink(28.0);
    painter.rect_filled(zone, CARD_RADIUS, ACCENT.gamma_multiply(0.08));
    painter.rect_stroke(zone, CARD_RADIUS, Stroke::new(2.0, ACCENT), StrokeKind::Inside);
    painter.text(
        zone.center() - Vec2::new(0.0, 24.0),
        Align2::CENTER_CENTER,
        "⬇",
        FontId::proportional(44.0),
        ACCENT_HOVER,
    );
    painter.text(
        zone.center() + Vec2::new(0.0, 22.0),
        Align2::CENTER_CENTER,
        app.t("drop_hint"),
        semibold(18.0),
        TEXT,
    );
}

// -------------------------------------------------------------------------------------------
// Library

fn library(app: &mut App, ui: &mut egui::Ui) {
    egui::Panel::right("inspector")
        .exact_size(INSPECTOR_WIDTH)
        .resizable(false)
        .frame(side_panel_frame())
        .show(ui, |ui| inspector(app, ui));

    egui::CentralPanel::default()
        .frame(content_frame())
        .show(ui, |ui| {
            let ctx = ui.ctx().clone();
            ui.horizontal(|ui| {
                if ui.add(primary_button(app.t("add_animations"))).clicked() {
                    pick_media(app, &ctx);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let mut visible = !app.cfg.overlays_hidden;
                    if toggle(ui, &mut visible)
                        .on_hover_text(app.t("hide_all"))
                        .changed()
                    {
                        app.set_overlays_hidden(!visible);
                    }
                    ui.label(RichText::new(app.t("overlays_visible")).color(TEXT_DIM));
                    ui.add_space(12.0);
                    let hint = format!("🔍  {}", app.t("search"));
                    ui.add(
                        egui::TextEdit::singleline(&mut app.search)
                            .hint_text(hint)
                            .desired_width(ui.available_width())
                            .margin(Margin::symmetric(10, 8)),
                    );
                });
            });
            ui.add_space(6.0);

            let total = app.cfg.items.len();
            let shown = app.cfg.items.iter().filter(|i| i.enabled).count();
            ui.label(
                RichText::new(format!(
                    "{total} {}  ·  {shown} {}",
                    app.t("animations_count"),
                    app.t("on_desktop_count")
                ))
                .color(TEXT_DIM)
                .small(),
            );
            ui.add_space(8.0);

            if app.cfg.items.is_empty() {
                library_empty(app, ui);
                return;
            }
            let needle = app.search.to_lowercase();
            let ids: Vec<u64> = app
                .cfg
                .items
                .iter()
                .filter(|i| needle.is_empty() || i.name.to_lowercase().contains(&needle))
                .map(|i| i.id)
                .collect();
            if ids.is_empty() {
                empty_state(ui, "🔍", app.t("no_results"), "");
                return;
            }
            grid(ui, &ids, |ui, id, width| library_card(app, ui, id, width));
        });
}

fn pick_media(app: &mut App, ctx: &egui::Context) {
    if let Some(files) = rfd::FileDialog::new()
        .add_filter("Media", MEDIA_EXTENSIONS)
        .pick_files()
    {
        app.add_files(ctx, files);
    }
}

fn grid(ui: &mut egui::Ui, ids: &[u64], mut card: impl FnMut(&mut egui::Ui, u64, f32)) {
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let available = ui.available_width();
        let columns = ((available + GAP) / (CARD_MIN_WIDTH + GAP)).floor().max(1.0) as usize;
        let width = (available - GAP * (columns as f32 - 1.0)) / columns as f32;
        for chunk in ids.chunks(columns) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                for &id in chunk {
                    card(ui, id, width);
                }
            });
            ui.add_space(GAP - ui.spacing().item_spacing.y);
        }
        ui.add_space(12.0);
    });
}

fn library_empty(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let free = ui.available_rect_before_wrap();
    let zone = Rect::from_min_size(
        free.min + Vec2::new(0.0, 8.0),
        Vec2::new(free.width(), (free.height() - 24.0).clamp(200.0, 380.0)),
    );
    ui.painter().rect_filled(zone, CARD_RADIUS, SURFACE);
    ui.painter()
        .rect_stroke(zone, CARD_RADIUS, Stroke::new(1.5, BORDER), StrokeKind::Inside);
    ui.scope_builder(egui::UiBuilder::new().max_rect(zone), |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(zone.height() * 0.22);
            ui.label(RichText::new("✨").size(42.0));
            ui.add_space(6.0);
            ui.label(RichText::new(app.t("drop_title")).font(semibold(18.0)).color(TEXT));
            ui.label(RichText::new(app.t("drop_formats")).color(TEXT_DIM));
            ui.add_space(12.0);
            if ui.add(primary_button(app.t("add_animations"))).clicked() {
                pick_media(app, &ctx);
            }
        });
    });
}

fn library_card(app: &mut App, ui: &mut egui::Ui, id: u64, width: f32) {
    let Some(item) = app.cfg.item(id).cloned() else { return };
    let ctx = ui.ctx().clone();
    let thumb_h = (width * 0.78).round();
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, thumb_h + 56.0), Sense::click());

    let kind = item
        .source
        .extension()
        .map(|e| e.to_string_lossy().to_uppercase())
        .unwrap_or_default();
    let subtitle = if item.decode.sprite_sheet.is_some() {
        format!("{kind} · {}", app.t("sprites"))
    } else {
        kind
    };
    let name = if item.name.is_empty() { "—" } else { &item.name };
    let painter = ui.painter().clone();
    let thumb = paint_card(
        &painter,
        rect,
        thumb_h,
        response.hovered(),
        app.selected == Some(id),
        name,
        &subtitle,
    );
    match app.card_image(&ctx, id) {
        Some(image) => {
            let tint = if item.enabled {
                Color32::WHITE
            } else {
                Color32::from_gray(150)
            };
            paint_fit(&painter, thumb.shrink(8.0), image.tex, image.size, tint);
            if let Some(next) = image.next {
                ctx.request_repaint_after(Duration::from_secs_f64(next));
            }
        }
        None => {
            painter.text(
                thumb.center(),
                Align2::CENTER_CENTER,
                "🎞",
                FontId::proportional(26.0),
                TEXT_DIM,
            );
        }
    }
    if item.enabled {
        badge(&painter, thumb.min + Vec2::new(8.0, 8.0), app.t("on_desktop"));
    }

    // Quick on/off switch in the corner.
    let switch = Rect::from_min_size(
        Pos2::new(rect.right() - 50.0, thumb.bottom() + 14.0),
        Vec2::new(38.0, 22.0),
    );
    let mut enabled = item.enabled;
    ui.scope_builder(egui::UiBuilder::new().max_rect(switch), |ui| {
        toggle(ui, &mut enabled).on_hover_text(app.t("show_on_desktop"))
    });

    if response.clicked() {
        app.selected = Some(id);
    }
    if response.double_clicked() {
        enabled = !item.enabled;
    }
    response.context_menu(|ui| {
        let label = if item.enabled { app.t("unlaunch") } else { app.t("launch") };
        if ui.button(label).clicked() {
            enabled = !item.enabled;
            ui.close();
        }
        if ui.button(app.t("duplicate")).clicked() {
            app.duplicate_item(id);
            ui.close();
        }
        if ui.button(RichText::new(app.t("remove")).color(DANGER)).clicked() {
            app.remove_item(id);
            ui.close();
        }
    });
    if enabled != item.enabled
        && let Some(item) = app.cfg.item_mut(id)
    {
        item.enabled = enabled;
    }
}

fn inspector(app: &mut App, ui: &mut egui::Ui) {
    let Some(original) = app.selected.and_then(|id| app.cfg.item(id)).cloned() else {
        empty_state(ui, "🖱", app.t("select_item"), "");
        return;
    };
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| editor(app, ui, original));
}

#[derive(Default)]
struct Actions {
    ai: bool,
    export: bool,
    duplicate: bool,
    remove: bool,
}

fn editor(app: &mut App, ui: &mut egui::Ui, original: Item) {
    let ctx = ui.ctx().clone();
    let lang = app.lang();
    let t = |k| lang.t(k);
    let mut item = original.clone();
    let is_video = media::decode::kind_of(&item.source) == MediaKind::Video;
    let status = app.item_status(item.id);
    let mut actions = Actions::default();

    caption(ui, t("selected_animation"));
    ui.add_space(2.0);

    // Preview.
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 210.0), Sense::hover());
    checkerboard(ui.painter(), rect, CARD_RADIUS);
    let mut info = String::new();
    match &status {
        ItemStatus::Loading => {
            ui.put(rect, egui::Spinner::new().size(28.0).color(ACCENT));
        }
        ItemStatus::Failed(err) => {
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink(16.0)), |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new(*err).color(DANGER));
                });
            });
        }
        ItemStatus::Ready(anim) => {
            let mut anim = anim.lock().unwrap();
            let frame = anim.frame(media::clock(), &item.playback);
            let size = Vec2::new(anim.size[0] as f32, anim.size[1] as f32);
            let tint = Color32::WHITE.gamma_multiply(item.look.opacity);
            paint_fit(ui.painter(), rect.shrink(12.0), frame.tex, size, tint);
            info = match anim.frame_count() {
                Some(n) => format!("{}×{} px  ·  {n} {}", anim.size[0], anim.size[1], t("frames_n")),
                None => format!("{}×{} px  ·  {}", anim.size[0], anim.size[1], t("video")),
            };
            ctx.request_repaint_after(Duration::from_secs_f64(frame.next));
        }
    }
    ui.add_space(4.0);

    ui.add(
        egui::TextEdit::singleline(&mut item.name)
            .font(semibold(16.0))
            .desired_width(ui.available_width())
            .margin(Margin::symmetric(10, 7)),
    );
    ui.horizontal(|ui| {
        ui.label(RichText::new(info).color(TEXT_DIM).small());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .add(Button::new(RichText::new(t("change")).small().color(ACCENT_HOVER)).frame(false))
                .on_hover_text(item.source.display().to_string())
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Media", MEDIA_EXTENSIONS)
                    .pick_file()
            {
                item.source = path;
            }
        });
    });
    ui.add_space(2.0);

    let launch = if item.enabled {
        ghost_button(t("unlaunch"))
    } else {
        primary_button(t("launch"))
    };
    if wide(ui, launch).clicked() {
        item.enabled = !item.enabled;
    }
    ui.add_space(8.0);

    section(ui, t("sec_look"), true, |ui| {
        slider_row(
            ui,
            t("scale"),
            egui::Slider::new(&mut item.look.scale, 0.05..=6.0)
                .logarithmic(true)
                .custom_formatter(percent)
                .custom_parser(parse_percent),
        );
        slider_row(
            ui,
            t("opacity"),
            egui::Slider::new(&mut item.look.opacity, 0.05..=1.0)
                .custom_formatter(percent)
                .custom_parser(parse_percent),
        );
        toggle_row(ui, t("flip_h"), &mut item.decode.flip_h);
        toggle_row(ui, t("flip_v"), &mut item.decode.flip_v);
    });

    section(ui, t("sec_playback"), true, |ui| {
        slider_row(
            ui,
            t("speed"),
            egui::Slider::new(&mut item.playback.speed, 0.1..=5.0)
                .logarithmic(true)
                .suffix("×"),
        );
        let mut forced = item.playback.fps_override.is_some();
        if toggle_row(ui, t("fps_override"), &mut forced) {
            item.playback.fps_override = forced.then_some(24.0);
        }
        if let Some(fps) = &mut item.playback.fps_override {
            row(ui, t("fps"), |ui| {
                ui.add(egui::DragValue::new(fps).range(1.0..=240.0).suffix(" fps"))
            });
        }
        toggle_row(ui, t("paused"), &mut item.playback.paused);
    });

    let mut moved = None;
    section(ui, t("sec_interaction"), false, |ui| {
        toggle_row(ui, t("draggable"), &mut item.window.draggable);
        toggle_row(ui, t("click_through"), &mut item.window.click_through);
        toggle_row(ui, t("always_on_top"), &mut item.window.always_on_top);
        row(ui, t("sec_position"), |ui| {
            let mut pos = item.pos;
            if ui.button("⊕").on_hover_text(t("center")).clicked()
                && let ItemStatus::Ready(anim) = &status
            {
                let size = crate::overlay::window_size(anim.lock().unwrap().size, &item.look);
                moved = Some((app.screen_size(&ctx) / 2.0 - size / 2.0).to_pos2());
            }
            let y = ui.add(egui::DragValue::new(&mut pos[1]).prefix("y ").speed(1.0));
            let x = ui.add(egui::DragValue::new(&mut pos[0]).prefix("x ").speed(1.0));
            if x.changed() || y.changed() {
                moved = Some(Pos2::new(pos[0], pos[1]));
            }
        });
    });

    section(ui, t("sec_audio"), false, |ui| {
        row(ui, t("audio_file"), |ui| {
            if item.audio.file.is_some()
                && ui.small_button("✕").on_hover_text(t("clear")).clicked()
            {
                item.audio.file = None;
            }
            let name = item
                .audio
                .file
                .as_ref()
                .and_then(|f| f.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| t("browse").to_owned());
            if ui.button(name).clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Audio", AUDIO_EXTENSIONS)
                    .pick_file()
            {
                item.audio.file = Some(path);
            }
        });
        if is_video {
            toggle_row(ui, t("video_sound"), &mut item.audio.video_sound);
        }
        slider_row(
            ui,
            t("volume"),
            egui::Slider::new(&mut item.audio.volume, 0.0..=1.0).custom_formatter(percent),
        );
    });

    if !is_video {
        section(ui, t("sec_sprite"), item.decode.sprite_sheet.is_some(), |ui| {
            let mut is_sheet = item.decode.sprite_sheet.is_some();
            if toggle_row(ui, t("is_sprite"), &mut is_sheet) {
                item.decode.sprite_sheet = is_sheet.then(SpriteSheet::default);
            }
            if let Some(sheet) = &mut item.decode.sprite_sheet {
                row(ui, t("columns"), |ui| {
                    ui.add(egui::DragValue::new(&mut sheet.columns).range(1..=256))
                });
                row(ui, t("rows"), |ui| {
                    ui.add(egui::DragValue::new(&mut sheet.rows).range(1..=256))
                });
                row(ui, t("frames"), |ui| {
                    ui.add(egui::DragValue::new(&mut sheet.frames).range(0..=65536))
                });
                row(ui, t("fps"), |ui| {
                    ui.add(egui::DragValue::new(&mut sheet.fps).range(0.5..=120.0))
                });
            }
        });
    }

    section(ui, t("sec_background"), item.decode.color_key.is_some(), |ui| {
        let mut keyed = item.decode.color_key.is_some();
        if toggle_row(ui, t("color_key"), &mut keyed) {
            item.decode.color_key = keyed.then(|| ColorKey {
                color: first_frame_corner(&item).unwrap_or([0, 255, 0]),
                ..Default::default()
            });
        }
        if let Some(key) = &mut item.decode.color_key {
            row(ui, t("key_color"), |ui| {
                ui.color_edit_button_srgb(&mut key.color);
                if ui.small_button(t("pick_corner")).clicked()
                    && let Some(color) = first_frame_corner(&original)
                {
                    key.color = color;
                }
            });
            slider_row(ui, t("tolerance"), egui::Slider::new(&mut key.tolerance, 0.0..=1.0));
            slider_row(ui, t("softness"), egui::Slider::new(&mut key.softness, 0.0..=0.5));
        }
        ui.add_space(4.0);
        ai_button(app, ui, &mut actions);
    });

    ui.add_space(4.0);
    ui.columns(3, |cols| {
        let add = |ui: &mut egui::Ui, button: Button<'static>| {
            ui.add_sized([ui.available_width(), 34.0], button).clicked()
        };
        actions.export = add(&mut cols[0], ghost_button(t("export_short")));
        actions.duplicate = add(&mut cols[1], ghost_button(t("duplicate")));
        actions.remove = add(&mut cols[2], danger_button(t("remove")));
    });
    ui.add_space(12.0);

    // Apply the edits, then the actions (which may remove or replace the item).
    let id = original.id;
    if item != original
        && let Some(slot) = app.cfg.item_mut(id)
    {
        *slot = item.clone();
    }
    if let Some(pos) = moved {
        app.move_item(id, pos);
    }
    if actions.ai {
        start_ai(app, &ctx, id);
    }
    if actions.export
        && let Some(out) = rfd::FileDialog::new()
            .add_filter("APNG", &["png"])
            .set_file_name(format!("{}.png", item.name))
            .save_file()
    {
        app.start_export(&ctx, id, out);
    }
    if actions.duplicate {
        app.duplicate_item(id);
    }
    if actions.remove {
        app.remove_item(id);
    }
}

#[cfg(feature = "ai")]
fn ai_button(app: &App, ui: &mut egui::Ui, actions: &mut Actions) {
    let busy = !app.jobs.is_empty();
    let button = Button::new(RichText::new(app.t("ai_remove")).color(Color32::WHITE))
        .fill(ACCENT_SOFT)
        .stroke(Stroke::new(1.0, ACCENT))
        .corner_radius(RADIUS)
        .min_size(Vec2::new(ui.available_width(), 34.0));
    let mut response = ui.add_enabled(!busy, button);
    if app.cfg.ai_model.is_none() {
        response = response.on_hover_text(app.t("ai_need_model"));
    }
    if response.clicked() {
        actions.ai = true;
    }
}

#[cfg(not(feature = "ai"))]
fn ai_button(app: &App, ui: &mut egui::Ui, _actions: &mut Actions) {
    ui.add_enabled(false, ghost_button(app.t("ai_remove")))
        .on_disabled_hover_text(app.t("ai_unavailable"));
}

#[cfg(feature = "ai")]
fn start_ai(app: &mut App, ctx: &egui::Context, id: u64) {
    app.start_ai(ctx, id);
}

#[cfg(not(feature = "ai"))]
fn start_ai(_app: &mut App, _ctx: &egui::Context, _id: u64) {}

fn first_frame_corner(item: &Item) -> Option<[u8; 3]> {
    if media::decode::kind_of(&item.source) != MediaKind::Image {
        return None;
    }
    let frames = media::decode::decode_image(&item.source).ok()?;
    Some(media::process::corner_color(&frames.first()?.image))
}

// -------------------------------------------------------------------------------------------
// Behaviors

fn behaviors(app: &mut App, ui: &mut egui::Ui) {
    if app.selected_behavior.is_none() && !app.behaviors.is_empty() {
        app.selected_behavior = Some(0);
    }
    egui::Panel::right("behavior_inspector")
        .exact_size(INSPECTOR_WIDTH)
        .resizable(false)
        .frame(side_panel_frame())
        .show(ui, |ui| behavior_inspector(app, ui));

    egui::CentralPanel::default()
        .frame(content_frame())
        .show(ui, |ui| {
            let ctx = ui.ctx().clone();
            ui.horizontal(|ui| {
                if ui.add(primary_button(app.t("load_package"))).clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Open Anima package", &["oapack"])
                        .pick_file()
                {
                    app.install_package(&ctx, &path);
                }
                if ui.add(ghost_button(app.t("load_folder"))).clicked()
                    && let Some(dir) = rfd::FileDialog::new()
                        .set_title(app.t("pick_folder_for_package"))
                        .pick_folder()
                {
                    app.add_behavior_dir(&ctx, dir);
                }
                if ui.add(ghost_button(app.t("create_package"))).clicked() {
                    create_package(app);
                }
            });
            ui.add_space(6.0);
            ui.label(RichText::new(app.t("behaviors_intro")).color(TEXT_DIM).small());
            ui.add_space(10.0);

            if app.behaviors.is_empty() {
                empty_state(ui, "🤖", app.t("no_behaviors"), app.t("no_behaviors_hint"));
                return;
            }
            let ids: Vec<u64> = (0..app.behaviors.len() as u64).collect();
            grid(ui, &ids, |ui, index, width| {
                behavior_card(app, ui, index as usize, width)
            });
        });
}

fn behavior_card(app: &mut App, ui: &mut egui::Ui, index: usize, width: f32) {
    let b = &app.behaviors[index];
    let thumb_h = (width * 0.62).round();
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, thumb_h + 56.0), Sense::click());
    let m = &b.package.manifest;
    let subtitle = if m.author.is_empty() {
        m.version.clone()
    } else {
        m.author.clone()
    };
    let painter = ui.painter().clone();
    let thumb = paint_card(
        &painter,
        rect,
        thumb_h,
        response.hovered(),
        app.selected_behavior == Some(index),
        &m.name,
        &subtitle,
    );
    match &b.preview {
        Some(tex) => paint_fit(
            &painter,
            thumb.shrink(8.0),
            tex.id(),
            tex.size_vec2(),
            Color32::WHITE,
        ),
        None => {
            painter.text(
                thumb.center(),
                Align2::CENTER_CENTER,
                "🤖",
                FontId::proportional(28.0),
                TEXT_DIM,
            );
        }
    }
    if b.running.is_some() {
        badge(&painter, thumb.min + Vec2::new(8.0, 8.0), app.t("running"));
    }
    if response.clicked() {
        app.selected_behavior = Some(index);
    }
}

fn behavior_inspector(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let Some(index) = app.selected_behavior.filter(|&i| i < app.behaviors.len()) else {
        empty_state(ui, "🖱", app.t("select_behavior"), "");
        return;
    };
    let lang = app.lang();
    let t = |k| lang.t(k);
    let mut action = None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let b = &app.behaviors[index];
        let m = &b.package.manifest;
        caption(ui, t("selected_behavior"));
        ui.add_space(2.0);
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 170.0), Sense::hover());
        checkerboard(ui.painter(), rect, CARD_RADIUS);
        if let Some(tex) = &b.preview {
            paint_fit(ui.painter(), rect.shrink(12.0), tex.id(), tex.size_vec2(), Color32::WHITE);
        }
        ui.add_space(4.0);
        ui.label(RichText::new(&m.name).font(semibold(18.0)).color(TEXT));
        let mut meta = Vec::new();
        if !m.version.is_empty() {
            meta.push(format!("v{}", m.version));
        }
        if !m.author.is_empty() {
            meta.push(m.author.clone());
        }
        if !meta.is_empty() {
            ui.label(RichText::new(meta.join("  ·  ")).color(TEXT_DIM).small());
        }
        if !m.description.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new(&m.description).color(TEXT));
        }
        ui.add_space(8.0);
        let running = b.running.is_some();
        let button = if running {
            ghost_button(t("stop"))
        } else {
            primary_button(t("run"))
        };
        if wide(ui, button).clicked() {
            action = Some(if running { "stop" } else { "run" });
        }
        ui.add_space(8.0);

        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            caption(ui, t("command"));
            ui.label(RichText::new(m.command.join(" ")).monospace().color(TEXT));
            ui.add_space(4.0);
            ui.label(
                RichText::new(b.package.dir.display().to_string())
                    .small()
                    .color(TEXT_DIM),
            );
        });
        ui.add_space(6.0);

        card_frame().fill(CODE_BG).show(ui, |ui| {
            ui.set_width(ui.available_width());
            caption(ui, t("log"));
            egui::ScrollArea::vertical()
                .id_salt("log")
                .max_height(220.0)
                .stick_to_bottom(true)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if b.log.is_empty() {
                        ui.label(RichText::new("—").color(TEXT_DIM));
                    }
                    for line in &b.log {
                        ui.label(RichText::new(line).monospace().color(TEXT_DIM));
                    }
                });
        });
        ui.add_space(8.0);
        if wide(ui, danger_button(t("remove_package"))).clicked() {
            action = Some("remove");
        }
    });
    match action {
        Some("run") => app.run_behavior(&ctx, index, false),
        Some("stop") => app.stop_behavior(index),
        Some("remove") => app.remove_behavior(&ctx, index),
        _ => {}
    }
}

fn create_package(app: &mut App) {
    let Some(dir) = rfd::FileDialog::new()
        .set_title(app.t("pick_folder_for_package"))
        .pick_folder()
    else {
        return;
    };
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "behavior".into());
    let Some(out) = rfd::FileDialog::new()
        .add_filter("Open Anima package", &["oapack"])
        .set_file_name(format!("{name}.oapack"))
        .save_file()
    else {
        return;
    };
    match crate::behavior::create_package(&dir, &out) {
        Ok(()) => app.notify(format!("{} — {}", app.t("package_created"), out.display()), false),
        Err(err) => app.notify(format!("{}: {err:#}", app.t("error")), true),
    }
}

fn confirm_dialog(app: &mut App, ctx: &egui::Context) {
    let Some(index) = app.confirm_run else { return };
    let Some(b) = app.behaviors.get(index) else {
        app.confirm_run = None;
        return;
    };
    let lang = app.lang();
    let (name, command) = (b.package.manifest.name.clone(), b.package.manifest.command.join(" "));
    let mut choice = None;
    let modal = egui::Modal::new(Id::new("confirm_run"))
        .frame(
            Frame::new()
                .fill(SURFACE)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(CARD_RADIUS)
                .inner_margin(Margin::same(22)),
        )
        .show(ctx, |ui| {
            ui.set_max_width(440.0);
            ui.label(RichText::new("⚠").size(28.0).color(WARNING));
            ui.label(RichText::new(&name).font(semibold(18.0)).color(TEXT));
            ui.add_space(4.0);
            ui.label(RichText::new(lang.t("confirm_run")).color(TEXT));
            ui.add_space(6.0);
            card_frame().fill(CODE_BG).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(&command).monospace().color(TEXT));
            });
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.add(primary_button(lang.t("run"))).clicked() {
                    choice = Some(true);
                }
                if ui.add(ghost_button(lang.t("cancel"))).clicked() {
                    choice = Some(false);
                }
            });
        });
    if modal.should_close() && choice.is_none() {
        choice = Some(false);
    }
    if let Some(run) = choice {
        app.confirm_run = None;
        if run {
            app.run_behavior(ctx, index, true);
        }
    }
}

// -------------------------------------------------------------------------------------------
// Settings

fn settings(app: &mut App, ui: &mut egui::Ui) {
    let lang = app.lang();
    egui::CentralPanel::default()
        .frame(Frame::new().fill(BG).inner_margin(Margin::symmetric(20, 16)))
        .show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                let width = ui.available_width().min(680.0);
                let pad = ((ui.available_width() - width) / 2.0).max(0.0);
                ui.horizontal(|ui| {
                    ui.add_space(pad);
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        ui.label(
                            RichText::new(lang.t("tab_settings"))
                                .font(semibold(24.0))
                                .color(TEXT),
                        );
                        ui.add_space(8.0);
                        settings_cards(app, ui);
                    });
                });
            });
        });
}

fn settings_cards(app: &mut App, ui: &mut egui::Ui) {
    let lang = app.lang();
    let t = |k| lang.t(k);

    section(ui, t("sec_general"), true, |ui| {
        row(ui, t("language"), |ui| {
            egui::ComboBox::from_id_salt("lang")
                .selected_text(app.cfg.lang.native_name())
                .show_ui(ui, |ui| {
                    for l in Lang::ALL {
                        ui.selectable_value(&mut app.cfg.lang, l, l.native_name());
                    }
                });
        });
        let mut autostart = app.autostart_enabled;
        if toggle_row(ui, t("autostart"), &mut autostart) {
            app.set_autostart(autostart);
        }
        toggle_row(ui, t("close_to_tray"), &mut app.cfg.close_to_tray);
        slider_row(
            ui,
            t("master_volume"),
            egui::Slider::new(&mut app.cfg.master_volume, 0.0..=1.0).custom_formatter(percent),
        );
    });

    section(ui, t("sec_video"), true, |ui| {
        let (dot, text) = match (&app.cfg.ffmpeg, app.ffmpeg()) {
            (Some(p), Some(_)) => (SUCCESS, p.display().to_string()),
            (None, Some(p)) => (SUCCESS, format!("{} — {}", t("ffmpeg_auto"), p.display())),
            (_, None) => (WARNING, t("ffmpeg_missing").to_owned()),
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("●").color(dot));
            ui.label(RichText::new(text).color(TEXT_DIM).small());
        });
        ui.horizontal(|ui| {
            if ui.add(ghost_button(t("browse"))).clicked()
                && let Some(p) = rfd::FileDialog::new().pick_file()
            {
                app.cfg.ffmpeg = Some(p);
            }
            if app.cfg.ffmpeg.is_some() && ui.add(ghost_button(t("clear"))).clicked() {
                app.cfg.ffmpeg = None;
            }
        });
    });

    section(ui, t("sec_ai"), true, |ui| {
        if !cfg!(feature = "ai") {
            ui.label(RichText::new(t("ai_unavailable")).color(TEXT_DIM));
            return;
        }
        let (dot, text) = match &app.cfg.ai_model {
            Some(p) => (SUCCESS, p.display().to_string()),
            None => (TEXT_DIM, t("none").to_owned()),
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("●").color(dot));
            ui.label(RichText::new(text).color(TEXT_DIM).small());
        });
        ui.label(RichText::new(t("ai_model_hint")).color(TEXT_DIM).small());
        if ui.add(ghost_button(t("browse"))).clicked()
            && let Some(p) = rfd::FileDialog::new()
                .add_filter("ONNX", &["onnx"])
                .pick_file()
        {
            app.cfg.ai_model = Some(p);
        }
    });

    section(ui, t("sec_data"), false, |ui| {
        if ui.add(ghost_button(t("open_data_dir"))).clicked() {
            open_in_file_manager(app.dirs.library.parent().unwrap_or(&app.dirs.library));
        }
    });

    card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (logo, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
            paint_logo(ui.painter(), logo.center(), 22.0);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(format!("Open Anima {}", env!("CARGO_PKG_VERSION")))
                        .font(semibold(16.0))
                        .color(TEXT),
                );
                ui.label(RichText::new(t("license_text")).color(TEXT_DIM).small());
                ui.hyperlink(env!("CARGO_PKG_REPOSITORY"));
            });
        });
    });
    ui.add_space(16.0);
}

fn open_in_file_manager(path: &std::path::Path) {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    if let Err(err) = std::process::Command::new(program).arg(path).spawn() {
        log::warn!("opening {}: {err}", path.display());
    }
}
