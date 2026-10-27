// SPDX-License-Identifier: AGPL-3.0-or-later
//! The control panel.

use std::path::PathBuf;
use std::time::Duration;

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Vec2};

use crate::app::{App, ItemStatus, Tab};
use crate::config::{ColorKey, Item, SpriteSheet};
use crate::i18n::Lang;
use crate::media::{self, decode::MediaKind};

const MEDIA_EXTENSIONS: &[&str] = &[
    "gif", "png", "apng", "webp", "jpg", "jpeg", "bmp", "tga", "tif", "tiff", "qoi", "mp4", "m4v",
    "webm", "mov", "mkv", "avi", "oapack",
];
const AUDIO_EXTENSIONS: &[&str] = &["mp3", "wav", "ogg", "flac", "m4a", "aac", "opus"];

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let dropped: Vec<PathBuf> =
        ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
    if !dropped.is_empty() {
        app.add_files(&ctx, dropped);
    }

    egui::Panel::top("tabs").show(ui, |ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Open Anima").strong());
            ui.add_space(12.0);
            for (tab, key) in [
                (Tab::Library, "tab_library"),
                (Tab::Behaviors, "tab_behaviors"),
                (Tab::Settings, "tab_settings"),
            ] {
                let label = app.t(key);
                ui.selectable_value(&mut app.tab, tab, label);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut hidden = app.cfg.overlays_hidden;
                if ui.toggle_value(&mut hidden, app.t("hide_all")).changed() {
                    app.set_overlays_hidden(hidden);
                }
            });
        });
        ui.add_space(2.0);
    });

    egui::Panel::bottom("status").show(ui, |ui| status_bar(app, ui));

    match app.tab {
        Tab::Library => library(app, ui),
        Tab::Behaviors => behaviors(app, ui),
        Tab::Settings => settings(app, ui),
    }

    confirm_dialog(app, &ctx);

    if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
        let rect = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, "drop".into()));
        painter.rect_filled(rect, 0.0, Color32::from_black_alpha(180));
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            app.t("drop_hint"),
            egui::FontId::proportional(22.0),
            Color32::WHITE,
        );
    }
}

fn status_bar(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        for job in &app.jobs {
            ui.spinner();
            ui.label(&job.label);
            ui.add(egui::ProgressBar::new(job.progress()).desired_width(120.0).show_percentage());
            ui.separator();
        }
        if let Some(notice) = &app.notice {
            if notice.at.elapsed() < Duration::from_secs(8) {
                let color = if notice.error {
                    ui.visuals().error_fg_color
                } else {
                    ui.visuals().text_color()
                };
                ui.label(RichText::new(&notice.text).color(color));
                ui.ctx().request_repaint_after(Duration::from_secs(1));
            } else {
                app.notice = None;
            }
        }
    });
}

// -------------------------------------------------------------------------------------------
// Library

fn library(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    egui::Panel::left("library")
        .resizable(true)
        .default_size(230.0)
        .show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(app.t("add")).clicked()
                    && let Some(files) = rfd::FileDialog::new()
                        .add_filter("Media", MEDIA_EXTENSIONS)
                        .pick_files()
                {
                    app.add_files(&ctx, files);
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for item in &mut app.cfg.items {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut item.enabled, "");
                        let selected = app.selected == Some(item.id);
                        let name = if item.name.is_empty() { "—" } else { &item.name };
                        if ui.selectable_label(selected, name).clicked() {
                            app.selected = Some(item.id);
                        }
                    });
                }
            });
        });

    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            if app.cfg.items.is_empty() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(app.t("empty_library"));
                    ui.add_space(8.0);
                    ui.weak(app.t("drop_hint"));
                });
                return;
            }
            match app.selected.and_then(|id| app.cfg.item(id)).cloned() {
                Some(item) => editor(app, ui, item),
                None => {
                    ui.label(app.t("select_item"));
                }
            }
        });
    });
}

fn editor(app: &mut App, ui: &mut egui::Ui, original: Item) {
    let ctx = ui.ctx().clone();
    let lang = app.lang();
    let t = |k| lang.t(k);
    let mut item = original.clone();
    let kind = media::decode::kind_of(&item.source);
    let status = app.item_status(item.id);
    let is_video = kind == MediaKind::Video;

    ui.horizontal(|ui| {
        ui.label(t("name"));
        ui.add(egui::TextEdit::singleline(&mut item.name).desired_width(220.0));
        ui.checkbox(&mut item.enabled, t("show_on_desktop"));
    });
    ui.horizontal(|ui| {
        ui.label(t("source"));
        ui.weak(item.source.display().to_string());
        if ui.small_button(t("change")).clicked()
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Media", MEDIA_EXTENSIONS)
                .pick_file()
        {
            item.source = path;
        }
    });
    ui.add_space(6.0);

    // Preview.
    let box_size = Vec2::new(ui.available_width().min(360.0), 220.0);
    let (rect, _) = ui.allocate_exact_size(box_size, egui::Sense::hover());
    checkerboard(ui.painter(), rect);
    match &status {
        ItemStatus::Loading => {
            ui.put(rect, egui::Spinner::new());
        }
        ItemStatus::Failed(err) => {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                *err,
                egui::FontId::proportional(13.0),
                ui.visuals().error_fg_color,
            );
        }
        ItemStatus::Ready(anim) => {
            let mut anim = anim.lock().unwrap();
            let frame = anim.frame(media::clock(), &item.playback);
            let size = Vec2::new(anim.size[0] as f32, anim.size[1] as f32);
            let fit = (rect.size() / size).min_elem().min(1.0) * size;
            let target = Rect::from_center_size(rect.center(), fit);
            ui.painter().image(
                frame.tex,
                target,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE.gamma_multiply(item.look.opacity),
            );
            let info = match anim.frame_count() {
                Some(n) => format!("{}×{} px · {n} frames", anim.size[0], anim.size[1]),
                None => format!("{}×{} px · video", anim.size[0], anim.size[1]),
            };
            ui.weak(info);
            ctx.request_repaint_after(Duration::from_secs_f64(frame.next));
        }
    }
    ui.add_space(6.0);

    let mut moved = None;
    section(ui, t("sec_position"), |ui| {
        ui.horizontal(|ui| {
            let mut pos = item.pos;
            let a = ui.add(egui::DragValue::new(&mut pos[0]).prefix("x ").speed(1.0));
            let b = ui.add(egui::DragValue::new(&mut pos[1]).prefix("y ").speed(1.0));
            if a.changed() || b.changed() {
                moved = Some(Pos2::new(pos[0], pos[1]));
            }
            if ui.button(t("center")).clicked()
                && let ItemStatus::Ready(anim) = &status
            {
                let size = crate::overlay::window_size(anim.lock().unwrap().size, &item.look);
                moved = Some((app.screen_size(&ctx) / 2.0 - size / 2.0).to_pos2());
            }
        });
    });

    section(ui, t("sec_look"), |ui| {
        ui.add(
            egui::Slider::new(&mut item.look.scale, 0.05..=6.0)
                .logarithmic(true)
                .text(t("scale")),
        );
        ui.add(egui::Slider::new(&mut item.look.opacity, 0.05..=1.0).text(t("opacity")));
        ui.checkbox(&mut item.decode.flip_h, t("flip_h"));
        ui.checkbox(&mut item.decode.flip_v, t("flip_v"));
    });

    section(ui, t("sec_playback"), |ui| {
        ui.add(
            egui::Slider::new(&mut item.playback.speed, 0.1..=5.0)
                .logarithmic(true)
                .text(t("speed")),
        );
        ui.horizontal(|ui| {
            let mut forced = item.playback.fps_override.is_some();
            if ui.checkbox(&mut forced, t("fps_override")).changed() {
                item.playback.fps_override = forced.then_some(24.0);
            }
            if let Some(fps) = &mut item.playback.fps_override {
                ui.add(egui::DragValue::new(fps).range(1.0..=240.0).suffix(" fps"));
            }
        });
        ui.checkbox(&mut item.playback.paused, t("paused"));
    });

    section(ui, t("sec_behavior"), |ui| {
        ui.checkbox(&mut item.window.draggable, t("draggable"));
        ui.checkbox(&mut item.window.click_through, t("click_through"));
        ui.checkbox(&mut item.window.always_on_top, t("always_on_top"));
    });

    section(ui, t("sec_audio"), |ui| {
        ui.horizontal(|ui| {
            ui.label(t("audio_file"));
            match &item.audio.file {
                Some(f) => ui.weak(f.file_name().unwrap_or_default().to_string_lossy()),
                None => ui.weak(t("none")),
            };
            if ui.small_button(t("browse")).clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Audio", AUDIO_EXTENSIONS)
                    .pick_file()
            {
                item.audio.file = Some(path);
            }
            if item.audio.file.is_some() && ui.small_button(t("clear")).clicked() {
                item.audio.file = None;
            }
        });
        if is_video {
            ui.checkbox(&mut item.audio.video_sound, t("video_sound"));
        }
        ui.add(egui::Slider::new(&mut item.audio.volume, 0.0..=1.0).text(t("volume")));
    });

    if !is_video {
        section(ui, t("sec_sprite"), |ui| {
            let mut is_sheet = item.decode.sprite_sheet.is_some();
            if ui.checkbox(&mut is_sheet, t("is_sprite")).changed() {
                item.decode.sprite_sheet = is_sheet.then(SpriteSheet::default);
            }
            if let Some(sheet) = &mut item.decode.sprite_sheet {
                egui::Grid::new("sheet").num_columns(2).show(ui, |ui| {
                    ui.label(t("columns"));
                    ui.add(egui::DragValue::new(&mut sheet.columns).range(1..=256));
                    ui.end_row();
                    ui.label(t("rows"));
                    ui.add(egui::DragValue::new(&mut sheet.rows).range(1..=256));
                    ui.end_row();
                    ui.label(t("frames"));
                    ui.add(egui::DragValue::new(&mut sheet.frames).range(0..=65536));
                    ui.end_row();
                    ui.label(t("fps"));
                    ui.add(egui::DragValue::new(&mut sheet.fps).range(0.5..=120.0));
                    ui.end_row();
                });
            }
        });
    }

    section(ui, t("sec_edit"), |ui| {
        let mut keyed = item.decode.color_key.is_some();
        if ui.checkbox(&mut keyed, t("color_key")).changed() {
            item.decode.color_key = keyed.then(|| ColorKey {
                color: first_frame_corner(&item).unwrap_or([0, 255, 0]),
                ..Default::default()
            });
        }
        if let Some(key) = &mut item.decode.color_key {
            ui.horizontal(|ui| {
                ui.color_edit_button_srgb(&mut key.color);
                if ui.button(t("pick_corner")).clicked()
                    && let Some(color) = first_frame_corner(&original)
                {
                    key.color = color;
                }
            });
            ui.add(egui::Slider::new(&mut key.tolerance, 0.0..=1.0).text(t("tolerance")));
            ui.add(egui::Slider::new(&mut key.softness, 0.0..=0.5).text(t("softness")));
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ai_button(app, ui, &ctx, item.id);
            if ui.button(t("export_apng")).clicked()
                && let Some(out) = rfd::FileDialog::new()
                    .add_filter("APNG", &["png"])
                    .set_file_name(format!("{}.png", item.name))
                    .save_file()
            {
                app.start_export(&ctx, item.id, out);
            }
        });
    });

    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if ui.button(t("duplicate")).clicked() {
            app.duplicate_item(item.id);
        }
        if ui.button(t("remove")).clicked() {
            app.remove_item(item.id);
        }
    });

    if app.cfg.item(original.id).is_some() {
        if item != original
            && let Some(slot) = app.cfg.item_mut(original.id)
        {
            *slot = item;
        }
        if let Some(pos) = moved {
            app.move_item(original.id, pos);
        }
    }
}

#[cfg(feature = "ai")]
fn ai_button(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context, id: u64) {
    let busy = !app.jobs.is_empty();
    let response = ui.add_enabled(!busy, egui::Button::new(app.t("ai_remove")));
    let response = if app.cfg.ai_model.is_none() {
        response.on_hover_text(app.t("ai_need_model"))
    } else {
        response
    };
    if response.clicked() {
        app.start_ai(ctx, id);
    }
}

#[cfg(not(feature = "ai"))]
fn ai_button(app: &mut App, ui: &mut egui::Ui, _ctx: &egui::Context, _id: u64) {
    ui.add_enabled(false, egui::Button::new(app.t("ai_remove")))
        .on_disabled_hover_text(app.t("ai_unavailable"));
}

fn first_frame_corner(item: &Item) -> Option<[u8; 3]> {
    if media::decode::kind_of(&item.source) != MediaKind::Image {
        return None;
    }
    let frames = media::decode::decode_image(&item.source).ok()?;
    Some(media::process::corner_color(&frames.first()?.image))
}

fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong())
        .default_open(true)
        .show(ui, add);
}

fn checkerboard(painter: &egui::Painter, rect: Rect) {
    let cell = 12.0;
    painter.rect_filled(rect, 4.0, Color32::from_gray(200));
    let cols = (rect.width() / cell).ceil() as i32;
    let rows = (rect.height() / cell).ceil() as i32;
    for y in 0..rows {
        for x in 0..cols {
            if (x + y) % 2 == 0 {
                let min = rect.min + Vec2::new(x as f32 * cell, y as f32 * cell);
                let r = Rect::from_min_size(min, Vec2::splat(cell)).intersect(rect);
                painter.rect_filled(r, 0.0, Color32::from_gray(160));
            }
        }
    }
}

// -------------------------------------------------------------------------------------------
// Behaviors

fn behaviors(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let lang = app.lang();
    let t = |k| lang.t(k);

    egui::Panel::top("behaviors_top").show(ui, |ui| {
        ui.add_space(4.0);
        ui.label(t("behaviors_intro"));
        ui.horizontal(|ui| {
            if ui.button(t("load_package")).clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Open Anima package", &["oapack"])
                    .pick_file()
            {
                app.install_package(&ctx, &path);
            }
            if ui.button(t("load_folder")).clicked()
                && let Some(dir) = rfd::FileDialog::new()
                    .set_title(t("pick_folder_for_package"))
                    .pick_folder()
            {
                app.add_behavior_dir(&ctx, dir);
            }
            if ui.button(t("create_package")).clicked() {
                create_package(app);
            }
        });
        ui.add_space(4.0);
    });

    egui::Panel::left("behavior_list")
        .resizable(true)
        .default_size(220.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (i, b) in app.behaviors.iter().enumerate() {
                    let dot = if b.running.is_some() { "🟢 " } else { "⚪ " };
                    let selected = app.selected_behavior == Some(i);
                    if ui
                        .selectable_label(selected, format!("{dot}{}", b.package.manifest.name))
                        .clicked()
                    {
                        app.selected_behavior = Some(i);
                    }
                }
            });
        });

    egui::CentralPanel::default().show(ui, |ui| {
        let Some(index) = app.selected_behavior.filter(|&i| i < app.behaviors.len()) else {
            return;
        };
        let mut action = None;
        {
            let b = &app.behaviors[index];
            let m = &b.package.manifest;
            ui.horizontal(|ui| {
                if let Some(tex) = &b.preview {
                    let size = tex.size_vec2();
                    let scale = (96.0 / size.max_elem()).min(1.0);
                    ui.image((tex.id(), size * scale));
                }
                ui.vertical(|ui| {
                    ui.heading(&m.name);
                    if !m.version.is_empty() {
                        ui.weak(format!("v{}", m.version));
                    }
                    if !m.author.is_empty() {
                        ui.label(format!("{} : {}", t("author"), m.author));
                    }
                    let state = if b.running.is_some() { t("running") } else { t("stopped") };
                    ui.label(state);
                });
            });
            if !m.description.is_empty() {
                ui.label(&m.description);
            }
            ui.horizontal(|ui| {
                ui.label(t("command"));
                ui.code(m.command.join(" "));
            });
            ui.weak(b.package.dir.display().to_string());
            ui.horizontal(|ui| {
                if b.running.is_some() {
                    if ui.button(t("stop")).clicked() {
                        action = Some("stop");
                    }
                } else if ui.button(t("run")).clicked() {
                    action = Some("run");
                }
                if ui.button(t("remove_package")).clicked() {
                    action = Some("remove");
                }
            });
            ui.separator();
            ui.label(RichText::new(t("log")).strong());
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .auto_shrink(false)
                .show(ui, |ui| {
                    for line in &b.log {
                        ui.monospace(line);
                    }
                });
        }
        match action {
            Some("run") => app.run_behavior(&ctx, index, false),
            Some("stop") => app.stop_behavior(index),
            Some("remove") => app.remove_behavior(&ctx, index),
            _ => {}
        }
    });
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
    let modal = egui::Modal::new(egui::Id::new("confirm_run")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.heading(&name);
        ui.label(lang.t("confirm_run"));
        ui.horizontal(|ui| {
            ui.label(lang.t("command"));
            ui.code(&command);
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(lang.t("run")).clicked() {
                choice = Some(true);
            }
            if ui.button(lang.t("cancel")).clicked() {
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
    let t = |k| lang.t(k);
    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("settings")
                .num_columns(2)
                .spacing([16.0, 10.0])
                .show(ui, |ui| {
                    ui.label(t("language"));
                    egui::ComboBox::from_id_salt("lang")
                        .selected_text(app.cfg.lang.native_name())
                        .show_ui(ui, |ui| {
                            for l in Lang::ALL {
                                ui.selectable_value(&mut app.cfg.lang, l, l.native_name());
                            }
                        });
                    ui.end_row();

                    ui.label("");
                    let mut autostart = app.autostart_enabled;
                    if ui.checkbox(&mut autostart, t("autostart")).changed() {
                        app.set_autostart(autostart);
                    }
                    ui.end_row();

                    ui.label("");
                    ui.checkbox(&mut app.cfg.close_to_tray, t("close_to_tray"));
                    ui.end_row();

                    ui.label(t("master_volume"));
                    ui.add(egui::Slider::new(&mut app.cfg.master_volume, 0.0..=1.0));
                    ui.end_row();

                    ui.label(t("ffmpeg"));
                    ui.horizontal(|ui| {
                        match (&app.cfg.ffmpeg, app.ffmpeg()) {
                            (Some(p), _) => ui.weak(p.display().to_string()),
                            (None, Some(p)) => {
                                ui.weak(format!("{} ({})", t("ffmpeg_auto"), p.display()))
                            }
                            (None, None) => {
                                ui.colored_label(ui.visuals().warn_fg_color, t("ffmpeg_missing"))
                            }
                        };
                        if ui.small_button(t("browse")).clicked()
                            && let Some(p) = rfd::FileDialog::new().pick_file()
                        {
                            app.cfg.ffmpeg = Some(p);
                        }
                        if app.cfg.ffmpeg.is_some() && ui.small_button(t("clear")).clicked() {
                            app.cfg.ffmpeg = None;
                        }
                    });
                    ui.end_row();

                    ui.label(t("ai_model"));
                    ui.vertical(|ui| {
                        if cfg!(feature = "ai") {
                            ui.horizontal(|ui| {
                                match &app.cfg.ai_model {
                                    Some(p) => ui.weak(p.display().to_string()),
                                    None => ui.weak(t("none")),
                                };
                                if ui.small_button(t("browse")).clicked()
                                    && let Some(p) = rfd::FileDialog::new()
                                        .add_filter("ONNX", &["onnx"])
                                        .pick_file()
                                {
                                    app.cfg.ai_model = Some(p);
                                }
                            });
                            ui.weak(t("ai_model_hint"));
                        } else {
                            ui.weak(t("ai_unavailable"));
                        }
                    });
                    ui.end_row();

                    ui.label("");
                    if ui.button(t("open_data_dir")).clicked() {
                        open_in_file_manager(app.dirs.library.parent().unwrap_or(&app.dirs.library));
                    }
                    ui.end_row();
                });

            ui.add_space(16.0);
            ui.separator();
            ui.heading(t("about"));
            ui.label(format!("Open Anima {}", env!("CARGO_PKG_VERSION")));
            ui.label(t("license_text"));
            ui.hyperlink(env!("CARGO_PKG_REPOSITORY"));
        });
    });
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
