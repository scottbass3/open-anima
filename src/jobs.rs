// SPDX-License-Identifier: AGPL-3.0-or-later
//! Slow edits (AI background removal, exports) running on worker threads.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crossbeam_channel::{Receiver, bounded};
use eframe::egui;

use crate::config::Item;
use crate::media;

pub enum JobDone {
    /// A processed copy of item `from` was written to `path`.
    NewItem { from: u64, path: PathBuf },
    Exported(PathBuf),
}

pub struct Job {
    pub label: String,
    progress: Arc<AtomicU32>,
    done: Receiver<Result<JobDone, String>>,
}

impl Job {
    /// 0‥1
    pub fn progress(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 1000.0
    }

    pub fn poll(&self) -> Option<Result<JobDone, String>> {
        self.done.try_recv().ok()
    }

    fn spawn(
        ctx: &egui::Context,
        label: String,
        work: impl FnOnce(&dyn Fn(f32)) -> anyhow::Result<JobDone> + Send + 'static,
    ) -> Self {
        let progress = Arc::new(AtomicU32::new(0));
        let (tx, rx) = bounded(1);
        let ctx = ctx.clone();
        let p = progress.clone();
        std::thread::Builder::new()
            .name("job".into())
            .spawn(move || {
                let report = |f: f32| {
                    p.store((f.clamp(0.0, 1.0) * 1000.0) as u32, Ordering::Relaxed);
                    ctx.request_repaint();
                };
                let result = work(&report).map_err(|e| format!("{e:#}"));
                let _ = tx.send(result);
                ctx.request_repaint();
            })
            .expect("spawning job thread");
        Self {
            label,
            progress,
            done: rx,
        }
    }

    pub fn export(
        ctx: &egui::Context,
        label: String,
        item: &Item,
        ffmpeg: Option<PathBuf>,
        out: PathBuf,
    ) -> Self {
        let (source, decode) = (item.source.clone(), item.decode.clone());
        Self::spawn(ctx, label, move |progress| {
            let frames = media::decode_frames(&source, &decode, ffmpeg.as_deref())?;
            progress(0.5);
            media::export::write_apng(&out, &frames)?;
            Ok(JobDone::Exported(out))
        })
    }

    #[cfg(feature = "ai")]
    pub fn remove_background(
        ctx: &egui::Context,
        label: String,
        item: &Item,
        ffmpeg: Option<PathBuf>,
        model: PathBuf,
        out: PathBuf,
    ) -> Self {
        let (id, source, decode) = (item.id, item.source.clone(), item.decode.clone());
        Self::spawn(ctx, label, move |progress| {
            let mut frames = media::decode_frames(&source, &decode, ffmpeg.as_deref())?;
            progress(0.02);
            let segmenter = media::ai::Segmenter::load(&model)?;
            let n = frames.len() as f32;
            for (i, frame) in frames.iter_mut().enumerate() {
                let mask = segmenter.mask(&frame.image)?;
                media::process::apply_mask(&mut frame.image, &mask);
                progress(0.02 + 0.93 * (i + 1) as f32 / n);
            }
            media::export::write_apng(&out, &frames)?;
            Ok(JobDone::NewItem { from: id, path: out })
        })
    }
}
