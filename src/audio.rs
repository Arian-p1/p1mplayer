use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player};
use std::path::Path;
use std::time::Duration;

/// Thin wrapper around rodio's `Player` providing the controls the UI needs.
pub struct AudioPlayer {
    // Keep the output stream alive for as long as the player exists.
    _stream: MixerDeviceSink,
    player: Player,
    duration: Duration,
    has_track: bool,
    volume: f32,
}

impl AudioPlayer {
    pub fn new() -> Result<Self, String> {
        let mut stream = DeviceSinkBuilder::open_default_sink()
            .map_err(|e| format!("audio device error: {e}"))?;
        stream.log_on_drop(false); // silence the cosmetic shutdown warning
        let player = Player::connect_new(stream.mixer());
        Ok(AudioPlayer {
            _stream: stream,
            player,
            duration: Duration::ZERO,
            has_track: false,
            volume: 1.0,
        })
    }

    /// Replace whatever is playing with `path`, starting immediately.
    pub fn play_file(&mut self, path: &Path, duration_secs: u64) -> Result<(), String> {
        let file = std::fs::File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
        let decoder = Decoder::try_from(file).map_err(|e| format!("decode {}: {e}", path.display()))?;
        self.player.clear();
        self.player.append(decoder);
        // `clear()` pauses; make sure we are playing and at the right volume.
        self.apply_volume();
        self.player.play();
        self.duration = Duration::from_secs(duration_secs);
        self.has_track = true;
        Ok(())
    }

    pub fn toggle(&self) {
        if self.player.is_paused() {
            self.player.play();
        } else {
            self.player.pause();
        }
    }

    /// Stop playback entirely and forget the current track. Used when the playing
    /// file is deleted from disk.
    pub fn stop(&mut self) {
        self.player.clear();
        self.has_track = false;
        self.duration = Duration::ZERO;
    }

    pub fn is_playing(&self) -> bool {
        self.has_track && !self.player.is_paused() && !self.player.empty()
    }

    /// True once the current track has finished on its own.
    pub fn track_finished(&self) -> bool {
        self.has_track && self.player.empty()
    }

    pub fn set_volume(&mut self, v: f32) {
        self.volume = v.clamp(0.0, 1.0);
        self.apply_volume();
    }

    fn apply_volume(&self) {
        // Perceptual-ish curve so the low end of the slider is usable.
        self.player.set_volume(self.volume * self.volume);
    }

    /// Seek by a relative offset in seconds (positive = forward), clamped to the track.
    pub fn seek_relative(&self, secs: i64) {
        let cur = self.player.get_pos();
        let target = if secs >= 0 {
            cur + Duration::from_secs(secs as u64)
        } else {
            cur.saturating_sub(Duration::from_secs((-secs) as u64))
        };
        let target = target.min(self.duration);
        let _ = self.player.try_seek(target);
    }

    /// Seek to an absolute fraction (0..1) of the track.
    pub fn seek_fraction(&self, frac: f32) {
        let secs = self.duration.as_secs_f32() * frac.clamp(0.0, 1.0);
        let _ = self.player.try_seek(Duration::from_secs_f32(secs));
    }

    pub fn position(&self) -> Duration {
        self.player.get_pos().min(self.duration)
    }

    pub fn duration(&self) -> Duration {
        self.duration
    }
}
