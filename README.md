# Open Anima

Put animations on your desktop. Open Anima shows GIFs, APNGs, animated WebPs, sprite sheets,
still images and videos in borderless, transparent windows that float over everything else.
You can drag them around, make them click-through, add sound, cut out their background, and
drive them with small programs ("behaviors") that make characters walk around your screen.

Open Anima is free software written in Rust, licensed under the
[GNU AGPL v3 or later](LICENSE).

> Open Anima is an independent project. It is not affiliated with, or endorsed by, the authors
> of any commercial desktop animation software.

## Features

- **Formats**: GIF, APNG, animated WebP, PNG/JPEG/BMP/TGA/TIFF/QOI, sprite sheets (any grid),
  and MP4/WebM/MOV/MKV/AVI videos through `ffmpeg` (alpha is kept for VP9 WebM).
- **Desktop overlays**: real per-pixel transparency, always-on-top, click-through, drag with
  the mouse, size, opacity, playback speed, forced frame rate, pause. Clicks pass through the
  transparent parts of an animation.
- **Sound**: attach an audio file (MP3, WAV, OGG, FLAC, …) to any animation, or play the sound
  track of a video. Per-animation and master volume.
- **Editing**: mirror, remove a color (green screen) with tolerance and softness, **AI
  background removal** (U²-Net family ONNX models, runs locally), export the result as an
  animated PNG.
- **Behaviors**: packages (`.oapack`) that run a program which tells Open Anima what to show
  and where. Any language works: the program just prints JSON lines. The file-based protocol
  used by other desktop animation tools (`temp/actions.mddo` + `activador.act`) is also
  understood, so existing packs can be run by loading their folder. See
  [docs/behaviors.md](docs/behaviors.md).
- **Desktop integration**: tray icon (show the panel, hide all animations, quit), start with
  Windows, drag & drop files onto the panel, English and French interface.

Everything you set up is saved and restored at the next launch, including running behaviors.

## Install

Build from source with a recent stable Rust (1.94+):

```bash
cargo build --release
```

The executable is `target/release/open-anima.exe`. Optional runtime tools:

- [`ffmpeg`](https://ffmpeg.org/) (with `ffprobe`) in `PATH`, or chosen in Settings, to play
  videos.
- An ONNX segmentation model for AI background removal, chosen in Settings. `u2netp.onnx`
  (4.7 MB, fast) from the [rembg model releases](https://github.com/danielgatis/rembg/releases)
  works well; `u2net.onnx`, `silueta.onnx` and `isnet-general-use.onnx` work too.

To build without the AI feature (smaller and faster to compile):

```bash
cargo build --release --no-default-features
```

## Usage

1. Click **Add…** or drop files on the window. Each file becomes an animation of your library
   and appears on the desktop.
2. Select it to change its size, opacity, speed, sound, sprite sheet layout or background.
   Double-click an animation on the desktop to jump to its settings.
3. Closing the window keeps Open Anima running in the notification area; use the tray icon to
   reopen it, hide every animation at once or quit.

Command line: `open-anima --minimized` starts with the panel hidden (used by "Start with
Windows").

## Platforms

Windows 10 and 11 are the primary target; overlays are native layered windows there. On other
platforms overlays are transparent egui windows: the code builds, but it hasn't been tested and
the tray icon needs GTK on Linux.

## Development

```bash
cargo test
cargo run
```

Source layout:

| Path | What |
| --- | --- |
| `src/app.rs` | Application state, sync between the library, overlays and behaviors |
| `src/ui.rs` | Control panel |
| `src/overlay/` | Desktop windows (`win.rs`: native Windows backend) |
| `src/media/` | Decoding, video through ffmpeg, edits, AI, APNG export |
| `src/behavior.rs` | Behavior packages and their protocols |
| `src/config.rs` | Saved settings |
| `examples/behaviors/` | Example behaviors for both protocols |

## License

Copyright (C) 2026 Open Anima contributors.

This program is free software: you can redistribute it and/or modify it under the terms of the
GNU Affero General Public License as published by the Free Software Foundation, either version
3 of the License, or (at your option) any later version. See [LICENSE](LICENSE).
