// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Open Anima — put animations on your desktop.
// Copyright (C) 2026 Open Anima contributors
//
// This program is free software: you can redistribute it and/or modify it under the terms of
// the GNU Affero General Public License as published by the Free Software Foundation, either
// version 3 of the License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY;
// without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
// See the GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License along with this
// program. If not, see <https://www.gnu.org/licenses/>.

// No console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod behavior;
mod config;
mod i18n;
mod jobs;
mod media;
mod overlay;
mod process;
mod theme;
mod tray;
mod workshop;
mod ui;

use eframe::egui;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<String> = std::env::args().collect();
    let start_hidden = args.iter().any(|a| a == "--minimized");
    // `--workshop <link>` opens the Workshop import dialog on that item.
    let workshop_link = args
        .iter()
        .position(|a| a == "--workshop")
        .and_then(|i| args.get(i + 1).cloned());
    let dirs = config::Dirs::new().unwrap_or_else(|err| {
        eprintln!("Open Anima: {err:#}");
        std::process::exit(1);
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Open Anima")
            .with_app_id("open-anima")
            .with_inner_size([1180.0, 740.0])
            .with_min_inner_size([900.0, 560.0])
            // The panel draws its own title bar.
            .with_decorations(false)
            .with_icon(tray::window_icon())
            .with_drag_and_drop(true)
            // Outside Windows, overlays are egui viewports and the renderer only enables
            // transparent surfaces when the root viewport asks for it. The panel paints an
            // opaque background anyway.
            .with_transparent(!cfg!(windows)),
        ..Default::default()
    };
    eframe::run_native(
        "Open Anima",
        options,
        Box::new(move |cc| {
            let mut app = app::App::new(cc, dirs, start_hidden);
            if let Some(link) = workshop_link {
                app.workshop.link = link;
                app.workshop_lookup(&cc.egui_ctx);
            }
            Ok(Box::new(app))
        }),
    )
}
