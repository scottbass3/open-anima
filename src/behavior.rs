// SPDX-License-Identifier: AGPL-3.0-or-later
//! Behavior packages: external programs that drive animations on the desktop.
//!
//! A package is a folder (or a `.oapack` zip of that folder) with a `manifest.toml`, the program
//! and its assets. The program talks to Open Anima with one of two protocols:
//!
//! * **stdio** (recommended): one JSON command per line on its standard output, see
//!   [`Command`] and `docs/behaviors.md`.
//! * **files** (compatibility): the program writes `temp/actions.mddo` with a line such as
//!   `[1,'temp\\walk.gif','1-(100,200)']` and then `1` into `activador.act`. Open Anima reads
//!   the action and writes `0` back.

use std::io::{BufRead, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context as _, bail};
use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::process;

pub const MANIFEST: &str = "manifest.toml";
const LEGACY_ACTIONS: &str = "temp/actions.mddo";
const LEGACY_TRIGGER: &str = "activador.act";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Stdio,
    Files,
    /// Listen to both.
    #[default]
    Auto,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Manifest {
    pub name: String,
    pub author: String,
    pub version: String,
    pub description: String,
    /// Preview image, relative to the package folder.
    pub preview: Option<PathBuf>,
    /// Program and arguments. The program is resolved relative to the package folder first,
    /// then through `PATH` (so `["python", "main.py"]` works).
    pub command: Vec<String>,
    pub protocol: Protocol,
}

/// A behavior found on disk.
#[derive(Clone, Debug)]
pub struct Package {
    pub dir: PathBuf,
    pub manifest: Manifest,
    /// Whether the package has a real manifest or was guessed from the folder layout.
    pub has_manifest: bool,
}

impl Package {
    pub fn open(dir: &Path) -> anyhow::Result<Self> {
        let manifest_path = dir.join(MANIFEST);
        if manifest_path.is_file() {
            let text = std::fs::read_to_string(&manifest_path)?;
            let mut manifest: Manifest =
                toml::from_str(&text).with_context(|| format!("{}", manifest_path.display()))?;
            if manifest.name.is_empty() {
                manifest.name = folder_name(dir);
            }
            if manifest.command.is_empty() {
                bail!("{MANIFEST}: `command` is empty");
            }
            return Ok(Self {
                dir: dir.to_path_buf(),
                manifest,
                has_manifest: true,
            });
        }
        // Folder laid out like the packages of other desktop animation tools: an executable at
        // the root and the assets in `temp/`.
        let exe = std::fs::read_dir(dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| {
                p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
            })
            .context("no manifest.toml and no executable in this folder")?;
        let preview = ["preview.png", "preview.gif", "temp/preview.png"]
            .into_iter()
            .map(PathBuf::from)
            .find(|p| dir.join(p).is_file());
        Ok(Self {
            dir: dir.to_path_buf(),
            manifest: Manifest {
                name: folder_name(dir),
                command: vec![exe.file_name().unwrap().to_string_lossy().into_owned()],
                protocol: Protocol::Files,
                preview,
                ..Default::default()
            },
            has_manifest: false,
        })
    }

    /// Stable identifier, also used for the overlay keys.
    pub fn id(&self) -> String {
        self.dir.to_string_lossy().into_owned()
    }

    /// What the user approves before the first run: any change asks again.
    pub fn trust_token(&self) -> String {
        format!("{}|{}", self.id(), self.manifest.command.join("\u{1f}"))
    }

    pub fn preview_path(&self) -> Option<PathBuf> {
        self.manifest
            .preview
            .as_ref()
            .map(|p| self.dir.join(p))
            .filter(|p| p.is_file())
    }
}

fn folder_name(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "behavior".into())
}

/// Extracts a `.oapack` into `packages_dir` and returns the package.
pub fn install(archive: &Path, packages_dir: &Path) -> anyhow::Result<Package> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file).context("not a valid .oapack (zip) file")?;
    let stem = archive
        .file_stem()
        .map(|s| sanitize(&s.to_string_lossy()))
        .unwrap_or_else(|| "package".into());
    let mut dir = packages_dir.join(&stem);
    let mut n = 2;
    while dir.exists() {
        dir = packages_dir.join(format!("{stem}-{n}"));
        n += 1;
    }
    // `extract` refuses entries that would escape `dir`.
    zip.extract(&dir)?;
    // Archives created by zipping the folder itself have a single top-level directory.
    let root = single_subdir(&dir).unwrap_or_else(|| dir.clone());
    Package::open(&root)
}

fn single_subdir(dir: &Path) -> Option<PathBuf> {
    let entries: Vec<_> = std::fs::read_dir(dir).ok()?.filter_map(|e| e.ok()).collect();
    match &entries[..] {
        [only] if only.path().is_dir() && !dir.join(MANIFEST).exists() => Some(only.path()),
        _ => None,
    }
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || "-_ ".contains(c) { c } else { '_' })
        .collect()
}

/// Zips a behavior folder into a distributable `.oapack`, writing a manifest first if needed.
pub fn create_package(dir: &Path, out: &Path) -> anyhow::Result<()> {
    let package = Package::open(dir)?;
    if !package.has_manifest {
        std::fs::write(dir.join(MANIFEST), toml::to_string_pretty(&package.manifest)?)?;
    }
    let mut zip = zip::ZipWriter::new(std::fs::File::create(out)?);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current)? {
            let path = entry?.path();
            let rel = path.strip_prefix(dir)?.to_string_lossy().replace('\\', "/");
            if path.is_dir() {
                zip.add_directory(format!("{rel}/"), options)?;
                stack.push(path);
            } else if path != out {
                zip.start_file(rel, options)?;
                std::io::copy(&mut std::fs::File::open(&path)?, &mut zip)?;
            }
        }
    }
    zip.finish()?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Anchor {
    /// `x`/`y` is the center of the animation.
    #[default]
    Center,
    /// `x`/`y` is the top-left corner.
    TopLeft,
}

/// A command sent by a behavior. Coordinates are in physical screen pixels, as returned by the
/// operating system's APIs.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Command {
    /// Shows `file` in `slot` (restarting it if it was already shown there).
    Show {
        #[serde(default = "default_slot")]
        slot: String,
        file: PathBuf,
        x: Option<f32>,
        y: Option<f32>,
        #[serde(default)]
        anchor: Anchor,
        scale: Option<f32>,
        #[serde(default)]
        flip_h: bool,
        #[serde(default = "yes")]
        click_through: bool,
        #[serde(default)]
        draggable: bool,
    },
    Move {
        #[serde(default = "default_slot")]
        slot: String,
        x: f32,
        y: f32,
        #[serde(default)]
        anchor: Anchor,
    },
    Hide {
        #[serde(default = "default_slot")]
        slot: String,
    },
    /// Plays a sound once.
    Sound {
        file: PathBuf,
        #[serde(default = "one")]
        volume: f32,
    },
    /// Hides every slot of this behavior.
    Clear,
    #[serde(skip)]
    Log(String),
}

fn default_slot() -> String {
    "main".into()
}
fn yes() -> bool {
    true
}
fn one() -> f32 {
    1.0
}

/// Parses the legacy `actions.mddo` content, e.g. `[1,'temp\\a.gif','1-(10,20)']`.
pub fn parse_legacy(text: &str) -> Option<Command> {
    let inner = text.trim().strip_prefix('[')?.strip_suffix(']')?;
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for c in inner.chars() {
        match (quote, c) {
            (None, '\'' | '"') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, ',') => fields.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    fields.push(current);
    let [show, file, pos] = &fields[..] else {
        return None;
    };
    let file = file.trim().replace("\\\\", "\\");
    let file = if cfg!(windows) { file } else { file.replace('\\', "/") };
    let (active, coords) = pos.trim().split_once('-')?;
    let coords = coords.trim().strip_prefix('(')?.strip_suffix(')')?;
    let (x, y) = coords.split_once(',')?;
    let (x, y): (f32, f32) = (x.trim().parse().ok()?, y.trim().parse().ok()?);
    let positioned = active.trim() == "1";
    if show.trim() == "0" {
        return Some(Command::Hide { slot: default_slot() });
    }
    Some(Command::Show {
        slot: default_slot(),
        file: PathBuf::from(file),
        x: positioned.then_some(x),
        y: positioned.then_some(y),
        anchor: Anchor::Center,
        scale: None,
        flip_h: false,
        click_through: true,
        draggable: false,
    })
}

/// A running behavior process.
pub struct Running {
    child: Child,
    stop: Arc<AtomicBool>,
    pub commands: Receiver<Command>,
}

impl Running {
    pub fn start(package: &Package, ctx: &egui::Context) -> anyhow::Result<Self> {
        let [program, args @ ..] = &package.manifest.command[..] else {
            bail!("empty command");
        };
        let local = package.dir.join(program);
        let program = if local.is_file() { local } else { PathBuf::from(program) };
        let protocol = package.manifest.protocol;
        if protocol != Protocol::Stdio {
            // Start from a clean state so a stale trigger doesn't replay an old action.
            let _ = std::fs::write(package.dir.join(LEGACY_TRIGGER), "0");
        }
        let mut child = process::command(&program)
            .args(args)
            .current_dir(&package.dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("starting {}", program.display()))?;
        process::adopt(&child);

        let (tx, rx) = unbounded();
        let stop = Arc::new(AtomicBool::new(false));
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        spawn_reader(stdout, tx.clone(), ctx.clone(), protocol != Protocol::Files);
        spawn_reader(stderr, tx.clone(), ctx.clone(), false);
        if protocol != Protocol::Stdio {
            let dir = package.dir.clone();
            let stop = stop.clone();
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name("behavior-files".into())
                .spawn(move || watch_legacy_files(&dir, &tx, &ctx, &stop))?;
        }
        Ok(Self {
            child,
            stop,
            commands: rx,
        })
    }

    /// `Some(exit status)` once the process has ended.
    pub fn exited(&mut self) -> Option<String> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(status.to_string()),
            Ok(None) => None,
            Err(err) => Some(err.to_string()),
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_reader(
    stream: impl std::io::Read + Send + 'static,
    tx: Sender<Command>,
    ctx: egui::Context,
    parse_json: bool,
) {
    let _ = std::thread::Builder::new()
        .name("behavior-out".into())
        .spawn(move || {
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else { break };
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let cmd = if parse_json && trimmed.starts_with('{') {
                    serde_json::from_str(trimmed)
                        .unwrap_or_else(|err| Command::Log(format!("{trimmed}  ⚠ {err}")))
                } else {
                    Command::Log(line)
                };
                if tx.send(cmd).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
}

fn watch_legacy_files(dir: &Path, tx: &Sender<Command>, ctx: &egui::Context, stop: &AtomicBool) {
    let trigger = dir.join(LEGACY_TRIGGER);
    let actions = dir.join(LEGACY_ACTIONS);
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(30));
        let fired = std::fs::read_to_string(&trigger)
            .map(|s| s.trim() == "1")
            .unwrap_or(false);
        if !fired {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&actions) else {
            continue;
        };
        if let Ok(mut f) = std::fs::File::create(&trigger) {
            let _ = f.write_all(b"0");
        }
        let cmd = parse_legacy(&text)
            .unwrap_or_else(|| Command::Log(format!("unreadable action: {}", text.trim())));
        if tx.send(cmd).is_err() {
            break;
        }
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_show_with_position() {
        let cmd = parse_legacy(r"[1,'temp\\walk.gif','1-(120,40)']").unwrap();
        let Command::Show { file, x, y, .. } = cmd else {
            panic!("{cmd:?}")
        };
        let expected = if cfg!(windows) { r"temp\walk.gif" } else { "temp/walk.gif" };
        assert_eq!(file, PathBuf::from(expected));
        assert_eq!((x, y), (Some(120.0), Some(40.0)));
    }

    #[test]
    fn legacy_show_without_position_and_file_with_spaces() {
        let cmd = parse_legacy(r"[1,'temp\\022Fl (1).gif','0-(0,0)']").unwrap();
        let Command::Show { file, x, .. } = cmd else {
            panic!("{cmd:?}")
        };
        assert!(file.to_string_lossy().ends_with("022Fl (1).gif"));
        assert_eq!(x, None);
    }

    #[test]
    fn legacy_hide() {
        assert_eq!(
            parse_legacy("[0,'temp\\\\a.gif','0-(0,0)']"),
            Some(Command::Hide { slot: "main".into() })
        );
    }

    #[test]
    fn package_round_trip() {
        let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/behaviors/bouncer");
        let tmp = std::env::temp_dir().join(format!("open-anima-pkg-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let archive = tmp.join("bouncer.oapack");
        create_package(&example, &archive).unwrap();
        let installed = install(&archive, &tmp).unwrap();
        assert_eq!(installed.manifest.name, "Bouncer");
        assert_eq!(installed.manifest.protocol, Protocol::Stdio);
        assert!(installed.dir.join("assets/ball.png").is_file());
        assert!(installed.preview_path().is_some());
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn json_commands() {
        let cmd: Command =
            serde_json::from_str(r#"{"op":"show","file":"assets/a.gif","x":1,"y":2}"#).unwrap();
        assert!(matches!(cmd, Command::Show { ref slot, click_through: true, .. } if slot == "main"));
        let cmd: Command = serde_json::from_str(r#"{"op":"hide","slot":"cat"}"#).unwrap();
        assert_eq!(cmd, Command::Hide { slot: "cat".into() });
    }
}
