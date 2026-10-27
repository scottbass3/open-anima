// SPDX-License-Identifier: AGPL-3.0-or-later
//! Borderless, transparent desktop windows that display one animation each.
//!
//! On Windows they are native layered windows driven by their own thread (see [`win`]): it is
//! the only reliable way to get per-pixel transparency there, and it lets clicks go through the
//! transparent parts of the animation. Elsewhere they are egui viewports.
//!
//! All positions and sizes in this module are in physical screen pixels.

#[cfg(windows)]
mod win;

use std::sync::{Arc, Mutex};

use crossbeam_channel::Sender;
use eframe::egui::{self, Pos2, Vec2};

use crate::config::{Look, Playback, WindowOptions};
use crate::media::Anim;

/// Identifies an overlay window.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum OverlayKey {
    /// An animation of the library.
    Item(u64),
    /// A slot driven by a running behavior package.
    Behavior { package: String, slot: String },
}

#[derive(Clone, Debug)]
pub enum OverlayEvent {
    /// The user double-clicked the overlay.
    Activated(OverlayKey),
    /// The user closed the window (Alt+F4…).
    Closed(OverlayKey),
}

/// State shared between the control panel and the overlay window.
pub struct Shared {
    pub key: OverlayKey,
    pub anim: Arc<Mutex<Anim>>,
    pub look: Look,
    pub playback: Playback,
    pub window: WindowOptions,
    /// Position requested by the app; applied on the next frame of the overlay.
    pub move_to: Option<Pos2>,
    /// Where the window currently is, as reported by the OS.
    pub reported_pos: Option<Pos2>,
    pub events: Sender<OverlayEvent>,
    /// Woken up when an event is sent.
    pub ctx: egui::Context,
}

impl Shared {
    pub fn send(&self, event: OverlayEvent) {
        let _ = self.events.send(event);
        self.ctx.request_repaint();
    }
}

pub type SharedRef = Arc<Mutex<Shared>>;

/// Size of the window for the given animation size.
pub fn window_size(anim_px: [u32; 2], look: &Look) -> Vec2 {
    let scale = look.scale.max(0.01);
    Vec2::new(
        (anim_px[0] as f32 * scale).round(),
        (anim_px[1] as f32 * scale).round(),
    )
    .max(Vec2::splat(1.0))
}

/// Applies pending [`Shared::move_to`] requests right away instead of at the next frame.
pub fn wake() {
    #[cfg(windows)]
    win::wake();
}

/// The set of overlays to display. Every pass of the app calls [`Overlays::show`] for each
/// overlay that should exist, then [`Overlays::flush`]; the others are closed.
pub struct Overlays {
    #[cfg(windows)]
    native: win::Native,
    #[cfg(windows)]
    pending: Vec<(SharedRef, Pos2)>,
}

impl Overlays {
    pub fn new() -> Self {
        Self {
            #[cfg(windows)]
            native: win::Native::new(),
            #[cfg(windows)]
            pending: Vec::new(),
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, shared: &SharedRef, spawn_pos: Pos2) {
        #[cfg(windows)]
        {
            let _ = ctx;
            self.pending.push((shared.clone(), spawn_pos));
        }
        #[cfg(not(windows))]
        viewport::show(ctx, shared, spawn_pos);
    }

    pub fn flush(&mut self) {
        #[cfg(windows)]
        self.native.sync(std::mem::take(&mut self.pending));
    }
}

#[cfg(not(windows))]
mod viewport {
    use eframe::egui::{
        self, Pos2, Rect, Sense, ViewportBuilder, ViewportCommand, ViewportId,
    };

    use super::*;

    pub fn show(ctx: &egui::Context, shared: &SharedRef, spawn_pos: Pos2) {
        let ppp = ctx.pixels_per_point();
        let (id, builder) = {
            let s = shared.lock().unwrap();
            let size = {
                let anim = s.anim.lock().unwrap();
                window_size(anim.size, &s.look) / ppp
            };
            let level = if s.window.always_on_top {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            };
            let builder = ViewportBuilder::default()
                .with_title("Open Anima overlay")
                .with_decorations(false)
                .with_transparent(true)
                .with_has_shadow(false)
                .with_resizable(false)
                .with_taskbar(false)
                .with_active(false)
                .with_inner_size(size)
                .with_position(spawn_pos / ppp)
                .with_window_level(level)
                .with_mouse_passthrough(s.window.click_through);
            (ViewportId::from_hash_of(&s.key), builder)
        };
        let shared = shared.clone();
        ctx.show_viewport_deferred(id, builder, move |ui, _class| overlay_ui(ui, &shared));
    }

    fn overlay_ui(ui: &mut egui::Ui, shared: &SharedRef) {
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let mut s = shared.lock().unwrap();

        if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
            s.reported_pos = Some((rect.min.to_vec2() * ppp).to_pos2());
        }
        if let Some(pos) = s.move_to.take() {
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(pos / ppp));
            s.reported_pos = Some(pos);
        }

        let frame = s.anim.lock().unwrap().frame(crate::media::clock(), &s.playback);
        let rect = ctx.content_rect();
        let tint = egui::Color32::WHITE.gamma_multiply(s.look.opacity.clamp(0.0, 1.0));
        ui.painter().image(
            frame.tex,
            rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            tint,
        );

        let response = ui.interact(rect, ui.id().with("overlay"), Sense::click_and_drag());
        if s.window.draggable && response.drag_started() {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if response.double_clicked() {
            s.send(OverlayEvent::Activated(s.key.clone()));
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            s.send(OverlayEvent::Closed(s.key.clone()));
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(frame.next));
    }
}
