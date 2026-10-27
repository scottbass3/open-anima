// SPDX-License-Identifier: AGPL-3.0-or-later
//! Application state and the glue between the library, the overlays and the behaviors.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui::{self, Pos2, Vec2, ViewportCommand};

use crate::audio::{Audio, Track};
use crate::behavior::{self, Anchor, Command, Package, Running};
use crate::config::{Config, DecodeOptions, Dirs, Look, Playback, WindowOptions};
use crate::i18n::Lang;
use crate::jobs::{Job, JobDone};
use crate::media::{self, Anim, LoadRequest, LoadResult, decode::MediaKind};
use crate::overlay::{self, OverlayEvent, OverlayKey, Overlays, Shared, SharedRef};
use crate::tray::{Tray, TrayMsg};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Library,
    Behaviors,
    Settings,
}

pub enum ItemStatus<'a> {
    Loading,
    Failed(&'a str),
    Ready(Arc<Mutex<Anim>>),
}

/// Runtime state of a library item.
struct ItemRuntime {
    loaded: (PathBuf, DecodeOptions),
    loading: Option<Receiver<LoadResult>>,
    error: Option<String>,
    anim: Option<Arc<Mutex<Anim>>>,
    overlay: Option<SharedRef>,
    spawn_pos: Pos2,
    track: Track,
}

enum Cached {
    Loading(Receiver<LoadResult>),
    Ready(Arc<Mutex<Anim>>),
    Failed,
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct AnimKey {
    path: PathBuf,
    flip_h: bool,
}

/// An animation slot of a running behavior.
struct Slot {
    key: AnimKey,
    look: Look,
    window: WindowOptions,
    /// Requested position; applied once the animation is loaded.
    target: Option<(Pos2, Anchor)>,
    restart: bool,
    last_pos: Option<Pos2>,
    overlay: Option<SharedRef>,
    spawn_pos: Pos2,
}

pub struct Behavior {
    pub package: Package,
    /// Lives outside the packages directory (added with "Load folder").
    pub external: bool,
    pub running: Option<Running>,
    pub log: VecDeque<String>,
    pub preview: Option<egui::TextureHandle>,
    slots: HashMap<String, Slot>,
    cache: HashMap<AnimKey, Cached>,
    sounds: Vec<rodio::Player>,
}

impl Behavior {
    fn new(package: Package, external: bool, ctx: &egui::Context) -> Self {
        let preview = package.preview_path().and_then(|p| {
            let frames = media::decode::decode_image(&p).ok()?;
            Some(ctx.load_texture(
                format!("preview:{}", package.id()),
                media::to_color_image(&frames[0].image),
                egui::TextureOptions::LINEAR,
            ))
        });
        Self {
            package,
            external,
            running: None,
            log: VecDeque::new(),
            preview,
            slots: HashMap::new(),
            cache: HashMap::new(),
            sounds: Vec::new(),
        }
    }

    pub fn log(&mut self, line: impl Into<String>) {
        if self.log.len() >= 300 {
            self.log.pop_front();
        }
        self.log.push_back(line.into());
    }

    fn stop(&mut self) {
        self.running = None;
        self.slots.clear();
        self.sounds.clear();
    }
}

pub struct Notice {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

pub struct App {
    pub dirs: Dirs,
    pub cfg: Config,
    saved: Config,
    last_save: Instant,
    audio: Audio,
    overlays: Overlays,
    items: HashMap<u64, ItemRuntime>,
    pub behaviors: Vec<Behavior>,
    events_tx: Sender<OverlayEvent>,
    events_rx: Receiver<OverlayEvent>,
    tray: Option<Tray>,
    tray_lang: Lang,
    pub jobs: Vec<Job>,
    autostart: Option<auto_launch::AutoLaunch>,
    pub autostart_enabled: bool,
    pub panel_visible: bool,
    quitting: bool,

    // UI state.
    pub tab: Tab,
    pub selected: Option<u64>,
    pub selected_behavior: Option<usize>,
    pub confirm_run: Option<usize>,
    pub notice: Option<Notice>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, dirs: Dirs, start_hidden: bool) -> Self {
        let ctx = &cc.egui_ctx;
        let cfg = Config::load(&dirs.config_file);
        let tray = Tray::new(ctx, cfg.lang, cfg.overlays_hidden)
            .map_err(|err| log::error!("tray icon: {err:#}"))
            .ok();
        let autostart = std::env::current_exe().ok().and_then(|exe| {
            auto_launch::AutoLaunchBuilder::new()
                .set_app_name("Open Anima")
                .set_app_path(&exe.to_string_lossy())
                .set_args(&["--minimized"])
                .build()
                .ok()
        });
        let autostart_enabled = autostart
            .as_ref()
            .and_then(|a| a.is_enabled().ok())
            .unwrap_or(false);
        let (events_tx, events_rx) = unbounded();
        let panel_visible = !(start_hidden && tray.is_some());
        if !panel_visible {
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }
        let mut app = Self {
            tray_lang: cfg.lang,
            saved: cfg.clone(),
            cfg,
            dirs,
            last_save: Instant::now(),
            audio: Audio::new(),
            overlays: Overlays::new(),
            items: HashMap::new(),
            behaviors: Vec::new(),
            events_tx,
            events_rx,
            tray,
            jobs: Vec::new(),
            autostart,
            autostart_enabled,
            panel_visible,
            quitting: false,
            tab: Tab::Library,
            selected: None,
            selected_behavior: None,
            confirm_run: None,
            notice: None,
        };
        app.selected = app.cfg.items.first().map(|i| i.id);
        app.reload_behaviors(ctx);
        for index in 0..app.behaviors.len() {
            let package = &app.behaviors[index].package;
            if app.cfg.active_behaviors.contains(&package.dir)
                && app.cfg.trusted.contains(&package.trust_token())
            {
                app.run_behavior(ctx, index, false);
            }
        }
        app
    }

    pub fn lang(&self) -> Lang {
        self.cfg.lang
    }

    pub fn t(&self, key: &'static str) -> &'static str {
        self.cfg.lang.t(key)
    }

    pub fn notify(&mut self, text: impl Into<String>, error: bool) {
        let text = text.into();
        if error {
            log::warn!("{text}");
        }
        self.notice = Some(Notice {
            text,
            error,
            at: Instant::now(),
        });
    }

    pub fn ffmpeg(&self) -> Option<PathBuf> {
        media::video::find_ffmpeg(self.cfg.ffmpeg.as_deref())
    }

    pub fn item_status(&self, id: u64) -> ItemStatus<'_> {
        match self.items.get(&id) {
            Some(ItemRuntime { anim: Some(a), .. }) => ItemStatus::Ready(a.clone()),
            Some(ItemRuntime { error: Some(e), .. }) => ItemStatus::Failed(e),
            _ => ItemStatus::Loading,
        }
    }

    /// Moves the overlay of an item after the user typed new coordinates.
    pub fn move_item(&mut self, id: u64, pos: Pos2) {
        if let Some(item) = self.cfg.item_mut(id) {
            item.pos = [pos.x, pos.y];
        }
        if let Some(overlay) = self.items.get(&id).and_then(|rt| rt.overlay.as_ref()) {
            overlay.lock().unwrap().move_to = Some(pos);
        }
    }

    /// Size of the monitor of the panel, in physical pixels.
    pub fn screen_size(&self, ctx: &egui::Context) -> Vec2 {
        ctx.input(|i| {
            let ppp = i.viewport().native_pixels_per_point.unwrap_or(1.0);
            i.viewport().monitor_size.map(|s| s * ppp)
        })
        .unwrap_or(Vec2::new(1920.0, 1080.0))
    }

    pub fn add_files(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        for path in paths {
            match media::decode::kind_of(&path) {
                MediaKind::Image | MediaKind::Video => {
                    let id = self.cfg.add_item(path);
                    self.selected = Some(id);
                    self.tab = Tab::Library;
                }
                MediaKind::Audio => {
                    // Attach dropped sounds to the selected animation.
                    match self.selected.and_then(|id| self.cfg.item_mut(id)) {
                        Some(item) => item.audio.file = Some(path),
                        None => self.notify(self.t("select_item"), true),
                    }
                }
                MediaKind::Package => self.install_package(ctx, &path),
                MediaKind::Unknown if path.is_dir() => self.add_behavior_dir(ctx, path),
                MediaKind::Unknown => {
                    let msg = format!("{}: {}", self.t("error"), path.display());
                    self.notify(msg, true);
                }
            }
        }
    }

    pub fn remove_item(&mut self, id: u64) {
        self.cfg.items.retain(|i| i.id != id);
        self.items.remove(&id);
        if self.selected == Some(id) {
            self.selected = self.cfg.items.first().map(|i| i.id);
        }
    }

    pub fn duplicate_item(&mut self, id: u64) {
        let Some(mut copy) = self.cfg.item(id).cloned() else {
            return;
        };
        copy.id = self.cfg.next_id;
        self.cfg.next_id += 1;
        copy.pos = [copy.pos[0] + 40.0, copy.pos[1] + 40.0];
        self.selected = Some(copy.id);
        self.cfg.items.push(copy);
    }

    pub fn set_overlays_hidden(&mut self, hidden: bool) {
        self.cfg.overlays_hidden = hidden;
        if let Some(tray) = &self.tray {
            tray.set_hidden(hidden);
        }
    }

    pub fn set_panel_visible(&mut self, ctx: &egui::Context, visible: bool) {
        self.panel_visible = visible;
        ctx.send_viewport_cmd(ViewportCommand::Visible(visible));
        if visible {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }
    }

    pub fn quit(&mut self, ctx: &egui::Context) {
        self.quitting = true;
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }

    pub fn set_autostart(&mut self, enabled: bool) {
        let Some(auto) = &self.autostart else { return };
        let result = if enabled { auto.enable() } else { auto.disable() };
        match result {
            Ok(()) => self.autostart_enabled = enabled,
            Err(err) => self.notify(format!("{}: {err}", self.t("error")), true),
        }
    }

    pub fn save(&mut self) {
        match self.cfg.save(&self.dirs.config_file) {
            Ok(()) => self.saved = self.cfg.clone(),
            Err(err) => log::error!("saving config: {err:#}"),
        }
        self.last_save = Instant::now();
    }

    // ---------------------------------------------------------------------------------------
    // Jobs

    pub fn start_export(&mut self, ctx: &egui::Context, id: u64, out: PathBuf) {
        let Some(item) = self.cfg.item(id) else { return };
        let label = format!("{} — {}", item.name, self.t("export_apng"));
        let job = Job::export(ctx, label, item, self.ffmpeg(), out);
        self.jobs.push(job);
    }

    #[cfg(feature = "ai")]
    pub fn start_ai(&mut self, ctx: &egui::Context, id: u64) {
        let Some(model) = self.cfg.ai_model.clone() else {
            self.notify(self.t("ai_need_model"), true);
            return;
        };
        let Some(item) = self.cfg.item(id) else { return };
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let safe: String = item
            .name
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
            .collect();
        let out = self.dirs.library.join(format!("{safe}-nobg-{stamp}.png"));
        let label = format!("{} — {}", item.name, self.t("ai_running"));
        let job = Job::remove_background(ctx, label, item, self.ffmpeg(), model, out);
        self.jobs.push(job);
    }

    fn poll_jobs(&mut self) {
        let mut finished = Vec::new();
        self.jobs.retain(|job| match job.poll() {
            Some(result) => {
                finished.push(result);
                false
            }
            None => true,
        });
        for result in finished {
            match result {
                Ok(JobDone::Exported(path)) => {
                    self.notify(format!("{} — {}", self.t("done"), path.display()), false)
                }
                Ok(JobDone::NewItem { from, path }) => {
                    let Some(original) = self.cfg.item(from).cloned() else {
                        continue;
                    };
                    let id = self.cfg.next_id;
                    self.cfg.next_id += 1;
                    let mut copy = original;
                    copy.id = id;
                    copy.source = path;
                    // The edits are baked into the new file.
                    copy.decode = DecodeOptions::default();
                    copy.name = format!("{} ✨", copy.name);
                    self.cfg.items.push(copy);
                    if let Some(original) = self.cfg.item_mut(from) {
                        original.enabled = false;
                    }
                    self.selected = Some(id);
                    self.notify(self.t("done"), false);
                }
                Err(err) => self.notify(format!("{}: {err}", self.t("error")), true),
            }
        }
    }

    // ---------------------------------------------------------------------------------------
    // Library items → overlays

    fn sync_items(&mut self, ctx: &egui::Context) {
        let ffmpeg = self.cfg.ffmpeg.clone();
        let master = self.cfg.master_volume;
        let hidden = self.cfg.overlays_hidden;
        self.items
            .retain(|id, _| self.cfg.items.iter().any(|i| i.id == *id));

        for item in &mut self.cfg.items {
            // Disabled items are only loaded while selected, for the preview.
            let wanted = if item.enabled || self.selected == Some(item.id) {
                (item.source.clone(), item.decode.clone())
            } else {
                (PathBuf::new(), DecodeOptions::default())
            };
            let rt = self.items.entry(item.id).or_insert_with(|| ItemRuntime {
                loaded: (PathBuf::new(), DecodeOptions::default()),
                loading: None,
                error: None,
                anim: None,
                overlay: None,
                spawn_pos: Pos2::ZERO,
                track: Track::default(),
            });
            if rt.loaded != wanted {
                rt.loaded = wanted;
                rt.anim = None;
                rt.overlay = None;
                rt.error = None;
                rt.loading = None;
            }
            if rt.anim.is_none()
                && rt.loading.is_none()
                && rt.error.is_none()
                && !rt.loaded.0.as_os_str().is_empty()
            {
                rt.loading = Some(media::spawn_load(
                    ctx.clone(),
                    LoadRequest {
                        name: format!("item{}", item.id),
                        source: item.source.clone(),
                        decode: item.decode.clone(),
                        ffmpeg: ffmpeg.clone(),
                        cache: self.dirs.cache.clone(),
                    },
                ));
            }
            if let Some(rx) = &rt.loading
                && let Ok(result) = rx.try_recv()
            {
                rt.loading = None;
                match result {
                    Ok(anim) => rt.anim = Some(Arc::new(Mutex::new(anim))),
                    Err(err) => rt.error = Some(err),
                }
            }

            let visible = item.enabled && !hidden && rt.anim.is_some();
            if !visible {
                rt.overlay = None;
            } else if let Some(anim) = &rt.anim {
                let overlay = rt.overlay.get_or_insert_with(|| {
                    rt.spawn_pos = Pos2::new(item.pos[0], item.pos[1]);
                    Arc::new(Mutex::new(Shared {
                        key: OverlayKey::Item(item.id),
                        anim: anim.clone(),
                        look: item.look.clone(),
                        playback: item.playback.clone(),
                        window: item.window.clone(),
                        move_to: None,
                        reported_pos: None,
                        events: self.events_tx.clone(),
                        ctx: ctx.clone(),
                    }))
                });
                {
                    let mut s = overlay.lock().unwrap();
                    s.look = item.look.clone();
                    s.playback = item.playback.clone();
                    s.window = item.window.clone();
                    if s.move_to.is_none()
                        && let Some(p) = s.reported_pos
                    {
                        item.pos = [p.x.round(), p.y.round()];
                    }
                }
                self.overlays.show(ctx, overlay, rt.spawn_pos);
            }

            let sound = if visible {
                item.audio.file.clone().or_else(|| {
                    let anim = rt.anim.as_ref()?.lock().unwrap();
                    item.audio
                        .video_sound
                        .then(|| anim.audio_track.clone())
                        .flatten()
                })
            } else {
                None
            };
            rt.track.sync(
                &self.audio,
                sound.as_deref(),
                item.audio.volume * master,
                item.playback.paused,
            );
        }
    }

    // ---------------------------------------------------------------------------------------
    // Behaviors

    pub fn reload_behaviors(&mut self, ctx: &egui::Context) {
        let mut found: Vec<(PathBuf, bool)> = std::fs::read_dir(&self.dirs.packages)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .map(|p| (p, false))
            .collect();
        found.extend(self.cfg.behavior_dirs.iter().map(|p| (p.clone(), true)));

        let mut old: Vec<Behavior> = std::mem::take(&mut self.behaviors);
        for (dir, external) in found {
            // A package may contain a single folder with the actual content.
            let dir = if dir.join(behavior::MANIFEST).is_file() || external {
                dir
            } else {
                single_child_dir(&dir).unwrap_or(dir)
            };
            if let Some(i) = old.iter().position(|b| b.package.dir == dir) {
                self.behaviors.push(old.swap_remove(i));
                continue;
            }
            match Package::open(&dir) {
                Ok(package) => self.behaviors.push(Behavior::new(package, external, ctx)),
                Err(err) => log::warn!("ignoring behavior {}: {err:#}", dir.display()),
            }
        }
        self.behaviors
            .sort_by(|a, b| a.package.manifest.name.cmp(&b.package.manifest.name));
        if self
            .selected_behavior
            .is_some_and(|i| i >= self.behaviors.len())
        {
            self.selected_behavior = None;
        }
    }

    pub fn install_package(&mut self, ctx: &egui::Context, archive: &Path) {
        match behavior::install(archive, &self.dirs.packages) {
            Ok(package) => {
                let dir = package.dir.clone();
                self.reload_behaviors(ctx);
                self.selected_behavior = self.behaviors.iter().position(|b| b.package.dir == dir);
                self.tab = Tab::Behaviors;
            }
            Err(err) => self.notify(format!("{}: {err:#}", self.t("error")), true),
        }
    }

    pub fn add_behavior_dir(&mut self, ctx: &egui::Context, dir: PathBuf) {
        if let Err(err) = Package::open(&dir) {
            self.notify(format!("{}: {err:#}", self.t("error")), true);
            return;
        }
        if !self.cfg.behavior_dirs.contains(&dir) {
            self.cfg.behavior_dirs.push(dir.clone());
        }
        self.reload_behaviors(ctx);
        self.selected_behavior = self.behaviors.iter().position(|b| b.package.dir == dir);
        self.tab = Tab::Behaviors;
    }

    pub fn remove_behavior(&mut self, ctx: &egui::Context, index: usize) {
        let Some(b) = self.behaviors.get_mut(index) else { return };
        b.stop();
        let dir = b.package.dir.clone();
        self.cfg.active_behaviors.retain(|d| *d != dir);
        if b.external {
            self.cfg.behavior_dirs.retain(|d| *d != dir);
        } else if let Some(top) = top_level_in(&self.dirs.packages, &dir) {
            if let Err(err) = std::fs::remove_dir_all(&top) {
                self.notify(format!("{}: {err}", self.t("error")), true);
            }
        }
        self.reload_behaviors(ctx);
    }

    /// Asks for confirmation the first time, then starts the behavior.
    pub fn run_behavior(&mut self, ctx: &egui::Context, index: usize, confirmed: bool) {
        let Some(b) = self.behaviors.get(index) else { return };
        let token = b.package.trust_token();
        if !confirmed && !self.cfg.trusted.contains(&token) {
            self.confirm_run = Some(index);
            return;
        }
        if !self.cfg.trusted.contains(&token) {
            self.cfg.trusted.push(token);
        }
        let b = &mut self.behaviors[index];
        b.stop();
        match Running::start(&b.package, ctx) {
            Ok(running) => {
                if !self.cfg.active_behaviors.contains(&b.package.dir) {
                    self.cfg.active_behaviors.push(b.package.dir.clone());
                }
                b.running = Some(running);
                b.log(format!("▶ {}", b.package.manifest.command.join(" ")));
                preload(b, ctx, &self.cfg.ffmpeg, &self.dirs.cache);
            }
            Err(err) => b.log(format!("⚠ {err:#}")),
        }
    }

    pub fn stop_behavior(&mut self, index: usize) {
        if let Some(b) = self.behaviors.get_mut(index) {
            if b.running.is_some() {
                b.log("■");
            }
            b.stop();
            self.cfg.active_behaviors.retain(|d| *d != b.package.dir);
        }
    }

    fn sync_behaviors(&mut self, ctx: &egui::Context) {
        let screen = self.screen_size(ctx);
        let master = self.cfg.master_volume;
        let ffmpeg = self.cfg.ffmpeg.clone();
        let cache_dir = self.dirs.cache.clone();
        let hidden = self.cfg.overlays_hidden;

        for b in &mut self.behaviors {
            // Commands from the process.
            let mut commands = Vec::new();
            let mut exited = false;
            if let Some(running) = &mut b.running {
                commands.extend(running.commands.try_iter());
                if let Some(status) = running.exited() {
                    commands.extend(running.commands.try_iter());
                    commands.push(Command::Log(format!("■ {status}")));
                    exited = true;
                }
            }
            for cmd in commands {
                if let Some(running) = &b.running {
                    running.live.lock().unwrap().applied(&cmd);
                }
                apply_command(b, cmd, master, &self.audio, ctx, &ffmpeg, &cache_dir);
            }
            if exited {
                b.stop();
            }
            b.sounds.retain(|p| !p.empty());

            // Finished loads.
            for cached in b.cache.values_mut() {
                if let Cached::Loading(rx) = cached
                    && let Ok(result) = rx.try_recv()
                {
                    *cached = match result {
                        Ok(anim) => Cached::Ready(Arc::new(Mutex::new(anim))),
                        Err(_) => Cached::Failed,
                    };
                }
            }

            // Slots → overlays.
            let package_id = b.package.id();
            let mut failed = Vec::new();
            for (name, slot) in &mut b.slots {
                let anim = match b.cache.get(&slot.key) {
                    Some(Cached::Ready(anim)) => anim.clone(),
                    Some(Cached::Failed) | None => {
                        failed.push(name.clone());
                        continue;
                    }
                    Some(Cached::Loading(_)) => continue,
                };
                if slot.restart {
                    anim.lock().unwrap().restart();
                    slot.restart = false;
                }
                let size = overlay::window_size(anim.lock().unwrap().size, &slot.look);
                let target = slot.target.take().map(|(p, anchor)| match anchor {
                    Anchor::Center => p - size / 2.0,
                    Anchor::TopLeft => p,
                });
                if let Some(overlay) = &slot.overlay {
                    let mut s = overlay.lock().unwrap();
                    if !Arc::ptr_eq(&s.anim, &anim) {
                        s.anim = anim.clone();
                    }
                    s.look = slot.look.clone();
                    s.window = slot.window.clone();
                    if let Some(p) = target {
                        s.move_to = Some(p);
                        slot.last_pos = Some(p);
                    } else if let Some(p) = s.reported_pos {
                        slot.last_pos = Some(p);
                    }
                } else {
                    let pos = target
                        .or(slot.last_pos)
                        .unwrap_or_else(|| (screen / 2.0 - size / 2.0).to_pos2());
                    slot.spawn_pos = pos;
                    slot.last_pos = Some(pos);
                    slot.overlay = Some(Arc::new(Mutex::new(Shared {
                        key: OverlayKey::Behavior {
                            package: package_id.clone(),
                            slot: name.clone(),
                        },
                        anim,
                        look: slot.look.clone(),
                        playback: Playback::default(),
                        window: slot.window.clone(),
                        move_to: None,
                        reported_pos: None,
                        events: self.events_tx.clone(),
                        ctx: ctx.clone(),
                    })));
                }
                if let Some(overlay) = &slot.overlay {
                    if let Some(running) = &b.running {
                        running.live.lock().unwrap().publish(name, overlay, size);
                    }
                    if !hidden {
                        self.overlays.show(ctx, overlay, slot.spawn_pos);
                    }
                }
            }
            for name in failed {
                if let Some(running) = &b.running {
                    running.live.lock().unwrap().unpublish(&name);
                }
                if let Some(slot) = b.slots.remove(&name) {
                    b.log(format!("⚠ cannot load {}", slot.key.path.display()));
                }
            }
        }
    }

    fn handle_events(&mut self, ctx: &egui::Context) {
        let tray_msgs: Vec<TrayMsg> = self
            .tray
            .as_ref()
            .map(|t| t.messages.try_iter().collect())
            .unwrap_or_default();
        for msg in tray_msgs {
            match msg {
                TrayMsg::OpenPanel => self.set_panel_visible(ctx, true),
                TrayMsg::ToggleHidden => self.set_overlays_hidden(!self.cfg.overlays_hidden),
                TrayMsg::Quit => self.quit(ctx),
            }
        }

        let events: Vec<OverlayEvent> = self.events_rx.try_iter().collect();
        for event in events {
            match event {
                OverlayEvent::Activated(OverlayKey::Item(id)) => {
                    self.tab = Tab::Library;
                    self.selected = Some(id);
                    self.set_panel_visible(ctx, true);
                }
                OverlayEvent::Activated(OverlayKey::Behavior { package, .. }) => {
                    self.tab = Tab::Behaviors;
                    self.selected_behavior =
                        self.behaviors.iter().position(|b| b.package.id() == package);
                    self.set_panel_visible(ctx, true);
                }
                OverlayEvent::Closed(OverlayKey::Item(id)) => {
                    if let Some(item) = self.cfg.item_mut(id) {
                        item.enabled = false;
                    }
                }
                OverlayEvent::Closed(OverlayKey::Behavior { package, slot }) => {
                    if let Some(b) = self.behaviors.iter_mut().find(|b| b.package.id() == package) {
                        b.slots.remove(&slot);
                    }
                }
            }
        }

        if ctx.input(|i| i.viewport().close_requested())
            && !self.quitting
            && self.cfg.close_to_tray
            && self.tray.is_some()
        {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.set_panel_visible(ctx, false);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_command(
    b: &mut Behavior,
    cmd: Command,
    master: f32,
    audio: &Audio,
    ctx: &egui::Context,
    ffmpeg: &Option<PathBuf>,
    cache_dir: &Path,
) {
    match cmd {
        Command::Show {
            slot,
            file,
            x,
            y,
            anchor,
            scale,
            flip_h,
            click_through,
            draggable,
        } => {
            let key = AnimKey {
                path: b.package.dir.join(file),
                flip_h,
            };
            ensure_loaded(b, &key, ctx, ffmpeg, cache_dir);
            let target = x.zip(y).map(|(x, y)| (Pos2::new(x, y), anchor));
            let look = Look {
                scale: scale.unwrap_or(1.0),
                opacity: 1.0,
            };
            let window = WindowOptions {
                draggable,
                click_through,
                always_on_top: true,
            };
            let entry = b.slots.entry(slot).or_insert_with(|| Slot {
                key: key.clone(),
                look: look.clone(),
                window: window.clone(),
                target: None,
                restart: true,
                last_pos: None,
                overlay: None,
                spawn_pos: Pos2::ZERO,
            });
            entry.key = key;
            entry.look = look;
            entry.window = window;
            entry.restart = true;
            if target.is_some() {
                entry.target = target;
            }
        }
        Command::Move { slot, x, y, anchor } => {
            if let Some(s) = b.slots.get_mut(&slot) {
                s.target = Some((Pos2::new(x, y), anchor));
            }
        }
        Command::Hide { slot } => {
            b.slots.remove(&slot);
        }
        Command::Clear => b.slots.clear(),
        Command::Sound { file, volume } => {
            let path = b.package.dir.join(file);
            if let Some(player) = audio.play(&path, volume * master, false) {
                b.sounds.push(player);
            }
        }
        Command::Log(line) => b.log(line),
    }
}

fn ensure_loaded(
    b: &mut Behavior,
    key: &AnimKey,
    ctx: &egui::Context,
    ffmpeg: &Option<PathBuf>,
    cache_dir: &Path,
) {
    if b.cache.contains_key(key) {
        return;
    }
    let rx = media::spawn_load(
        ctx.clone(),
        LoadRequest {
            name: format!("{}:{}", b.package.id(), key.path.display()),
            source: key.path.clone(),
            decode: DecodeOptions {
                flip_h: key.flip_h,
                ..Default::default()
            },
            ffmpeg: ffmpeg.clone(),
            cache: cache_dir.to_path_buf(),
        },
    );
    b.cache.insert(key.clone(), Cached::Loading(rx));
}

/// Decodes the images shipped with a behavior right away so the first `show` isn't late.
fn preload(b: &mut Behavior, ctx: &egui::Context, ffmpeg: &Option<PathBuf>, cache_dir: &Path) {
    let dirs = ["temp", "assets"].map(|d| b.package.dir.join(d));
    let files: Vec<PathBuf> = dirs
        .iter()
        .flat_map(|d| std::fs::read_dir(d).into_iter().flatten())
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| media::decode::kind_of(p) == MediaKind::Image)
        .take(64)
        .collect();
    for path in files {
        let key = AnimKey {
            path,
            flip_h: false,
        };
        ensure_loaded(b, &key, ctx, ffmpeg, cache_dir);
    }
}

fn single_child_dir(dir: &Path) -> Option<PathBuf> {
    let entries: Vec<_> = std::fs::read_dir(dir).ok()?.filter_map(|e| e.ok()).collect();
    match &entries[..] {
        [only] if only.path().is_dir() => Some(only.path()),
        _ => None,
    }
}

/// The direct child of `root` that contains `path`.
fn top_level_in(root: &Path, path: &Path) -> Option<PathBuf> {
    let rel = path.strip_prefix(root).ok()?;
    let first = rel.components().next()?;
    Some(root.join(first))
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Overlays must be see-through; the panel paints its own opaque background.
        [0.0; 4]
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_events(ctx);
        self.poll_jobs();
        self.sync_items(ctx);
        self.sync_behaviors(ctx);
        self.overlays.flush();

        if self.tray_lang != self.cfg.lang {
            self.tray_lang = self.cfg.lang;
            self.tray = Tray::new(ctx, self.cfg.lang, self.cfg.overlays_hidden).ok();
        }
        if self.cfg != self.saved && self.last_save.elapsed() > Duration::from_secs(1) {
            self.save();
        }
        // Housekeeping (positions, behavior commands) also has to happen while the panel is
        // hidden in the tray.
        ctx.request_repaint_after(Duration::from_millis(500));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        crate::ui::draw(self, ui);
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if self.cfg != self.saved {
            self.save();
        }
    }
}
