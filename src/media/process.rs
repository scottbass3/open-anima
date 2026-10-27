// SPDX-License-Identifier: AGPL-3.0-or-later
//! Pixel edits applied to decoded frames.

use image::RgbaImage;

use crate::config::{ColorKey, DecodeOptions};

pub fn apply(image: &mut RgbaImage, opts: &DecodeOptions) {
    if let Some(key) = &opts.color_key {
        color_key(image, key);
    }
    if opts.flip_h {
        image::imageops::flip_horizontal_in_place(image);
    }
    if opts.flip_v {
        image::imageops::flip_vertical_in_place(image);
    }
}

/// Fades out pixels close to the key color. Distances are normalised to 0‥1.
pub fn color_key(image: &mut RgbaImage, key: &ColorKey) {
    let [kr, kg, kb] = key.color.map(|c| c as f32);
    let norm = 255.0 * 3f32.sqrt();
    let tolerance = key.tolerance.max(0.0);
    let softness = key.softness.max(1e-4);
    for px in image.pixels_mut() {
        let [r, g, b, a] = px.0;
        let (dr, dg, db) = (r as f32 - kr, g as f32 - kg, b as f32 - kb);
        let dist = (dr * dr + dg * dg + db * db).sqrt() / norm;
        let factor = ((dist - tolerance) / softness).clamp(0.0, 1.0);
        px.0[3] = (a as f32 * factor).round() as u8;
    }
}

/// Color of the top-left pixel, which is the background for most green-screen animations.
pub fn corner_color(image: &RgbaImage) -> [u8; 3] {
    let p = image.get_pixel(0, 0).0;
    [p[0], p[1], p[2]]
}

/// Multiplies the alpha channel with `mask` (same size, 0‥255).
pub fn apply_mask(image: &mut RgbaImage, mask: &image::GrayImage) {
    for (px, m) in image.pixels_mut().zip(mask.pixels()) {
        px.0[3] = ((px.0[3] as u16 * m.0[0] as u16) / 255) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_key_removes_exact_match_and_keeps_others() {
        let mut img = RgbaImage::from_pixel(2, 1, image::Rgba([0, 255, 0, 255]));
        img.put_pixel(1, 0, image::Rgba([200, 30, 30, 255]));
        color_key(&mut img, &ColorKey::default());
        assert_eq!(img.get_pixel(0, 0).0[3], 0);
        assert_eq!(img.get_pixel(1, 0).0[3], 255);
    }
}
