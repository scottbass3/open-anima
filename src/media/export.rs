// SPDX-License-Identifier: AGPL-3.0-or-later
//! Writing frames back to disk as an animated PNG, which keeps full alpha (unlike GIF).

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use anyhow::bail;

use super::decode::RawFrame;

pub fn write_apng(path: &Path, frames: &[RawFrame]) -> anyhow::Result<()> {
    let Some(first) = frames.first() else {
        bail!("nothing to export");
    };
    let (width, height) = first.image.dimensions();
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    if frames.len() > 1 {
        encoder.set_animated(frames.len() as u32, 0)?;
    }
    let mut writer = encoder.write_header()?;
    for frame in frames {
        if frames.len() > 1 {
            let ms = frame.delay.as_millis().clamp(1, u16::MAX as u128) as u16;
            writer.set_frame_delay(ms, 1000)?;
            writer.set_dispose_op(png::DisposeOp::None)?;
            writer.set_blend_op(png::BlendOp::Source)?;
        }
        if frame.image.dimensions() != (width, height) {
            // Sprite sheet cells and video frames always match; anything else is resized.
            let resized = image::imageops::resize(
                &frame.image,
                width,
                height,
                image::imageops::FilterType::Triangle,
            );
            writer.write_image_data(resized.as_raw())?;
        } else {
            writer.write_image_data(frame.image.as_raw())?;
        }
    }
    writer.finish()?;
    Ok(())
}
