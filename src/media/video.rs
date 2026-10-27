// SPDX-License-Identifier: AGPL-3.0-or-later
//! Video playback through an external `ffmpeg` process that pipes raw RGBA frames.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context as _, bail};
use crossbeam_channel::{Receiver, bounded};
use image::RgbaImage;

use super::decode::RawFrame;
use crate::process;
use super::process as edit;
use crate::config::DecodeOptions;

/// Videos bigger than this are scaled down: nobody wants a 4K overlay and memory adds up fast.
const MAX_SIDE: u32 = 1280;

#[derive(Clone, Copy, Debug)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub fps: f32,
}

/// Locates `ffmpeg`, either the configured one or the one in `PATH`.
pub fn find_ffmpeg(configured: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = configured {
        return path.is_file().then(|| path.to_path_buf());
    }
    let exe = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join(exe))
        .find(|p| p.is_file())
}

fn ffprobe_next_to(ffmpeg: &Path) -> PathBuf {
    let name = if cfg!(windows) { "ffprobe.exe" } else { "ffprobe" };
    ffmpeg.with_file_name(name)
}

pub fn probe(ffmpeg: &Path, path: &Path) -> anyhow::Result<VideoInfo> {
    let out = process::command(ffprobe_next_to(ffmpeg))
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,avg_frame_rate,r_frame_rate",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .context("running ffprobe (it should be installed next to ffmpeg)")?;
    if !out.status.success() {
        bail!("ffprobe: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let fields: Vec<&str> = text.trim().split(',').collect();
    let [w, h, avg, r] = fields[..] else {
        bail!("unexpected ffprobe output: {text}");
    };
    let rate = |s: &str| -> Option<f32> {
        let (n, d) = s.split_once('/').unwrap_or((s, "1"));
        let (n, d): (f32, f32) = (n.parse().ok()?, d.parse().ok()?);
        (d > 0.0 && n > 0.0).then_some(n / d)
    };
    let fps = rate(avg).or_else(|| rate(r)).unwrap_or(30.0).clamp(1.0, 120.0);
    let (mut width, mut height): (u32, u32) = (w.parse()?, h.parse()?);
    let longest = width.max(height);
    if longest > MAX_SIDE {
        width = (width * MAX_SIDE / longest) & !1;
        height = (height * MAX_SIDE / longest) & !1;
    }
    Ok(VideoInfo { width, height, fps })
}

fn ffmpeg_decode(ffmpeg: &Path, path: &Path, info: &VideoInfo, looping: bool) -> Command {
    let mut cmd = process::command(ffmpeg);
    cmd.args(["-v", "error", "-nostdin"]);
    if looping {
        cmd.args(["-stream_loop", "-1"]);
    }
    // The native VP8/VP9 decoders drop the alpha channel; libvpx keeps it.
    match path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).as_deref() {
        Some("webm") => {
            cmd.args(["-c:v", "libvpx-vp9"]);
        }
        _ => {}
    }
    cmd.arg("-i")
        .arg(path)
        .args(["-an", "-f", "rawvideo", "-pix_fmt", "rgba", "-vf"])
        .arg(format!("scale={}:{}", info.width, info.height))
        .arg("-")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    cmd
}

/// A looping video being decoded in the background.
pub struct VideoStream {
    pub info: VideoInfo,
    pub frames: Receiver<RgbaImage>,
    child: Arc<Mutex<Child>>,
}

impl VideoStream {
    pub fn start(
        ffmpeg: &Path,
        path: &Path,
        info: VideoInfo,
        opts: DecodeOptions,
    ) -> anyhow::Result<Self> {
        let mut child = ffmpeg_decode(ffmpeg, path, &info, true)
            .spawn()
            .context("starting ffmpeg")?;
        process::adopt(&child);
        let mut stdout = child.stdout.take().context("ffmpeg stdout")?;
        // A few frames of buffer: enough to absorb hiccups, small enough to stay cheap.
        let (tx, rx) = bounded(4);
        let frame_len = (info.width * info.height * 4) as usize;
        std::thread::Builder::new()
            .name("video-decode".into())
            .spawn(move || {
                loop {
                    let mut buf = vec![0u8; frame_len];
                    if stdout.read_exact(&mut buf).is_err() {
                        break;
                    }
                    let Some(mut img) = RgbaImage::from_raw(info.width, info.height, buf) else {
                        break;
                    };
                    edit::apply(&mut img, &opts);
                    if tx.send(img).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            info,
            frames: rx,
            child: Arc::new(Mutex::new(child)),
        })
    }
}

impl Drop for VideoStream {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Decodes a whole video into memory, for editing and exports.
pub fn decode_all(
    ffmpeg: &Path,
    path: &Path,
    max_frames: usize,
) -> anyhow::Result<Vec<RawFrame>> {
    let info = probe(ffmpeg, path)?;
    let mut child = ffmpeg_decode(ffmpeg, path, &info, false).spawn()?;
    process::adopt(&child);
    let mut stdout = child.stdout.take().context("ffmpeg stdout")?;
    let frame_len = (info.width * info.height * 4) as usize;
    let delay = Duration::from_secs_f32(1.0 / info.fps);
    let mut frames = Vec::new();
    while frames.len() < max_frames {
        let mut buf = vec![0u8; frame_len];
        if stdout.read_exact(&mut buf).is_err() {
            break;
        }
        if let Some(image) = RgbaImage::from_raw(info.width, info.height, buf) {
            frames.push(RawFrame { image, delay });
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    if frames.is_empty() {
        bail!("ffmpeg produced no frame");
    }
    Ok(frames)
}

/// Extracts the audio track to a WAV file so rodio can loop it. Returns `None` for silent videos.
pub fn extract_audio(ffmpeg: &Path, path: &Path, cache: &Path) -> Option<PathBuf> {
    let stamp = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let name = format!(
        "{}-{:x}.wav",
        path.file_stem()?.to_string_lossy(),
        hash(&(path, stamp))
    );
    let out = cache.join(name);
    if out.is_file() {
        return Some(out);
    }
    let status = process::command(ffmpeg)
        .args(["-v", "error", "-nostdin", "-y", "-i"])
        .arg(path)
        .args(["-vn", "-ac", "2", "-ar", "44100", "-f", "wav"])
        .arg(&out)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    // ffmpeg writes a header-only file when there is no audio stream.
    let big_enough = std::fs::metadata(&out).map(|m| m.len() > 1024).unwrap_or(false);
    if status.success() && big_enough {
        Some(out)
    } else {
        let _ = std::fs::remove_file(&out);
        None
    }
}

fn hash(value: &impl std::hash::Hash) -> u64 {
    use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
    BuildHasherDefault::<DefaultHasher>::default().hash_one(value)
}
