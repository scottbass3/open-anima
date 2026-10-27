// SPDX-License-Identifier: AGPL-3.0-or-later
//! AI background removal with salient-object segmentation models (U²-Net family, ONNX).

use std::path::Path;
use std::sync::Arc;

use anyhow::Context as _;
use image::{GrayImage, RgbaImage};
use tract_onnx::prelude::*;

type Plan = Arc<TypedRunnableModel>;

pub struct Segmenter {
    plan: Plan,
    side: usize,
}

impl Segmenter {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let model = tract_onnx::onnx()
            .model_for_path(path)
            .with_context(|| format!("reading model {}", path.display()))?;
        // U²-Net, silueta and friends take 320×320; IS-Net takes 1024×1024. Use the size declared
        // by the model when it has one.
        let side = model
            .input_fact(0)
            .ok()
            .and_then(|fact| fact.shape.as_concrete_finite().ok().flatten())
            .and_then(|shape| shape.get(2).copied())
            .unwrap_or(320);
        let plan = model
            .with_input_fact(0, f32::fact([1, 3, side, side]).into())?
            .into_optimized()?
            .into_runnable()?;
        Ok(Self { plan, side })
    }

    /// Foreground probability of every pixel of `image`, as a mask of the same size.
    pub fn mask(&self, image: &RgbaImage) -> anyhow::Result<GrayImage> {
        let side = self.side as u32;
        let small = image::imageops::resize(
            image,
            side,
            side,
            image::imageops::FilterType::Triangle,
        );
        // Same preprocessing as the reference implementation: scale by the max, then ImageNet
        // normalisation.
        let max = small
            .pixels()
            .flat_map(|p| p.0[..3].iter().copied())
            .max()
            .unwrap_or(255)
            .max(1) as f32;
        const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
        const STD: [f32; 3] = [0.229, 0.224, 0.225];
        let input = tract_ndarray::Array4::from_shape_fn(
            (1, 3, self.side, self.side),
            |(_, c, y, x)| {
                let v = small.get_pixel(x as u32, y as u32).0[c] as f32 / max;
                (v - MEAN[c]) / STD[c]
            },
        );
        let outputs = self.plan.run(tvec!(Tensor::from(input).into()))?;
        let pred = outputs[0].to_plain_array_view::<f32>()?;
        let values: Vec<f32> = pred.iter().copied().collect();
        let (lo, hi) = values
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        let range = (hi - lo).max(1e-6);
        let mask = GrayImage::from_fn(side, side, |x, y| {
            let v = values[(y * side + x) as usize];
            image::Luma([(((v - lo) / range) * 255.0).round() as u8])
        });
        Ok(image::imageops::resize(
            &mask,
            image.width(),
            image.height(),
            image::imageops::FilterType::Triangle,
        ))
    }
}
