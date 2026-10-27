// SPDX-License-Identifier: AGPL-3.0-or-later
//! Notification area icon and the application icon.

use crossbeam_channel::{Receiver, unbounded};
use eframe::egui;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::i18n::Lang;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayMsg {
    OpenPanel,
    ToggleHidden,
    Quit,
}

pub struct Tray {
    _icon: TrayIcon,
    hide_all: CheckMenuItem,
    pub messages: Receiver<TrayMsg>,
}

impl Tray {
    pub fn new(ctx: &egui::Context, lang: Lang, hidden: bool) -> anyhow::Result<Self> {
        let open = MenuItem::new(lang.t("open_panel"), true, None);
        let hide_all = CheckMenuItem::new(lang.t("hide_all"), true, hidden, None);
        let quit = MenuItem::new(lang.t("quit"), true, None);
        let menu = Menu::new();
        menu.append_items(&[&open, &hide_all, &PredefinedMenuItem::separator(), &quit])?;

        let (w, h, rgba) = icon_rgba(32);
        let icon = TrayIconBuilder::new()
            .with_tooltip("Open Anima")
            .with_icon(tray_icon::Icon::from_rgba(rgba, w, h)?)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()?;

        let (tx, rx) = unbounded();
        let (open_id, hide_id, quit_id) = (open.id().clone(), hide_all.id().clone(), quit.id().clone());
        {
            let tx = tx.clone();
            let ctx = ctx.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                let msg = if event.id == open_id {
                    TrayMsg::OpenPanel
                } else if event.id == hide_id {
                    TrayMsg::ToggleHidden
                } else if event.id == quit_id {
                    TrayMsg::Quit
                } else {
                    return;
                };
                let _ = tx.send(msg);
                ctx.request_repaint();
            }));
        }
        {
            let ctx = ctx.clone();
            TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    let _ = tx.send(TrayMsg::OpenPanel);
                    ctx.request_repaint();
                }
            }));
        }
        Ok(Self {
            _icon: icon,
            hide_all,
            messages: rx,
        })
    }

    pub fn set_hidden(&self, hidden: bool) {
        self.hide_all.set_checked(hidden);
    }
}

/// Draws the logo: a violet disc with a play triangle, so no binary asset is needed.
pub fn icon_rgba(size: u32) -> (u32, u32, Vec<u8>) {
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    let s = size as f32;
    let c = s / 2.0;
    for y in 0..size {
        for x in 0..size {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let d = ((px - c).powi(2) + (py - c).powi(2)).sqrt();
            let disc = (c - d).clamp(0.0, 1.0);
            // Triangle pointing right, centred slightly to the right.
            let (tx, ty) = ((px - s * 0.38) / (s * 0.34), (py - c) / (s * 0.30));
            let in_tri = tx >= 0.0 && tx <= 1.0 && ty.abs() <= 1.0 - tx;
            let (r, g, b) = if in_tri { (255, 255, 255) } else { (124, 77, 255) };
            rgba.extend_from_slice(&[r, g, b, (disc * 255.0) as u8]);
        }
    }
    (size, size, rgba)
}

pub fn window_icon() -> egui::IconData {
    let (width, height, rgba) = icon_rgba(64);
    egui::IconData {
        rgba,
        width,
        height,
    }
}
