use crate::audio::AudioPlayer;
use crate::media::{self, ControlEvent, Media, SeekDirection};
use crate::model::{fmt_secs, AppState, Playlist, Track};
use crate::palette::{self, ThemeColors};
use crate::queue::Queue;
use crate::{library, store};
use slint::{ModelRc, VecModel};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Duration;

slint::include_modules!();

/// Which list is currently shown in the main panel.
#[derive(Clone, Copy, PartialEq)]
enum View {
    Library,
    Queue,
    Playlist(usize),
}

pub struct Controller {
    state: AppState,
    player: Option<AudioPlayer>,
    queue: Queue,
    view: View,
    /// Library indices visible in the current view, in display order.
    visible: Vec<usize>,
    /// Whether we intend to keep playing (drives auto-advance on track end).
    playing_intent: bool,
    /// Receives streamed results from an in-progress background folder scan.
    scan_rx: Option<Receiver<ScanMsg>>,
    /// Decoded cover thumbnails, keyed by thumbnail path, so each is read once.
    cover_cache: HashMap<PathBuf, slint::Image>,
    /// Tracks added by the in-progress scan so far (drives the completion log).
    scan_added: usize,
    /// Active search filter, stored lower-cased. Empty means "show everything".
    search_query: String,
    /// Themes derived from cover art, keyed by cover path, computed at most once.
    theme_cache: HashMap<PathBuf, ThemeColors>,
    /// Cover whose theme is currently applied (`None` = base Gruvbox). Lets us skip
    /// recomputing/re-pushing the theme when the playing track hasn't changed.
    applied_theme_cover: Option<PathBuf>,
    /// System media controls (MPRIS/SMTC). `None` if the OS service is unavailable.
    media: Option<Media>,
    /// Control events (play/pause/next/…) coming from the OS, drained each tick.
    media_rx: Option<Receiver<ControlEvent>>,
    /// Library index last published to the OS as metadata (dedup guard).
    media_track: Option<usize>,
    /// Playback state last published to the OS (dedup guard).
    media_state: Option<MediaState>,
}

/// Playback state as exposed to the OS media controls (used to dedup updates).
#[derive(Clone, Copy, PartialEq)]
enum MediaState {
    Playing,
    Paused,
    Stopped,
}

/// A message streamed from the background scan thread.
enum ScanMsg {
    /// A batch of freshly scanned tracks.
    Batch(Vec<Track>),
    /// Scan finished; the folder to register as scanned (`None` for a startup refresh).
    Done(Option<PathBuf>),
}

impl Controller {
    pub fn new(state: AppState) -> Self {
        let mut player = match AudioPlayer::new() {
            Ok(p) => Some(p),
            Err(e) => {
                eprintln!("p1mplayer: audio disabled: {e}");
                None
            }
        };
        if let Some(p) = player.as_mut() {
            p.set_volume(state.volume);
        }
        let (media, media_rx) = match Media::new() {
            Some((m, rx)) => (Some(m), Some(rx)),
            None => {
                eprintln!("p1mplayer: system media controls unavailable");
                (None, None)
            }
        };
        Controller {
            state,
            player,
            queue: Queue::default(),
            view: View::Library,
            visible: Vec::new(),
            playing_intent: false,
            scan_rx: None,
            cover_cache: HashMap::new(),
            scan_added: 0,
            search_query: String::new(),
            theme_cache: HashMap::new(),
            applied_theme_cover: None,
            media,
            media_rx,
            media_track: None,
            media_state: None,
        }
    }

    // ---------- view / list building ----------

    fn compute_visible(&self) -> (Vec<usize>, String) {
        let (mut indices, title) = match self.view {
            View::Library => (
                (0..self.state.library.len()).collect::<Vec<usize>>(),
                "Library".to_string(),
            ),
            View::Queue => (Vec::new(), "Queue".to_string()),
            View::Playlist(pi) => {
                let pl = &self.state.playlists[pi];
                let indices: Vec<usize> = pl
                    .track_paths
                    .iter()
                    .filter_map(|p| self.state.index_of_path(p))
                    .collect();
                (indices, pl.name.clone())
            }
        };
        // Apply the search filter to library/playlist lists (the queue is built
        // separately and keyed by position, so it is left untouched).
        if !self.search_query.is_empty() {
            indices.retain(|&i| self.matches_query(i));
        }
        (indices, title)
    }

    /// Case-insensitive match of the active query against a track's title/artist/album.
    /// `search_query` is already lower-cased, so only the track fields are folded here.
    fn matches_query(&self, lib_index: usize) -> bool {
        if self.search_query.is_empty() {
            return true;
        }
        let q = self.search_query.as_str();
        let t = &self.state.library[lib_index];
        t.title.to_lowercase().contains(q)
            || t.artist.to_lowercase().contains(q)
            || t.album.to_lowercase().contains(q)
    }

    /// Look up a track's cover thumbnail, decoding (and caching) it at most once.
    fn cover_for(&mut self, lib_index: usize) -> (slint::Image, bool) {
        let Some(path) = self.state.library[lib_index].cover_path.clone() else {
            return (slint::Image::default(), false);
        };
        if let Some(img) = self.cover_cache.get(&path) {
            return (img.clone(), true);
        }
        if path.exists() {
            if let Ok(img) = slint::Image::load_from_path(&path) {
                self.cover_cache.insert(path, img.clone());
                return (img, true);
            }
        }
        (slint::Image::default(), false)
    }

    fn make_row(&mut self, lib_index: usize) -> TrackRow {
        let (cover, has_cover) = self.cover_for(lib_index);
        let t = &self.state.library[lib_index];
        TrackRow {
            id: lib_index as i32,
            title: t.title.clone().into(),
            artist: t.artist.clone().into(),
            album: t.album.clone().into(),
            duration: t.duration_label().into(),
            cover,
            has_cover,
        }
    }

    fn build_rows(&mut self, indices: &[usize]) -> Vec<TrackRow> {
        indices.iter().map(|&i| self.make_row(i)).collect()
    }

    /// Refresh sidebar + main list + queue list.
    pub fn refresh_lists(&mut self, ui: &MainWindow) {
        let (visible, title) = self.compute_visible();
        self.visible = visible;

        let rows = self.build_rows(&self.visible.clone());
        ui.set_tracks(ModelRc::new(VecModel::from(rows)));
        ui.set_view_title(title.into());
        ui.set_active_view(match self.view {
            View::Library => 0,
            View::Queue => 1,
            View::Playlist(_) => 2,
        });
        ui.set_selected_playlist(match self.view {
            View::Playlist(pi) => pi as i32,
            _ => -1,
        });

        // playlists sidebar
        let pls: Vec<PlaylistRow> = self
            .state
            .playlists
            .iter()
            .enumerate()
            .map(|(i, p)| PlaylistRow {
                id: i as i32,
                name: p.name.clone().into(),
                count: p.track_paths.len() as i32,
            })
            .collect();
        ui.set_playlists(ModelRc::new(VecModel::from(pls)));

        self.refresh_queue(ui);
        self.refresh_now_playing(ui);
    }

    fn refresh_queue(&mut self, ui: &MainWindow) {
        let order: Vec<usize> = self.queue.order().to_vec();
        let rows = self.build_rows(&order);
        ui.set_queue(ModelRc::new(VecModel::from(rows)));
        ui.set_current_queue_pos(self.queue.current_pos().map(|p| p as i32).unwrap_or(-1));
    }

    pub fn refresh_now_playing(&mut self, ui: &MainWindow) {
        match self.queue.current_track() {
            Some(lib) => {
                let (cover, has_cover) = self.cover_for(lib);
                let t = &self.state.library[lib];
                ui.set_now_title(t.title.clone().into());
                ui.set_now_artist(t.artist.clone().into());
                ui.set_now_cover(cover);
                ui.set_now_has_cover(has_cover);
                ui.set_current_track_id(lib as i32);
            }
            None => {
                ui.set_now_title("Nothing playing".into());
                ui.set_now_artist("".into());
                ui.set_now_has_cover(false);
                ui.set_current_track_id(-1);
            }
        }
        ui.set_playing(self.player.as_ref().map(|p| p.is_playing()).unwrap_or(false));
        ui.set_repeat_mode(self.state.repeat.as_i32());
        ui.set_shuffle(self.state.shuffle);
        ui.set_volume(self.state.volume);
        ui.set_current_queue_pos(self.queue.current_pos().map(|p| p as i32).unwrap_or(-1));

        self.update_theme(ui);
        self.update_media();
    }

    /// Sync the OS media controls (MPRIS/SMTC) with the currently playing track and
    /// playback state. Deduped so it only emits D-Bus updates on real changes.
    fn update_media(&mut self) {
        if self.media.is_none() {
            return;
        }
        let cur = self.queue.current_track();
        if cur != self.media_track {
            self.media_track = cur;
            self.media_push_metadata();
        }
        let kind = if cur.is_none() {
            MediaState::Stopped
        } else if self.player.as_ref().map_or(false, |p| p.is_playing()) {
            MediaState::Playing
        } else {
            MediaState::Paused
        };
        if Some(kind) != self.media_state {
            self.media_state = Some(kind);
            self.media_push_playback();
        }
    }

    /// Publish the current track's metadata to the OS media controls.
    fn media_push_metadata(&mut self) {
        let Some(lib) = self.queue.current_track() else {
            return;
        };
        let (title, artist, album, cover, dur) = {
            let t = &self.state.library[lib];
            (
                t.title.clone(),
                t.artist.clone(),
                t.album.clone(),
                t.cover_path.clone(),
                Duration::from_secs(t.duration_secs),
            )
        };
        let cover_url = cover.as_deref().and_then(media::file_uri);
        if let Some(m) = self.media.as_mut() {
            m.set_metadata(&title, &artist, &album, cover_url.as_deref(), dur);
        }
    }

    /// Publish the current playback state + position to the OS media controls.
    fn media_push_playback(&mut self) {
        let has_track = self.queue.current_track().is_some();
        let pos = self.player.as_ref().map(|p| p.position()).unwrap_or_default();
        let playing = self.player.as_ref().map(|p| p.is_playing()).unwrap_or(false);
        if let Some(m) = self.media.as_mut() {
            if !has_track {
                m.set_stopped();
            } else if playing {
                m.set_playing(pos);
            } else {
                m.set_paused(pos);
            }
        }
    }

    /// Drain control events sent by the OS media controls and act on them.
    fn poll_media(&mut self, ui: &MainWindow) {
        let mut events = Vec::new();
        if let Some(rx) = self.media_rx.as_ref() {
            while let Ok(e) = rx.try_recv() {
                events.push(e);
            }
        }
        for e in events {
            match e {
                ControlEvent::Play => {
                    if self.player.as_ref().map_or(false, |p| !p.is_playing()) {
                        self.play_pause(ui);
                    }
                }
                ControlEvent::Pause => {
                    if self.player.as_ref().map_or(false, |p| p.is_playing()) {
                        self.play_pause(ui);
                    }
                }
                ControlEvent::Toggle => self.play_pause(ui),
                ControlEvent::Next => self.next_track(ui),
                ControlEvent::Previous => self.prev_track(ui),
                ControlEvent::Stop => {
                    if let Some(p) = self.player.as_mut() {
                        p.stop();
                    }
                    self.playing_intent = false;
                    self.refresh_now_playing(ui);
                }
                ControlEvent::SetPosition(pos) => self.seek_to_position(pos.0, ui),
                ControlEvent::SeekBy(dir, dur) => {
                    let secs = dur.as_secs() as i64;
                    self.seek_by(seek_delta(dir, secs), ui);
                }
                ControlEvent::Seek(dir) => self.seek_by(seek_delta(dir, 10), ui),
                _ => {} // SetVolume / OpenUri / Raise / Quit: not handled
            }
        }
    }

    fn seek_by(&mut self, secs: i64, ui: &MainWindow) {
        if let Some(p) = self.player.as_ref() {
            p.seek_relative(secs);
        }
        self.refresh_progress(ui);
        self.media_push_playback();
    }

    fn seek_to_position(&mut self, pos: Duration, ui: &MainWindow) {
        if let Some(p) = self.player.as_ref() {
            let total = p.duration().as_secs_f32();
            if total > 0.0 {
                p.seek_fraction(pos.as_secs_f32() / total);
            }
        }
        self.refresh_progress(ui);
        self.media_push_playback();
    }

    /// Recolor the whole UI from the cover of the track that is currently playing,
    /// falling back to the base Gruvbox palette for cover-less tracks (or when
    /// nothing is playing). Cheap to call repeatedly: it no-ops unless the active
    /// cover actually changed, and decodes each cover's palette at most once.
    fn update_theme(&mut self, ui: &MainWindow) {
        let cover = self
            .queue
            .current_track()
            .and_then(|i| self.state.library.get(i))
            .and_then(|t| t.cover_path.clone())
            .filter(|p| p.exists());

        if cover == self.applied_theme_cover {
            return;
        }
        self.applied_theme_cover = cover.clone();

        let colors = match cover {
            Some(path) => self
                .theme_cache
                .entry(path.clone())
                .or_insert_with(|| palette::from_cover(&path).unwrap_or_else(ThemeColors::gruvbox))
                .clone(),
            None => ThemeColors::gruvbox(),
        };

        push_theme(ui, &colors);
    }

    pub fn refresh_progress(&self, ui: &MainWindow) {
        if let Some(p) = self.player.as_ref() {
            let pos = p.position();
            let dur = p.duration();
            let frac = if dur.as_secs_f32() > 0.0 {
                (pos.as_secs_f32() / dur.as_secs_f32()).clamp(0.0, 1.0)
            } else {
                0.0
            };
            ui.set_progress(frac);
            ui.set_position_text(fmt_secs(pos.as_secs()).into());
            ui.set_duration_text(fmt_secs(dur.as_secs()).into());
            ui.set_playing(p.is_playing());
        }
    }

    // ---------- playback ----------

    fn start_track(&mut self, lib_index: usize, ui: &MainWindow) {
        let (path, dur) = {
            let t = &self.state.library[lib_index];
            (t.path.clone(), t.duration_secs)
        };
        if let Some(p) = self.player.as_mut() {
            match p.play_file(&path, dur) {
                Ok(()) => self.playing_intent = true,
                Err(e) => {
                    eprintln!("p1mplayer: {e}");
                    self.playing_intent = false;
                }
            }
        }
        self.refresh_queue(ui);
        self.refresh_now_playing(ui);
        self.refresh_progress(ui);
    }

    /// Play `lib_index` from the current view, building a queue from the visible list.
    pub fn play_track(&mut self, lib_index: usize, ui: &MainWindow) {
        let start = self.visible.iter().position(|&x| x == lib_index).unwrap_or(0);
        self.queue.set_tracks(self.visible.clone(), start);
        self.queue.set_shuffle(self.state.shuffle);
        if let Some(lib) = self.queue.current_track() {
            self.start_track(lib, ui);
        }
    }

    pub fn queue_track(&mut self, lib_index: usize, ui: &MainWindow) {
        let was_empty = self.queue.is_empty();
        self.queue.push(lib_index);
        if was_empty {
            if let Some(lib) = self.queue.current_track() {
                self.start_track(lib, ui);
            }
        }
        self.refresh_queue(ui);
    }

    pub fn play_queue_pos(&mut self, pos: usize, ui: &MainWindow) {
        if let Some(lib) = self.queue.jump_to(pos) {
            self.start_track(lib, ui);
        }
    }

    pub fn remove_queue_pos(&mut self, pos: usize, ui: &MainWindow) {
        self.queue.remove(pos);
        self.refresh_queue(ui);
        self.refresh_now_playing(ui);
    }

    pub fn play_pause(&mut self, ui: &MainWindow) {
        if let Some(p) = self.player.as_ref() {
            p.toggle();
            self.playing_intent = p.is_playing();
        }
        self.refresh_now_playing(ui);
    }

    pub fn next_track(&mut self, ui: &MainWindow) {
        let next = self.queue.next(self.state.repeat);
        match next {
            Some(lib) => self.start_track(lib, ui),
            None => {
                self.playing_intent = false;
                self.refresh_now_playing(ui);
            }
        }
    }

    pub fn prev_track(&mut self, ui: &MainWindow) {
        // If we're more than 3s into the track, restart it instead of going back.
        if let Some(p) = self.player.as_ref() {
            if p.position().as_secs() > 3 {
                p.seek_fraction(0.0);
                self.refresh_progress(ui);
                return;
            }
        }
        if let Some(lib) = self.queue.prev(self.state.repeat) {
            self.start_track(lib, ui);
        }
    }

    pub fn seek_forward(&mut self, ui: &MainWindow) {
        if let Some(p) = self.player.as_ref() {
            p.seek_relative(10);
        }
        self.refresh_progress(ui);
        self.media_push_playback();
    }

    pub fn seek_back(&mut self, ui: &MainWindow) {
        if let Some(p) = self.player.as_ref() {
            p.seek_relative(-10);
        }
        self.refresh_progress(ui);
        self.media_push_playback();
    }

    pub fn seek_to(&mut self, frac: f32, ui: &MainWindow) {
        if let Some(p) = self.player.as_ref() {
            p.seek_fraction(frac);
        }
        self.refresh_progress(ui);
        self.media_push_playback();
    }

    /// Called from the UI timer: advance automatically when a track ends and
    /// integrate the results of any finished background scan.
    pub fn tick(&mut self, ui: &MainWindow) {
        self.poll_scan(ui);
        self.poll_media(ui);

        let finished = self
            .player
            .as_ref()
            .map(|p| p.track_finished())
            .unwrap_or(false);
        if self.playing_intent && finished {
            self.next_track(ui);
        } else {
            self.refresh_progress(ui);
        }
    }

    /// Drain whatever the background scan thread has delivered since the last tick
    /// and merge it in. Batches are applied progressively so the library populates
    /// as files are discovered, instead of waiting for the whole scan to finish.
    fn poll_scan(&mut self, ui: &MainWindow) {
        // Take the receiver out so we can mutate `self` freely while draining; it is
        // put back below unless the scan has finished or the thread went away.
        let Some(rx) = self.scan_rx.take() else {
            return;
        };

        let mut new_tracks: Vec<Track> = Vec::new();
        let mut done: Option<Option<PathBuf>> = None;
        let mut disconnected = false;

        loop {
            match rx.try_recv() {
                Ok(ScanMsg::Batch(tracks)) => new_tracks.extend(tracks),
                Ok(ScanMsg::Done(dir)) => {
                    done = Some(dir);
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break, // still scanning
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }

        // Keep listening only if the scan is still running.
        if done.is_none() && !disconnected {
            self.scan_rx = Some(rx);
        }

        // Merge any freshly scanned tracks, skipping ones we already know about.
        // We append (cheap) during streaming and sort only once the scan finishes,
        // so a large scan doesn't re-sort the whole library on every tick.
        let mut changed = false;
        for t in new_tracks {
            if self.state.index_of_path(&t.path).is_none() {
                self.state.library.push(t);
                self.scan_added += 1;
                changed = true;
            }
        }

        if let Some(dir) = done {
            ui.set_scanning(false);
            if let Some(dir) = dir {
                if !self.state.scanned_dirs.contains(&dir) {
                    self.state.scanned_dirs.push(dir);
                }
            }
            // Sort into final order, surface the library, and persist on completion.
            self.sort_library();
            self.view = View::Library;
            self.refresh_lists(ui);
            eprintln!("p1mplayer: added {} new track(s)", self.scan_added);
            self.scan_added = 0;
            self.save();
        } else {
            // Still scanning: show what's arrived so far (appended, sorted at the end).
            if changed {
                self.refresh_lists(ui);
            }
            if disconnected {
                ui.set_scanning(false);
                self.scan_added = 0;
            }
        }
    }

    // ---------- modes / volume ----------

    pub fn toggle_shuffle(&mut self, ui: &MainWindow) {
        self.state.shuffle = !self.state.shuffle;
        self.queue.set_shuffle(self.state.shuffle);
        self.refresh_queue(ui);
        self.refresh_now_playing(ui);
        self.save();
    }

    pub fn cycle_repeat(&mut self, ui: &MainWindow) {
        self.state.repeat = self.state.repeat.cycle();
        self.refresh_now_playing(ui);
        self.save();
    }

    pub fn set_volume(&mut self, v: f32, ui: &MainWindow) {
        self.state.volume = v.clamp(0.0, 1.0);
        if let Some(p) = self.player.as_mut() {
            p.set_volume(self.state.volume);
        }
        ui.set_volume(self.state.volume);
    }

    // ---------- views ----------

    /// Live search from the toolbar box. Filters the current list as the user types.
    pub fn search(&mut self, query: String, ui: &MainWindow) {
        self.search_query = query.trim().to_lowercase();
        self.refresh_lists(ui);
    }

    /// Clear any active search and reset the search box in the UI.
    fn clear_search(&mut self, ui: &MainWindow) {
        self.search_query.clear();
        ui.set_search_text(slint::SharedString::new());
    }

    pub fn show_library(&mut self, ui: &MainWindow) {
        self.clear_search(ui);
        self.view = View::Library;
        self.refresh_lists(ui);
    }

    pub fn show_queue(&mut self, ui: &MainWindow) {
        self.clear_search(ui);
        self.view = View::Queue;
        self.refresh_lists(ui);
    }

    pub fn select_playlist(&mut self, pi: usize, ui: &MainWindow) {
        if pi < self.state.playlists.len() {
            self.clear_search(ui);
            self.view = View::Playlist(pi);
            self.refresh_lists(ui);
        }
    }

    // ---------- library / playlist mutations ----------

    pub fn add_folder(&mut self, ui: &MainWindow) {
        if self.scan_rx.is_some() {
            return; // a scan is already running
        }
        let Some(dir) = rfd::FileDialog::new().set_title("Add music folder").pick_folder() else {
            return;
        };

        // Scan on a background thread so the UI stays responsive for large libraries.
        // Tracks are streamed back in batches and merged by `poll_scan` on each tick.
        let covers = store::covers_dir();
        let (tx, rx) = std::sync::mpsc::channel();
        let scan_dir = dir.clone();
        std::thread::spawn(move || {
            library::scan_stream(std::slice::from_ref(&scan_dir), &covers, |batch| {
                let _ = tx.send(ScanMsg::Batch(batch));
            });
            let _ = tx.send(ScanMsg::Done(Some(scan_dir)));
        });
        self.scan_added = 0;
        self.scan_rx = Some(rx);
        ui.set_scanning(true);
    }

    /// Kick off a background re-scan of all previously added folders so files added
    /// or changed on disk since last launch are picked up — without blocking startup.
    /// The persisted library is already shown instantly; this just refreshes it.
    pub fn start_startup_rescan(&mut self, ui: &MainWindow) {
        if self.scan_rx.is_some() || self.state.scanned_dirs.is_empty() {
            return;
        }
        // Only scan folders that still exist on disk.
        let dirs: Vec<PathBuf> = self
            .state
            .scanned_dirs
            .iter()
            .filter(|d| d.exists())
            .cloned()
            .collect();
        if dirs.is_empty() {
            return;
        }
        let covers = store::covers_dir();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            library::scan_stream(&dirs, &covers, |batch| {
                let _ = tx.send(ScanMsg::Batch(batch));
            });
            let _ = tx.send(ScanMsg::Done(None));
        });
        self.scan_added = 0;
        self.scan_rx = Some(rx);
        ui.set_scanning(true);
    }

    fn sort_library(&mut self) {
        self.state.library.sort_by(|a, b| {
            a.artist
                .to_lowercase()
                .cmp(&b.artist.to_lowercase())
                .then(a.album.to_lowercase().cmp(&b.album.to_lowercase()))
                .then(a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        });
    }

    pub fn new_playlist(&mut self, name: String, ui: &MainWindow) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        self.state.playlists.push(Playlist {
            name: name.to_string(),
            track_paths: Vec::new(),
        });
        self.view = View::Playlist(self.state.playlists.len() - 1);
        self.refresh_lists(ui);
        self.save();
    }

    /// Add a track to the active playlist (or the most sensible fallback).
    pub fn add_track_to_playlist(&mut self, lib_index: usize, ui: &MainWindow) {
        let target = match self.view {
            View::Playlist(pi) => pi,
            _ => {
                if self.state.playlists.is_empty() {
                    self.state.playlists.push(Playlist {
                        name: "My Playlist".to_string(),
                        track_paths: Vec::new(),
                    });
                }
                self.state.playlists.len() - 1
            }
        };
        let path = self.state.library[lib_index].path.clone();
        let pl = &mut self.state.playlists[target];
        if !pl.track_paths.contains(&path) {
            pl.track_paths.push(path);
        }
        self.refresh_lists(ui);
        self.save();
    }

    /// Add a track to a specific playlist chosen from the context menu.
    pub fn add_track_to_playlist_id(
        &mut self,
        lib_index: usize,
        playlist_index: usize,
        ui: &MainWindow,
    ) {
        if lib_index >= self.state.library.len() || playlist_index >= self.state.playlists.len() {
            return;
        }
        let path = self.state.library[lib_index].path.clone();
        let pl = &mut self.state.playlists[playlist_index];
        if !pl.track_paths.contains(&path) {
            pl.track_paths.push(path);
        }
        self.refresh_lists(ui);
        self.save();
    }

    /// Remove a track (by library index) from the play queue, if present.
    pub fn remove_track_from_queue(&mut self, lib_index: usize, ui: &MainWindow) {
        self.queue.remove_library_index(lib_index);
        self.refresh_queue(ui);
        self.refresh_now_playing(ui);
    }

    /// Permanently delete a track's file from disk after confirmation, then forget
    /// it everywhere (library, playlists, queue) and stop it if it was playing.
    pub fn delete_track_file(&mut self, lib_index: usize, ui: &MainWindow) {
        if lib_index >= self.state.library.len() {
            return;
        }
        let path = self.state.library[lib_index].path.clone();

        // Destructive and irreversible — require explicit confirmation.
        let confirm = rfd::MessageDialog::new()
            .set_title("Delete file")
            .set_level(rfd::MessageLevel::Warning)
            .set_description(format!(
                "Permanently delete this file from disk?\n\n{}\n\nThis cannot be undone.",
                path.display()
            ))
            .set_buttons(rfd::MessageButtons::YesNo)
            .show();
        if confirm != rfd::MessageDialogResult::Yes {
            return;
        }

        if let Err(e) = std::fs::remove_file(&path) {
            eprintln!("p1mplayer: failed to delete {}: {e}", path.display());
            rfd::MessageDialog::new()
                .set_title("Delete failed")
                .set_level(rfd::MessageLevel::Error)
                .set_description(format!("Could not delete the file:\n{e}"))
                .set_buttons(rfd::MessageButtons::Ok)
                .show();
            return;
        }

        // If the deleted track is the one playing, stop so we don't keep playing a
        // file that no longer exists.
        if self.queue.current_track() == Some(lib_index) {
            if let Some(p) = self.player.as_mut() {
                p.stop();
            }
            self.playing_intent = false;
        }

        // Forget the track everywhere. Removing it from the library shifts every
        // later index down by one, so fix up the queue's stored indices to match.
        for pl in &mut self.state.playlists {
            pl.track_paths.retain(|p| p != &path);
        }
        self.state.library.remove(lib_index);
        self.queue.remove_library_index(lib_index);

        self.refresh_lists(ui);
        self.save();
    }

    pub fn save(&self) {
        store::save(&self.state);
    }
}

/// Translate a media-control seek direction + magnitude (seconds) into a signed offset.
fn seek_delta(dir: SeekDirection, secs: i64) -> i64 {
    match dir {
        SeekDirection::Forward => secs,
        SeekDirection::Backward => -secs,
    }
}

/// Write a full set of colours into the Slint `Theme` global, recoloring the UI.
fn push_theme(ui: &MainWindow, c: &ThemeColors) {
    let col = |(r, g, b): (u8, u8, u8)| slint::Color::from_rgb_u8(r, g, b);
    let theme = ui.global::<Theme>();
    theme.set_bg(col(c.bg));
    theme.set_bg_alt(col(c.bg_alt));
    theme.set_sidebar(col(c.sidebar));
    theme.set_card(col(c.card));
    theme.set_accent(col(c.accent));
    theme.set_accent_soft(col(c.accent_soft));
    theme.set_text(col(c.text));
    theme.set_text_dim(col(c.text_dim));
    theme.set_hover(col(c.hover));
}

