// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sound playback.

use std::fs::File;
use std::path::Path;

use rodio::{Decoder, MixerDeviceSink, Player, Source as _};

pub struct Audio {
    /// `None` when there is no output device; sounds are then silently skipped.
    sink: Option<MixerDeviceSink>,
}

impl Audio {
    pub fn new() -> Self {
        let sink = match rodio::DeviceSinkBuilder::open_default_sink() {
            Ok(mut sink) => {
                sink.log_on_drop(false);
                Some(sink)
            }
            Err(err) => {
                log::warn!("no audio output: {err}");
                None
            }
        };
        Self { sink }
    }

    pub fn play(&self, path: &Path, volume: f32, looping: bool) -> Option<Player> {
        let sink = self.sink.as_ref()?;
        let decoder = File::open(path)
            .map_err(anyhow::Error::from)
            .and_then(|f| Decoder::try_from(f).map_err(anyhow::Error::from));
        let source = match decoder {
            Ok(source) => source,
            Err(err) => {
                log::warn!("cannot play {}: {err}", path.display());
                return None;
            }
        };
        let player = Player::connect_new(sink.mixer());
        player.set_volume(volume);
        if looping {
            player.append(source.repeat_infinite());
        } else {
            player.append(source);
        }
        Some(player)
    }
}

/// A sound attached to an overlay, restarted only when its file changes.
#[derive(Default)]
pub struct Track {
    playing: Option<(std::path::PathBuf, Player)>,
}

impl Track {
    pub fn sync(&mut self, audio: &Audio, want: Option<&Path>, volume: f32, paused: bool) {
        match (want, &self.playing) {
            (None, _) => self.playing = None,
            (Some(path), Some((current, _))) if current == path => {}
            (Some(path), _) => {
                self.playing = audio
                    .play(path, volume, true)
                    .map(|player| (path.to_path_buf(), player));
            }
        }
        if let Some((_, player)) = &self.playing {
            if (player.volume() - volume).abs() > f32::EPSILON {
                player.set_volume(volume);
            }
            if paused != player.is_paused() {
                if paused { player.pause() } else { player.play() }
            }
        }
    }
}
