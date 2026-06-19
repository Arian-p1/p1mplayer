//! System "now playing" integration via the OS media controls — MPRIS over D-Bus
//! on Linux (what desktop panels, lock screens and media keys read), plus SMTC on
//! Windows and the Now Playing center on macOS — using the `souvlaki` crate.
//!
//! The OS delivers control events (play/pause/next/…) on a background D-Bus
//! thread. We forward them through a channel that the UI thread drains on its
//! regular tick, so the rest of the app stays single-threaded.

use souvlaki::{MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig};
use std::path::Path;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

pub use souvlaki::{MediaControlEvent as ControlEvent, SeekDirection};

/// Handle to the OS media controls. Dropping it tears down the D-Bus service.
pub struct Media {
    controls: MediaControls,
}

impl Media {
    /// Register with the OS media controls, returning the handle plus a receiver
    /// of control events emitted by the system. `None` if the media service is
    /// unavailable (e.g. no D-Bus session bus), in which case the app just runs
    /// without system integration.
    pub fn new() -> Option<(Media, Receiver<ControlEvent>)> {
        let config = PlatformConfig {
            display_name: "p1mplayer",
            dbus_name: "p1mplayer",
            hwnd: None, // only used on Windows
        };
        let mut controls = MediaControls::new(config).ok()?;
        let (tx, rx) = channel();
        controls
            .attach(move |event| {
                let _ = tx.send(event);
            })
            .ok()?;
        Some((Media { controls }, rx))
    }

    /// Publish the current track's metadata (shown on the system "now playing" UI).
    pub fn set_metadata(
        &mut self,
        title: &str,
        artist: &str,
        album: &str,
        cover_url: Option<&str>,
        duration: Duration,
    ) {
        let _ = self.controls.set_metadata(MediaMetadata {
            title: Some(title),
            artist: Some(artist),
            album: (!album.is_empty()).then_some(album),
            cover_url,
            duration: Some(duration),
        });
    }

    pub fn set_playing(&mut self, position: Duration) {
        let _ = self.controls.set_playback(MediaPlayback::Playing {
            progress: Some(MediaPosition(position)),
        });
    }

    pub fn set_paused(&mut self, position: Duration) {
        let _ = self.controls.set_playback(MediaPlayback::Paused {
            progress: Some(MediaPosition(position)),
        });
    }

    pub fn set_stopped(&mut self) {
        let _ = self.controls.set_playback(MediaPlayback::Stopped);
    }
}

/// Build a percent-encoded `file://` URI for a local cover image, as MPRIS expects.
/// Returns `None` for non-UTF-8 paths (MPRIS requires a valid UTF-8 string).
pub fn file_uri(path: &Path) -> Option<String> {
    let s = path.to_str()?;
    let mut out = String::from("file://");
    for &b in s.as_bytes() {
        match b {
            b'/' | b'-' | b'_' | b'.' | b'~' | b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    Some(out)
}
