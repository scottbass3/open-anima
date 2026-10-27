// SPDX-License-Identifier: AGPL-3.0-or-later
//! Importing animations from the Steam Workshop of Anima Engine.
//!
//! Workshop items are stored in Steam's content system: only the Steam client of an account
//! that owns the game can download them. Open Anima therefore never downloads them itself (except
//! the rare legacy items that have a public file URL): it shows the item, asks Steam to open it
//! so that the user can subscribe, waits for Steam to put the files in its library, and imports
//! them from there.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, bail};
use eframe::egui::ColorImage;
use serde::Deserialize;

use crate::behavior;
use crate::media::decode::{self, MediaKind};

pub const ANIMA_ENGINE_APP: u32 = 3474900;
const DETAILS_URL: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";

/// Extracts the item id from a Workshop link (or a bare id).
pub fn parse_id(input: &str) -> Option<u64> {
    let input = input.trim();
    if let Ok(id) = input.parse() {
        return Some(id);
    }
    // https://steamcommunity.com/sharedfiles/filedetails/?id=123&searchtext=
    if let Some(pos) = input.find("id=") {
        let digits: String = input[pos + 3..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        return digits.parse().ok();
    }
    // steam://url/CommunityFilePage/123
    let last = input.trim_end_matches('/').rsplit('/').next()?;
    last.parse().ok()
}

#[derive(Clone, Debug)]
pub struct Details {
    pub id: u64,
    pub app_id: u32,
    pub title: String,
    pub size: u64,
    pub preview_url: Option<String>,
    /// Only set for legacy items, which can be downloaded without Steam.
    pub file_url: Option<String>,
    pub file_name: Option<String>,
}

#[derive(Deserialize)]
struct ApiResponse {
    response: ApiDetailsList,
}

#[derive(Deserialize)]
struct ApiDetailsList {
    publishedfiledetails: Vec<ApiDetails>,
}

#[derive(Deserialize)]
struct ApiDetails {
    result: i32,
    #[serde(default)]
    consumer_app_id: u32,
    #[serde(default)]
    title: String,
    #[serde(default)]
    file_size: serde_json::Value,
    #[serde(default)]
    preview_url: String,
    #[serde(default)]
    file_url: String,
    #[serde(default)]
    filename: String,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .user_agent(concat!("OpenAnima/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// Public information about an item (no API key needed).
pub fn fetch_details(id: u64) -> anyhow::Result<Details> {
    let id_text = id.to_string();
    let response: ApiResponse = agent()
        .post(DETAILS_URL)
        .send_form([("itemcount", "1"), ("publishedfileids[0]", id_text.as_str())])
        .context("contacting Steam")?
        .body_mut()
        .read_json()
        .context("reading the answer of Steam")?;
    let Some(item) = response.response.publishedfiledetails.into_iter().next() else {
        bail!("Steam returned no item");
    };
    if item.result != 1 {
        bail!("this Workshop item doesn't exist or isn't public");
    }
    let size = match &item.file_size {
        serde_json::Value::String(s) => s.parse().unwrap_or(0),
        serde_json::Value::Number(n) => n.as_u64().unwrap_or(0),
        _ => 0,
    };
    let non_empty = |s: String| (!s.is_empty()).then_some(s);
    Ok(Details {
        id,
        app_id: item.consumer_app_id,
        title: item.title,
        size,
        preview_url: non_empty(item.preview_url),
        file_url: non_empty(item.file_url),
        file_name: non_empty(item.filename),
    })
}

/// The preview image of an item, reduced for display.
pub fn fetch_preview(url: &str) -> anyhow::Result<ColorImage> {
    let bytes = agent()
        .get(url)
        .call()?
        .body_mut()
        .with_config()
        .limit(16 * 1024 * 1024)
        .read_to_vec()?;
    let image = image::load_from_memory(&bytes)?.thumbnail(512, 512).into_rgba8();
    Ok(crate::media::to_color_image(&image))
}

/// Downloads a legacy item that has a public file URL into `dir`.
pub fn download_legacy(details: &Details, dir: &Path) -> anyhow::Result<()> {
    let url = details.file_url.as_deref().context("no public file")?;
    let name = details
        .file_name
        .as_deref()
        .and_then(|n| Path::new(n).file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("{}.bin", details.id));
    std::fs::create_dir_all(dir)?;
    let mut response = agent().get(url).call()?;
    let mut file = std::fs::File::create(dir.join(name))?;
    std::io::copy(&mut response.body_mut().as_reader(), &mut file)?;
    Ok(())
}

/// Folders of the Steam libraries of this computer.
pub fn steam_libraries() -> Vec<PathBuf> {
    let Some(steam) = steam_path() else {
        return Vec::new();
    };
    let mut libraries = vec![steam.clone()];
    let vdf = std::fs::read_to_string(steam.join("steamapps").join("libraryfolders.vdf"))
        .unwrap_or_default();
    for line in vdf.lines() {
        let mut parts = line.trim().split('"').filter(|s| !s.trim().is_empty());
        if parts.next() == Some("path")
            && let Some(path) = parts.next()
        {
            let path = PathBuf::from(path.replace("\\\\", "\\"));
            if !libraries.iter().any(|l| same_path(l, &path)) {
                libraries.push(path);
            }
        }
    }
    libraries
}

fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase().replace('/', "\\")
        == b.to_string_lossy().to_lowercase().replace('/', "\\")
}

#[cfg(windows)]
fn steam_path() -> Option<PathBuf> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (key, value) = (wide(r"Software\Valve\Steam"), wide("SteamPath"));
    let mut buf = vec![0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != 0 {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(PathBuf::from(String::from_utf16_lossy(&buf[..len])))
}

#[cfg(not(windows))]
fn steam_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    [".steam/steam", ".local/share/Steam", "Library/Application Support/Steam"]
        .into_iter()
        .map(|p| home.join(p))
        .find(|p| p.is_dir())
}

/// Where Steam put the files of a subscribed item, once its download is complete.
pub fn local_item(libraries: &[PathBuf], app_id: u32, id: u64) -> Option<PathBuf> {
    libraries
        .iter()
        .map(|lib| {
            lib.join("steamapps")
                .join("workshop")
                .join("content")
                .join(app_id.to_string())
                .join(id.to_string())
        })
        .find(|dir| std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some()))
}

/// Whether the game is installed: Steam only downloads the Workshop items of installed games.
pub fn app_installed(libraries: &[PathBuf], app_id: u32) -> bool {
    libraries.iter().any(|lib| {
        lib.join("steamapps")
            .join(format!("appmanifest_{app_id}.acf"))
            .is_file()
    })
}

/// Opens the item in the Steam client, where the user can subscribe to it.
pub fn open_in_steam(id: u64) -> anyhow::Result<()> {
    open_steam_url(&format!("steam://url/CommunityFilePage/{id}"))
}

/// Asks the Steam client to install the game.
pub fn install_in_steam(app_id: u32) -> anyhow::Result<()> {
    open_steam_url(&format!("steam://install/{app_id}"))
}

fn open_steam_url(url: &str) -> anyhow::Result<()> {
    #[cfg(windows)]
    let mut cmd = {
        let mut cmd = crate::process::command("cmd");
        cmd.args(["/C", "start", "", url]);
        cmd
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut cmd = std::process::Command::new("open");
        cmd.arg(url);
        cmd
    };
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let mut cmd = {
        let mut cmd = std::process::Command::new("xdg-open");
        cmd.arg(url);
        cmd
    };
    cmd.spawn().context("opening Steam")?;
    Ok(())
}

// -------------------------------------------------------------------------------------------
// SteamCMD

/// Valve's command line Steam client, which can download Workshop items without installing the
/// game (anonymously when the game allows it, otherwise with the user's account).
#[cfg(windows)]
const STEAMCMD_URL: &str = "https://steamcdn-a.akamaihd.net/client/installer/steamcmd.zip";

/// Exit code of SteamCMD after it updated itself; it has to be started again.
const STEAMCMD_UPDATED: i32 = 7;

pub fn steamcmd_supported() -> bool {
    cfg!(windows)
}

fn steamcmd_exe(dir: &Path) -> PathBuf {
    dir.join("steamcmd.exe")
}

/// Downloads SteamCMD into `dir` unless it is already there.
#[cfg(windows)]
fn ensure_steamcmd(dir: &Path, progress: &dyn Fn(String)) -> anyhow::Result<PathBuf> {
    let exe = steamcmd_exe(dir);
    if exe.is_file() {
        return Ok(exe);
    }
    progress("SteamCMD…".into());
    let bytes = agent()
        .get(STEAMCMD_URL)
        .call()
        .context("downloading SteamCMD")?
        .body_mut()
        .with_config()
        .limit(16 * 1024 * 1024)
        .read_to_vec()?;
    std::fs::create_dir_all(dir)?;
    zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .context("SteamCMD archive")?
        .extract(dir)?;
    anyhow::ensure!(exe.is_file(), "steamcmd.exe missing from the archive");
    Ok(exe)
}

#[cfg(not(windows))]
fn ensure_steamcmd(_dir: &Path, _progress: &dyn Fn(String)) -> anyhow::Result<PathBuf> {
    bail!("SteamCMD is only supported on Windows")
}

/// Downloads an item with SteamCMD and returns its folder.
///
/// Anonymously (`account` = `None`), SteamCMD runs hidden and its output is reported through
/// `progress`. With an account, it runs in its own console window so that the user types the
/// password and Steam Guard code there; Open Anima never sees them.
pub fn steamcmd_download(
    dir: &Path,
    app_id: u32,
    id: u64,
    account: Option<&str>,
    progress: &dyn Fn(String),
) -> anyhow::Result<PathBuf> {
    let exe = ensure_steamcmd(dir, progress)?;
    let target = dir
        .join("steamapps")
        .join("workshop")
        .join("content")
        .join(app_id.to_string())
        .join(id.to_string());
    let mut last_line = String::new();
    for _ in 0..3 {
        let mut cmd = match account {
            None => crate::process::command(&exe),
            Some(_) => std::process::Command::new(&exe),
        };
        cmd.current_dir(dir)
            .arg("+@ShutdownOnFailedCommand")
            .arg("1")
            .arg("+login")
            .arg(account.unwrap_or("anonymous"))
            .arg("+workshop_download_item")
            .arg(app_id.to_string())
            .arg(id.to_string())
            .arg("+quit");
        let status = match account {
            None => {
                use std::io::BufRead;
                let mut child = cmd
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .context("starting SteamCMD")?;
                crate::process::adopt(&child);
                let stdout = child.stdout.take().unwrap();
                for line in std::io::BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    let line = line.trim().to_owned();
                    if !line.is_empty() {
                        progress(line.clone());
                        last_line = line;
                    }
                }
                child.wait()?
            }
            Some(_) => {
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
                    cmd.creation_flags(CREATE_NEW_CONSOLE);
                }
                progress(String::new());
                let mut child = cmd.spawn().context("starting SteamCMD")?;
                child.wait()?
            }
        };
        if status.code() != Some(STEAMCMD_UPDATED) {
            break;
        }
    }
    if std::fs::read_dir(&target).is_ok_and(|mut e| e.next().is_some()) {
        return Ok(target);
    }
    if account.is_some() {
        last_line = steamcmd_log_error(dir).unwrap_or_default();
    }
    if last_line.is_empty() {
        bail!("SteamCMD didn't download the item");
    }
    bail!("SteamCMD didn't download the item: {last_line}")
}

/// Last error reported in SteamCMD's console log.
fn steamcmd_log_error(dir: &Path) -> Option<String> {
    let log = std::fs::read_to_string(dir.join("logs").join("console_log.txt")).ok()?;
    log.lines()
        .rev()
        .take(40)
        .find(|l| l.contains("ERROR") || l.contains("FAILED") || l.contains("failed"))
        .map(|l| l.split_once("] ").map_or(l, |(_, rest)| rest).trim().to_owned())
}

/// What an imported item contained.
#[derive(Debug, Default)]
pub struct Imported {
    pub media: Vec<PathBuf>,
    pub behaviors: Vec<PathBuf>,
    /// Files that look like packs but couldn't be opened.
    pub unsupported: Vec<PathBuf>,
}

/// Copies the files of an item into the library and sorts out what they are.
pub fn import(
    source: &Path,
    details: &Details,
    library: &Path,
    packages: &Path,
) -> anyhow::Result<Imported> {
    let dest = library
        .join("workshop")
        .join(format!("{}-{}", details.id, sanitize(&details.title)));
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    copy_dir(source, &dest)?;

    let mut imported = Imported::default();
    let mut stack = vec![dest];
    while let Some(dir) = stack.pop() {
        if is_behavior_dir(&dir) {
            imported.behaviors.push(dir);
            continue;
        }
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            match decode::kind_of(&path) {
                MediaKind::Image | MediaKind::Video => imported.media.push(path),
                MediaKind::Package => match behavior::install(&path, packages) {
                    Ok(package) => imported.behaviors.push(package.dir),
                    Err(_) => imported.unsupported.push(path),
                },
                _ if ext == "mdaa" || ext == "zip" => match extract_pack(&path, packages) {
                    Some(dir) => imported.behaviors.push(dir),
                    None => imported.unsupported.push(path),
                },
                _ => {}
            }
        }
    }
    Ok(imported)
}

/// A folder with a manifest, or an executable next to a `temp` folder (the layout of Anima
/// Engine packs).
fn is_behavior_dir(dir: &Path) -> bool {
    if dir.join(behavior::MANIFEST).is_file() {
        return true;
    }
    let has_exe = std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.filter_map(|e| e.ok()).any(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("exe"))
        })
    });
    has_exe && dir.join("temp").is_dir()
}

/// Extracts a zip-based pack and returns the folder holding its program.
fn extract_pack(archive: &Path, packages: &Path) -> Option<PathBuf> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(archive).ok()?).ok()?;
    let stem = sanitize(&archive.file_stem()?.to_string_lossy());
    let mut dir = packages.join(&stem);
    let mut n = 2;
    while dir.exists() {
        dir = packages.join(format!("{stem}-{n}"));
        n += 1;
    }
    zip.extract(&dir).ok()?;
    let mut stack = vec![dir];
    while let Some(dir) = stack.pop() {
        if is_behavior_dir(&dir) || behavior::Package::open(&dir).is_ok() {
            return Some(dir);
        }
        stack.extend(
            std::fs::read_dir(&dir)
                .ok()?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_dir()),
        );
    }
    None
}

fn copy_dir(from: &Path, to: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let path = entry?.path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_dir(&path, &target)?;
        } else {
            std::fs::copy(&path, &target)
                .with_context(|| format!("copying {}", path.display()))?;
        }
    }
    Ok(())
}

fn sanitize(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || "-_ ".contains(c) { c } else { '_' })
        .take(60)
        .collect();
    let clean = clean.trim().to_owned();
    if clean.is_empty() { "item".into() } else { clean }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_links() {
        let id = Some(3824772804);
        assert_eq!(
            parse_id("https://steamcommunity.com/sharedfiles/filedetails/?id=3824772804"),
            id
        );
        assert_eq!(
            parse_id("https://steamcommunity.com/sharedfiles/filedetails/?id=3824772804&searchtext=miku"),
            id
        );
        assert_eq!(parse_id("steam://url/CommunityFilePage/3824772804"), id);
        assert_eq!(parse_id(" 3824772804 "), id);
        assert_eq!(parse_id("https://example.com/"), None);
    }

    /// Needs network access: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn fetches_a_real_item() {
        let details = fetch_details(3820846364).unwrap();
        assert_eq!(details.app_id, ANIMA_ENGINE_APP);
        assert!(!details.title.is_empty());
        assert!(details.size > 0);
        let preview = fetch_preview(details.preview_url.as_deref().unwrap()).unwrap();
        assert!(preview.size[0] > 0 && preview.size[0] <= 512);
        assert!(fetch_details(1).is_err());
    }

    /// Downloads SteamCMD (about 45 MB) and a small item: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    #[cfg(windows)]
    fn downloads_with_steamcmd() {
        let root = std::env::temp_dir().join(format!("open-anima-steamcmd-{}", std::process::id()));
        let lines = std::sync::Mutex::new(Vec::new());
        let dir = steamcmd_download(&root.join("steamcmd"), ANIMA_ENGINE_APP, 3820846364, None, &|l| {
            lines.lock().unwrap().push(l)
        })
        .unwrap();
        assert!(!lines.lock().unwrap().is_empty());
        let details = fetch_details(3820846364).unwrap();
        let imported = import(&dir, &details, &root.join("library"), &root.join("packages")).unwrap();
        assert_eq!(imported.media.len(), 1, "{imported:?}");
        assert!(crate::media::decode::decode_image(&imported.media[0]).unwrap().len() > 1);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn imports_media_and_behaviors() {
        let root = std::env::temp_dir().join(format!("open-anima-ws-{}", std::process::id()));
        let item = root.join("steam/3474900/42");
        std::fs::create_dir_all(item.join("pack/temp")).unwrap();
        std::fs::write(item.join("pack/core.exe"), b"").unwrap();
        std::fs::write(item.join("pack/temp/walk.gif"), b"").unwrap();
        std::fs::create_dir_all(item.join("gifs")).unwrap();
        std::fs::write(item.join("gifs/dance.gif"), b"").unwrap();
        std::fs::write(item.join("readme.txt"), b"").unwrap();
        let details = Details {
            id: 42,
            app_id: ANIMA_ENGINE_APP,
            title: "Dancing cat!".into(),
            size: 0,
            preview_url: None,
            file_url: None,
            file_name: None,
        };
        let imported = import(&item, &details, &root.join("library"), &root.join("packages")).unwrap();
        assert_eq!(imported.media.len(), 1, "{imported:?}");
        assert!(imported.media[0].ends_with("gifs/dance.gif"));
        assert!(imported.media[0].starts_with(root.join("library/workshop/42-Dancing cat_")));
        assert_eq!(imported.behaviors.len(), 1);
        assert!(imported.behaviors[0].ends_with("pack"));
        std::fs::remove_dir_all(&root).ok();
    }
}
