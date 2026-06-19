# p1mplayer

A minimal, fast desktop music player written in Rust with a [Slint](https://slint.dev) UI.
It scans your music folders, shows cover art, manages playlists and a play queue,
and integrates with your system's media controls.

## Preview

<video src="2026-06-19T11-08-10-849Z.mp4" controls width="100%">
  Your browser/viewer can't play embedded video —
  <a href="2026-06-19T11-08-10-849Z.mp4">click here to watch the preview</a>.
</video>

> ▶️ If the player above doesn't render, [watch the preview video](2026-06-19T11-08-10-849Z.mp4).

## Features

- **Library** — recursively scan one or more music folders; scanning runs in the
  background and streams results in so the UI stays responsive.
- **Cover art** — embedded album art is extracted, thumbnailed, and cached.
- **Playlists** — create playlists and add tracks via hover buttons or the
  right‑click context menu.
- **Play queue** — with shuffle and repeat (off / one / all).
- **Playback** — play/pause, next/previous, seek, and volume.
- **Search** — live filter of the current list by title, artist, or album.
- **Theming** — Gruvbox dark by default, and the UI **recolors itself from the
  cover art** of the track that's playing.
- **System integration** — exposes MPRIS (Linux) / SMTC (Windows) so the track
  shows up in your desktop's "now playing" widget and responds to media keys.
- **Robust paths** — handles non‑UTF‑8 filenames (e.g. legacy code‑page names)
  without dropping or corrupting them.
- **Persistent** — library, playlists, scanned folders, volume, and modes are
  saved between runs.

## Building & running

Requires a Rust toolchain (edition 2024).

```sh
cargo run --release
```

### Linux system dependencies

The default backends need a few system libraries at build/run time:

- ALSA / PipeWire for audio output (`libasound2-dev`)
- D‑Bus for system media controls (`libdbus-1-dev`)
- A working OpenGL stack + fontconfig for the Slint renderer

## Configuration & data

State and the cover‑thumbnail cache live under your config directory
(`~/.config/p1mplayer/` on Linux):

- `state.json` — library, playlists, scanned folders, volume, repeat/shuffle
- `covers/` — cached cover‑art thumbnails

## Desktop integration (icon)

On Wayland, the taskbar/dock icon is resolved from the window's application id
(`p1mplayer`) via an installed desktop entry — not from the in‑window icon. A
desktop file is provided in [`packaging/p1mplayer.desktop`](packaging/p1mplayer.desktop).
To make the icon show up:

```sh
install -Dm644 ui/icon.png  ~/.local/share/icons/hicolor/256x256/apps/p1mplayer.png
install -Dm644 ui/icon.svg  ~/.local/share/icons/hicolor/scalable/apps/p1mplayer.svg
install -Dm644 packaging/p1mplayer.desktop ~/.local/share/applications/p1mplayer.desktop
gtk-update-icon-cache -f -t ~/.local/share/icons/hicolor 2>/dev/null || true
```

(Reload/restart your bar afterwards so it re‑reads the icon theme.)

## Project layout

```
src/
  main.rs      — startup, window/app‑id setup, callback wiring, tick loop
  app.rs       — Controller: app state, views, playback, theming, media sync
  audio.rs     — thin wrapper around rodio playback
  library.rs   — folder scanning + tag/cover extraction
  queue.rs     — play queue (shuffle, repeat, navigation)
  model.rs     — data types + lossless path (de)serialization
  palette.rs   — derive a theme from cover‑art colors
  media.rs     — system media controls (MPRIS/SMTC) via souvlaki
  store.rs     — load/save persisted state
ui/
  app.slint    — the Slint user interface
  icon.svg/png — application icon
packaging/
  p1mplayer.desktop — desktop entry for system integration
```

## Built with

[Slint](https://slint.dev) · [rodio](https://crates.io/crates/rodio) ·
[lofty](https://crates.io/crates/lofty) · [image](https://crates.io/crates/image) ·
[souvlaki](https://crates.io/crates/souvlaki) · [walkdir](https://crates.io/crates/walkdir) ·
[rfd](https://crates.io/crates/rfd) · [serde](https://serde.rs)

## Notes

- ALSA/JACK messages like *"unable to open slave"* or *"jack server is not running"*
  on startup are harmless probing output from the system audio libraries; playback
  falls back to your default device.
- `added 0 new track(s)` on launch just means the background rescan of your saved
  folders found nothing new.

