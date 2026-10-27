// SPDX-License-Identifier: AGPL-3.0-or-later
//! Decoding of still and animated images into RGBA frames.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, bail};
use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::{AnimationDecoder, RgbaImage};

use crate::config::SpriteSheet;

/// A decoded frame and how long it stays on screen.
#[derive(Clone)]
pub struct RawFrame {
    pub image: RgbaImage,
    pub delay: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    Package,
    Unknown,
}

pub fn kind_of(path: &Path) -> MediaKind {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "gif" | "png" | "apng" | "webp" | "jpg" | "jpeg" | "bmp" | "tga" | "tif" | "tiff" | "ico"
        | "qoi" => MediaKind::Image,
        "mp4" | "m4v" | "webm" | "mov" | "mkv" | "avi" | "wmv" | "flv" | "mpg" | "mpeg" => {
            MediaKind::Video
        }
        "mp3" | "wav" | "ogg" | "flac" | "m4a" | "aac" | "opus" => MediaKind::Audio,
        "oapack" => MediaKind::Package,
        _ => MediaKind::Unknown,
    }
}

/// Browsers show GIF frames with a delay of 10 ms or less for 100 ms; so do we, otherwise many
/// GIFs found online play absurdly fast.
fn normalize_delay(delay: image::Delay) -> Duration {
    let (num, den) = delay.numer_denom_ms();
    let ms = if den == 0 { 0.0 } else { num as f64 / den as f64 };
    if ms <= 10.0 {
        Duration::from_millis(100)
    } else {
        Duration::from_secs_f64(ms / 1000.0)
    }
}

fn collect(frames: image::Frames<'_>) -> anyhow::Result<Vec<RawFrame>> {
    frames
        .map(|f| {
            let f = f?;
            Ok(RawFrame {
                delay: normalize_delay(f.delay()),
                image: f.into_buffer(),
            })
        })
        .collect()
}

pub fn decode_image(path: &Path) -> anyhow::Result<Vec<RawFrame>> {
    let open = || -> anyhow::Result<BufReader<File>> {
        Ok(BufReader::new(
            File::open(path).with_context(|| format!("opening {}", path.display()))?,
        ))
    };
    let format = image::ImageReader::new(open()?)
        .with_guessed_format()?
        .format()
        .context("unrecognised image format")?;

    let frames = match format {
        image::ImageFormat::Gif => collect(GifDecoder::new(open()?)?.into_frames())?,
        image::ImageFormat::Png => {
            let decoder = PngDecoder::new(open()?)?;
            if decoder.is_apng()? {
                collect(decoder.apng()?.into_frames())?
            } else {
                still(path)?
            }
        }
        image::ImageFormat::WebP => {
            let decoder = WebPDecoder::new(open()?)?;
            if decoder.has_animation() {
                collect(decoder.into_frames())?
            } else {
                still(path)?
            }
        }
        _ => still(path)?,
    };
    if frames.is_empty() {
        bail!("the file contains no frame");
    }
    Ok(frames)
}

fn still(path: &Path) -> anyhow::Result<Vec<RawFrame>> {
    let image = image::ImageReader::open(path)?
        .with_guessed_format()?
        .decode()?
        .into_rgba8();
    Ok(vec![RawFrame {
        image,
        delay: Duration::from_secs(1),
    }])
}

/// Cuts every frame of `frames` (usually a single still image) into a grid of cells.
pub fn slice_sprite_sheet(frames: Vec<RawFrame>, sheet: &SpriteSheet) -> Vec<RawFrame> {
    let columns = sheet.columns.max(1);
    let rows = sheet.rows.max(1);
    let delay = Duration::from_secs_f32(1.0 / sheet.fps.clamp(0.1, 240.0));
    let max = if sheet.frames == 0 {
        columns * rows
    } else {
        sheet.frames.min(columns * rows)
    };
    let mut out = Vec::new();
    for frame in frames {
        let cell_w = frame.image.width() / columns;
        let cell_h = frame.image.height() / rows;
        if cell_w == 0 || cell_h == 0 {
            out.push(frame);
            continue;
        }
        for index in 0..max {
            let (cx, cy) = (index % columns, index / columns);
            let cell =
                image::imageops::crop_imm(&frame.image, cx * cell_w, cy * cell_h, cell_w, cell_h)
                    .to_image();
            out.push(RawFrame { image: cell, delay });
        }
    }
    out
}
