// SPDX-License-Identifier: AGPL-3.0-or-later
//! Loading media and playing it back.

#[cfg(feature = "ai")]
pub mod ai;
pub mod decode;
pub mod export;
pub mod process;
pub mod video;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use anyhow::{Context as _, bail};
use crossbeam_channel::{Receiver, bounded};
use eframe::egui::{self, ColorImage, TextureHandle, TextureId, TextureOptions};

use crate::config::{DecodeOptions, Playback};
use decode::{MediaKind, RawFrame};
use video::VideoStream;

/// Upper bound on frames decoded from a video for editing, to keep memory in check.
pub const MAX_EDIT_FRAMES: usize = 900;

/// Seconds since the start of the program. Every caller of [`Anim::frame`] uses this clock.
pub fn clock() -> f64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// A frame in the layout native overlays want: premultiplied BGRA, rows top to bottom.
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    /// Unique per frame, so renderers can skip unchanged frames.
    pub serial: u64,
}

impl Pixels {
    fn from_rgba(img: &image::RgbaImage) -> Arc<Self> {
        static SERIAL: AtomicU64 = AtomicU64::new(1);
        let mut bgra = Vec::with_capacity(img.as_raw().len());
        for px in img.pixels() {
            let [r, g, b, a] = px.0;
            let pm = |c: u8| ((c as u16 * a as u16 + 127) / 255) as u8;
            bgra.extend_from_slice(&[pm(b), pm(g), pm(r), a]);
        }
        Arc::new(Self {
            width: img.width(),
            height: img.height(),
            bgra,
            serial: SERIAL.fetch_add(1, Ordering::Relaxed),
        })
    }
}

struct Frame {
    tex: TextureHandle,
    pixels: Arc<Pixels>,
    /// Seconds.
    delay: f64,
}

enum Content {
    Frames {
        frames: Vec<Frame>,
        total: f64,
    },
    Video {
        stream: VideoStream,
        tex: TextureHandle,
        pixels: Arc<Pixels>,
        /// Clock time at which the next video frame is due.
        next_at: f64,
    },
}

/// What to display now.
pub struct FrameRef {
    pub tex: TextureId,
    pub pixels: Arc<Pixels>,
    /// Seconds until the frame changes.
    pub next: f64,
}

/// A loaded animation and its playback clock.
pub struct Anim {
    pub size: [u32; 2],
    /// Sound track extracted from a video, if any.
    pub audio_track: Option<PathBuf>,
    content: Content,
    clock: f64,
    last_now: Option<f64>,
}

impl Anim {
    pub fn frame_count(&self) -> Option<usize> {
        match &self.content {
            Content::Frames { frames, .. } => Some(frames.len()),
            Content::Video { .. } => None,
        }
    }

    pub fn restart(&mut self) {
        self.clock = 0.0;
        self.last_now = None;
        if let Content::Video { next_at, .. } = &mut self.content {
            *next_at = 0.0;
        }
    }

    /// Advances the playback clock to `now` (see [`clock`]) and returns the frame to show.
    ///
    /// Calling it several times with the same `now` (preview and overlay) is harmless.
    pub fn frame(&mut self, now: f64, playback: &Playback) -> FrameRef {
        let speed = playback.speed.max(0.01) as f64;
        let dt = self.last_now.map_or(0.0, |last| (now - last).max(0.0));
        self.last_now = Some(self.last_now.map_or(now, |last| last.max(now)));
        if !playback.paused {
            self.clock += dt * speed;
        }
        let clock = self.clock;
        let fps_override = playback.fps_override.map(|f| f.clamp(0.1, 240.0) as f64);

        let (tex, pixels, next) = match &mut self.content {
            Content::Frames { frames, total } => {
                let (frame, next) = if frames.len() == 1 {
                    (&frames[0], 1.0)
                } else if let Some(fps) = fps_override {
                    let step = 1.0 / fps;
                    let index = (clock / step) as usize % frames.len();
                    (&frames[index], step - clock % step)
                } else {
                    let t = clock % *total;
                    let mut end = 0.0;
                    let mut found = (&frames[0], frames[0].delay);
                    for frame in frames.iter() {
                        end += frame.delay;
                        if t < end {
                            found = (frame, end - t);
                            break;
                        }
                    }
                    found
                };
                (frame.tex.id(), frame.pixels.clone(), next)
            }
            Content::Video {
                stream,
                tex,
                pixels,
                next_at,
            } => {
                let step = 1.0 / fps_override.unwrap_or(stream.info.fps as f64);
                if clock >= *next_at {
                    match stream.frames.try_recv() {
                        Ok(img) => {
                            tex.set(to_color_image(&img), TextureOptions::LINEAR);
                            *pixels = Pixels::from_rgba(&img);
                            *next_at += step;
                            // Fell far behind (window hidden, system busy): resync instead of
                            // fast-forwarding.
                            if *next_at < clock - 0.25 {
                                *next_at = clock + step;
                            }
                        }
                        // The decoder hasn't caught up yet.
                        Err(_) => {
                            return FrameRef {
                                tex: tex.id(),
                                pixels: pixels.clone(),
                                next: 0.005,
                            };
                        }
                    }
                }
                (tex.id(), pixels.clone(), *next_at - clock)
            }
        };
        let next = if playback.paused { 0.25 } else { next / speed };
        FrameRef {
            tex,
            pixels,
            next: next.clamp(0.001, 1.0),
        }
    }
}

#[cfg(test)]
impl Anim {
    /// A single transparent pixel.
    pub fn test_dummy() -> Self {
        let img = image::RgbaImage::new(1, 1);
        let ctx = egui::Context::default();
        let frame = Frame {
            tex: ctx.load_texture("test", to_color_image(&img), TextureOptions::LINEAR),
            pixels: Pixels::from_rgba(&img),
            delay: 1.0,
        };
        Self {
            size: [1, 1],
            audio_track: None,
            content: Content::Frames {
                frames: vec![frame],
                total: 1.0,
            },
            clock: 0.0,
            last_now: None,
        }
    }
}

pub fn to_color_image(img: &image::RgbaImage) -> ColorImage {
    ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw())
}

pub struct LoadRequest {
    pub name: String,
    pub source: PathBuf,
    pub decode: DecodeOptions,
    pub ffmpeg: Option<PathBuf>,
    pub cache: PathBuf,
}

pub type LoadResult = Result<Anim, String>;

/// Decodes and uploads media on a worker thread. Dropping the receiver abandons the load.
pub fn spawn_load(ctx: egui::Context, req: LoadRequest) -> Receiver<LoadResult> {
    let (tx, rx) = bounded(1);
    std::thread::Builder::new()
        .name("media-load".into())
        .spawn(move || {
            let result = load(&ctx, &req).map_err(|e| format!("{e:#}"));
            if let Err(err) = &result {
                log::warn!("loading {}: {err}", req.source.display());
            }
            let _ = tx.send(result);
            ctx.request_repaint();
        })
        .expect("spawning loader thread");
    rx
}

fn load(ctx: &egui::Context, req: &LoadRequest) -> anyhow::Result<Anim> {
    let tex_name = format!("anim:{}", req.name);
    match decode::kind_of(&req.source) {
        MediaKind::Video => {
            let ffmpeg = video::find_ffmpeg(req.ffmpeg.as_deref())
                .context("ffmpeg is required to play videos")?;
            let info = video::probe(&ffmpeg, &req.source)?;
            let stream = VideoStream::start(&ffmpeg, &req.source, info, req.decode.clone())?;
            let blank = ColorImage::filled(
                [info.width as usize, info.height as usize],
                egui::Color32::TRANSPARENT,
            );
            let tex = ctx.load_texture(tex_name, blank, TextureOptions::LINEAR);
            let pixels = Pixels::from_rgba(&image::RgbaImage::new(info.width, info.height));
            let audio_track = video::extract_audio(&ffmpeg, &req.source, &req.cache);
            Ok(Anim {
                size: [info.width, info.height],
                audio_track,
                content: Content::Video {
                    stream,
                    tex,
                    pixels,
                    next_at: 0.0,
                },
                clock: 0.0,
                last_now: None,
            })
        }
        MediaKind::Image => {
            let frames = decode_frames(&req.source, &req.decode, None)?;
            let size = [frames[0].image.width(), frames[0].image.height()];
            let frames: Vec<Frame> = frames
                .iter()
                .enumerate()
                .map(|(i, f)| Frame {
                    tex: ctx.load_texture(
                        format!("{tex_name}#{i}"),
                        to_color_image(&f.image),
                        TextureOptions::LINEAR,
                    ),
                    pixels: Pixels::from_rgba(&f.image),
                    delay: f.delay.as_secs_f64().max(0.001),
                })
                .collect();
            let total = frames.iter().map(|f| f.delay).sum();
            Ok(Anim {
                size,
                audio_track: None,
                content: Content::Frames { frames, total },
                clock: 0.0,
                last_now: None,
            })
        }
        _ => bail!("unsupported file type"),
    }
}

/// Longest side of library thumbnails.
const THUMB_SIDE: u32 = 256;

/// First frame of a file, small, for the library grid.
pub fn thumbnail(source: &Path, opts: &DecodeOptions, ffmpeg: Option<&Path>) -> Option<ColorImage> {
    let first = match decode::kind_of(source) {
        MediaKind::Video => {
            let ffmpeg = video::find_ffmpeg(ffmpeg)?;
            video::decode_all(&ffmpeg, source, 1).ok()?.into_iter().next()?
        }
        MediaKind::Image => {
            // Decoding only the first frame is much cheaper than the whole animation.
            let image = image::ImageReader::open(source)
                .ok()?
                .with_guessed_format()
                .ok()?
                .decode()
                .map(|i| i.into_rgba8())
                .or_else(|_| decode::decode_image(source).map(|f| f[0].image.clone()))
                .ok()?;
            let frame = RawFrame {
                image,
                delay: Default::default(),
            };
            match &opts.sprite_sheet {
                Some(sheet) => decode::slice_sprite_sheet(vec![frame], sheet).into_iter().next()?,
                None => frame,
            }
        }
        _ => return None,
    };
    let mut image = first.image;
    process::apply(&mut image, opts);
    let (w, h) = image.dimensions();
    if w.max(h) > THUMB_SIDE {
        let scale = THUMB_SIDE as f32 / w.max(h) as f32;
        image = image::imageops::resize(
            &image,
            ((w as f32 * scale) as u32).max(1),
            ((h as f32 * scale) as u32).max(1),
            image::imageops::FilterType::Triangle,
        );
    }
    Some(to_color_image(&image))
}

/// Decodes every frame of any supported file with the edits of `opts` applied.
pub fn decode_frames(
    source: &Path,
    opts: &DecodeOptions,
    ffmpeg: Option<&Path>,
) -> anyhow::Result<Vec<RawFrame>> {
    let mut frames = match decode::kind_of(source) {
        MediaKind::Video => {
            let ffmpeg =
                video::find_ffmpeg(ffmpeg).context("ffmpeg is required to edit videos")?;
            video::decode_all(&ffmpeg, source, MAX_EDIT_FRAMES)?
        }
        _ => {
            let frames = decode::decode_image(source)?;
            match &opts.sprite_sheet {
                Some(sheet) => decode::slice_sprite_sheet(frames, sheet),
                None => frames,
            }
        }
    };
    for frame in &mut frames {
        process::apply(&mut frame.image, opts);
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use image::{Rgba, RgbaImage};

    use super::decode::{RawFrame, decode_image, slice_sprite_sheet};
    use super::export::write_apng;
    use crate::config::SpriteSheet;

    #[test]
    fn sprite_sheet_to_apng_round_trip() {
        // A 4×1 sheet whose cells are filled with different colors, transparent on the right.
        let mut sheet = RgbaImage::new(40, 10);
        for (x, _, px) in sheet.enumerate_pixels_mut() {
            *px = Rgba([(x / 10 * 60) as u8, 0, 0, if x % 10 < 5 { 255 } else { 0 }]);
        }
        let frames = slice_sprite_sheet(
            vec![RawFrame {
                image: sheet,
                delay: Duration::from_secs(1),
            }],
            &SpriteSheet {
                columns: 4,
                rows: 1,
                frames: 3,
                fps: 10.0,
            },
        );
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[2].image.get_pixel(0, 0).0, [120, 0, 0, 255]);

        let path = std::env::temp_dir().join(format!("open-anima-test-{}.png", std::process::id()));
        write_apng(&path, &frames).unwrap();
        let decoded = decode_image(&path).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded[1].image.dimensions(), (10, 10));
        assert_eq!(decoded[1].image.get_pixel(0, 0).0, [60, 0, 0, 255]);
        assert_eq!(decoded[1].image.get_pixel(9, 0).0[3], 0);
        assert_eq!(decoded[0].delay, Duration::from_millis(100));
    }
}
