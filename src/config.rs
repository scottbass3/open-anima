// SPDX-License-Identifier: AGPL-3.0-or-later
//! Persistent configuration: the animation library and global settings.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

use crate::i18n::Lang;

/// Where Open Anima keeps its files.
pub struct Dirs {
    pub config_file: PathBuf,
    /// Files produced by the editor (AI background removal, exports).
    pub library: PathBuf,
    /// Extracted behavior packages.
    pub packages: PathBuf,
    /// Throw-away files (audio tracks extracted from videos, …).
    pub cache: PathBuf,
}

impl Dirs {
    pub fn new() -> anyhow::Result<Self> {
        let dirs = directories::ProjectDirs::from("org", "OpenAnima", "Open Anima")
            .context("no home directory")?;
        let data = dirs.data_dir().to_path_buf();
        let this = Self {
            config_file: dirs.config_dir().join("config.json"),
            library: data.join("library"),
            packages: data.join("packages"),
            cache: dirs.cache_dir().to_path_buf(),
        };
        for dir in [
            dirs.config_dir(),
            this.library.as_path(),
            this.packages.as_path(),
            this.cache.as_path(),
        ] {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        Ok(this)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub lang: Lang,
    /// Global master volume, multiplied with each item's volume.
    pub master_volume: f32,
    /// Hide every overlay without disabling them one by one.
    pub overlays_hidden: bool,
    /// Close button sends the panel to the tray instead of quitting.
    pub close_to_tray: bool,
    /// `ffmpeg` executable used for video files; `None` means look it up in `PATH`.
    pub ffmpeg: Option<PathBuf>,
    /// ONNX segmentation model used for AI background removal.
    pub ai_model: Option<PathBuf>,
    pub items: Vec<Item>,
    pub next_id: u64,
    /// Behavior folders used in place (development), besides the installed packages.
    pub behavior_dirs: Vec<PathBuf>,
    /// Behaviors the user agreed to run, see [`crate::behavior::Package::trust_token`].
    pub trusted: Vec<String>,
    /// Behaviors running when the app was closed, restarted on launch.
    pub active_behaviors: Vec<PathBuf>,
    /// Tab of the panel shown last.
    pub tab: crate::app::Tab,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            lang: Lang::detect(),
            master_volume: 1.0,
            overlays_hidden: false,
            close_to_tray: true,
            ffmpeg: None,
            ai_model: None,
            items: Vec::new(),
            next_id: 1,
            behavior_dirs: Vec::new(),
            trusted: Vec::new(),
            active_behaviors: Vec::new(),
            tab: Default::default(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|err| {
                log::error!("invalid config {}: {err}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn add_item(&mut self, source: PathBuf) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let mut item = Item::new(id, source);
        // Cascade new items so they don't all stack on top of each other.
        let offset = (self.items.len() % 8) as f32 * 40.0;
        item.pos = [200.0 + offset, 200.0 + offset];
        self.items.push(item);
        id
    }

    pub fn item(&self, id: u64) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn item_mut(&mut self, id: u64) -> Option<&mut Item> {
        self.items.iter_mut().find(|i| i.id == id)
    }
}

/// One animation placed on the desktop.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Item {
    pub id: u64,
    pub name: String,
    pub source: PathBuf,
    pub enabled: bool,
    /// Top-left corner of the overlay window, in screen points.
    pub pos: [f32; 2],
    pub decode: DecodeOptions,
    pub look: Look,
    pub playback: Playback,
    pub window: WindowOptions,
    pub audio: AudioOptions,
}

impl Default for Item {
    fn default() -> Self {
        Self::new(0, PathBuf::new())
    }
}

impl Item {
    pub fn new(id: u64, source: PathBuf) -> Self {
        let name = source
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self {
            id,
            name,
            source,
            enabled: true,
            pos: [200.0, 200.0],
            decode: DecodeOptions::default(),
            look: Look::default(),
            playback: Playback::default(),
            window: WindowOptions::default(),
            audio: AudioOptions::default(),
        }
    }
}

/// Everything that changes the decoded pixels; any change requires reloading the media.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct DecodeOptions {
    pub sprite_sheet: Option<SpriteSheet>,
    pub color_key: Option<ColorKey>,
    pub flip_h: bool,
    pub flip_v: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SpriteSheet {
    pub columns: u32,
    pub rows: u32,
    /// Number of used cells, read left to right then top to bottom. 0 = all.
    pub frames: u32,
    pub fps: f32,
}

impl Default for SpriteSheet {
    fn default() -> Self {
        Self {
            columns: 4,
            rows: 1,
            frames: 0,
            fps: 12.0,
        }
    }
}

/// Makes every pixel close to `color` transparent (green screen style).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ColorKey {
    pub color: [u8; 3],
    /// Distance (0‥1) under which pixels become fully transparent.
    pub tolerance: f32,
    /// Extra distance over which pixels fade back to opaque.
    pub softness: f32,
}

impl Default for ColorKey {
    fn default() -> Self {
        Self {
            color: [0, 255, 0],
            tolerance: 0.15,
            softness: 0.1,
        }
    }
}

/// Cheap visual settings applied at render time.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Look {
    pub scale: f32,
    pub opacity: f32,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            scale: 1.0,
            opacity: 1.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Playback {
    pub speed: f32,
    /// Replace the file's own frame timing with a constant frame rate.
    pub fps_override: Option<f32>,
    pub paused: bool,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            speed: 1.0,
            fps_override: None,
            paused: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WindowOptions {
    pub draggable: bool,
    /// Mouse clicks go through the animation to whatever is underneath.
    pub click_through: bool,
    pub always_on_top: bool,
}

impl Default for WindowOptions {
    fn default() -> Self {
        Self {
            draggable: true,
            click_through: false,
            always_on_top: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AudioOptions {
    /// Separate audio file played in a loop with the animation.
    pub file: Option<PathBuf>,
    /// Play the sound track of video files.
    pub video_sound: bool,
    pub volume: f32,
}

impl Default for AudioOptions {
    fn default() -> Self {
        Self {
            file: None,
            video_sound: true,
            volume: 0.8,
        }
    }
}
