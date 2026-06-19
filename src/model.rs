use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Serde helpers that persist filesystem paths losslessly, even when a path is not
/// valid UTF-8 (e.g. a Persian filename stored in a legacy code page). Valid UTF-8
/// paths serialize as a plain string (readable, and backward compatible with the
/// previous format); anything else falls back to the exact encoded bytes.
mod os_path {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    #[derive(Serialize, Deserialize)]
    #[serde(untagged)]
    enum Repr {
        Text(String),
        Bytes(Vec<u8>),
    }

    fn to_repr(p: &Path) -> Repr {
        match p.to_str() {
            Some(s) => Repr::Text(s.to_owned()),
            None => Repr::Bytes(p.as_os_str().as_encoded_bytes().to_vec()),
        }
    }

    fn from_repr(r: Repr) -> PathBuf {
        match r {
            Repr::Text(s) => PathBuf::from(s),
            // SAFETY: the bytes were produced by `OsStr::as_encoded_bytes` (in `to_repr`,
            // possibly during a previous run on this platform), which is exactly the
            // contract `from_encoded_bytes_unchecked` requires.
            Repr::Bytes(b) => {
                PathBuf::from(unsafe { OsString::from_encoded_bytes_unchecked(b) })
            }
        }
    }

    /// For a plain `PathBuf` field.
    pub mod required {
        use super::*;
        pub fn serialize<S: Serializer>(p: &Path, s: S) -> Result<S::Ok, S::Error> {
            to_repr(p).serialize(s)
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<PathBuf, D::Error> {
            Ok(from_repr(Repr::deserialize(d)?))
        }
    }

    /// For an `Option<PathBuf>` field.
    pub mod optional {
        use super::*;
        pub fn serialize<S: Serializer>(p: &Option<PathBuf>, s: S) -> Result<S::Ok, S::Error> {
            p.as_ref().map(|p| to_repr(p)).serialize(s)
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<PathBuf>, D::Error> {
            Ok(Option::<Repr>::deserialize(d)?.map(from_repr))
        }
    }

    /// For a `Vec<PathBuf>` field.
    pub mod vec {
        use super::*;
        pub fn serialize<S: Serializer>(v: &[PathBuf], s: S) -> Result<S::Ok, S::Error> {
            v.iter().map(|p| to_repr(p)).collect::<Vec<_>>().serialize(s)
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<PathBuf>, D::Error> {
            Ok(Vec::<Repr>::deserialize(d)?.into_iter().map(from_repr).collect())
        }
    }
}

/// A single audio track. Paths only — we never copy the underlying audio file.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Track {
    #[serde(with = "os_path::required")]
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_secs: u64,
    /// Path to a cached cover image extracted from the file's tags (if any).
    #[serde(with = "os_path::optional", default)]
    pub cover_path: Option<PathBuf>,
}

impl Track {
    /// Human friendly mm:ss duration.
    pub fn duration_label(&self) -> String {
        fmt_secs(self.duration_secs)
    }

    /// Identity used to detect *content* duplicates: the same song that happens to
    /// exist as two separate files (e.g. copied into two folders). Normalised so
    /// case/whitespace differences don't defeat the match. This is intentionally
    /// distinct from file identity (`canonical_key`): two byte-identical copies in
    /// different locations are different files but the same song.
    pub fn content_key(&self) -> (String, String, String, u64) {
        (
            self.title.trim().to_lowercase(),
            self.artist.trim().to_lowercase(),
            self.album.trim().to_lowercase(),
            self.duration_secs,
        )
    }
}

/// A stable identity for a file on disk. `canonicalize` resolves symlinks, `.`/`..`,
/// and duplicate scan roots to a single real path, so the same underlying file is
/// recognised as one entry regardless of which path it was reached through. Falls
/// back to the path as-is when it can't be resolved (e.g. the file no longer exists),
/// which still dedups byte-identical paths.
pub fn canonical_key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Format a number of seconds as `m:ss` (or `h:mm:ss` for long tracks).
pub fn fmt_secs(total: u64) -> String {
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepeatMode {
    Off,
    One,
    All,
}

impl RepeatMode {
    pub fn cycle(self) -> Self {
        match self {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        }
    }
    pub fn as_i32(self) -> i32 {
        match self {
            RepeatMode::Off => 0,
            RepeatMode::One => 1,
            RepeatMode::All => 2,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Playlist {
    pub name: String,
    #[serde(with = "os_path::vec", default)]
    pub track_paths: Vec<PathBuf>,
}

/// The whole persisted application state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppState {
    pub library: Vec<Track>,
    pub playlists: Vec<Playlist>,
    #[serde(with = "os_path::vec", default)]
    pub scanned_dirs: Vec<PathBuf>,
    pub volume: f32,
    pub repeat: RepeatMode,
    pub shuffle: bool,
    /// When true, the library view collapses content duplicates (the same song
    /// existing as more than one file), showing each song only once.
    #[serde(default = "default_hide_duplicates")]
    pub hide_duplicates: bool,
}

/// Hide content duplicates by default: a freshly scanned library that contains the
/// same song twice should look tidy out of the box. Users can toggle it off.
fn default_hide_duplicates() -> bool {
    true
}

impl Default for AppState {
    fn default() -> Self {
        AppState {
            library: Vec::new(),
            playlists: Vec::new(),
            scanned_dirs: Vec::new(),
            volume: 1.0,
            repeat: RepeatMode::Off,
            shuffle: false,
            hide_duplicates: default_hide_duplicates(),
        }
    }
}

impl AppState {
    /// Find the library index of a track by path.
    pub fn index_of_path(&self, path: &std::path::Path) -> Option<usize> {
        self.library.iter().position(|t| t.path == path)
    }

    /// Drop library entries that resolve to the *same file on disk*. These are not
    /// real duplicates the user created — they come from scanning a file through more
    /// than one path (a followed symlink, overlapping scan folders, or `.`/`..` in a
    /// path). The first entry in each group is kept, preserving order. Returns how
    /// many entries were removed.
    pub fn dedup_library_files(&mut self) -> usize {
        let before = self.library.len();
        let mut seen = std::collections::HashSet::new();
        self.library.retain(|t| seen.insert(canonical_key(&t.path)));
        before - self.library.len()
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn sample_track(path: PathBuf) -> Track {
        Track {
            path,
            title: "t".into(),
            artist: "a".into(),
            album: "al".into(),
            duration_secs: 1,
            cover_path: None,
        }
    }

    #[test]
    fn utf8_path_round_trips_as_string() {
        let t = sample_track(PathBuf::from("/music/هیچکس.mp3"));
        let json = serde_json::to_string(&t).unwrap();
        // Readable string form, backward compatible with the old format.
        assert!(json.contains("/music/"));
        let back: Track = serde_json::from_str(&json).unwrap();
        assert_eq!(back.path, t.path);
    }

    #[test]
    fn content_key_ignores_case_and_whitespace() {
        let mut a = sample_track(PathBuf::from("/music/a.mp3"));
        a.title = "  Hello World ".into();
        a.artist = "Some ARTIST".into();
        a.album = "An Album".into();
        a.duration_secs = 200;

        let mut b = sample_track(PathBuf::from("/elsewhere/b.mp3"));
        b.title = "hello world".into();
        b.artist = "some artist".into();
        b.album = "an album".into();
        b.duration_secs = 200;

        assert_eq!(a.content_key(), b.content_key(), "same song, different files");

        // A different duration is treated as a different song.
        let mut c = b.clone();
        c.duration_secs = 201;
        assert_ne!(a.content_key(), c.content_key());
    }

    // The reported bug: the *same file* scanned through two different paths (e.g. a
    // followed symlink or overlapping scan folders) produced two library entries.
    // Identical paths must collapse to a single entry; genuinely different paths to
    // missing files are left alone.
    #[test]
    fn dedup_library_files_collapses_identical_paths() {
        let mut state = AppState::default();
        let p = PathBuf::from("/music/song.mp3");
        state.library.push(sample_track(p.clone()));
        state.library.push(sample_track(p.clone()));
        state.library.push(sample_track(PathBuf::from("/music/other.mp3")));

        let removed = state.dedup_library_files();
        assert_eq!(removed, 1);
        assert_eq!(state.library.len(), 2);
        assert_eq!(state.library[0].path, p);
        assert_eq!(state.library[1].path, PathBuf::from("/music/other.mp3"));
    }

    #[test]
    fn hide_duplicates_defaults_on_and_survives_old_state() {
        // Fresh default has the feature enabled.
        assert!(AppState::default().hide_duplicates);
        // State serialized before the field existed still loads (serde default).
        let json = r#"{"library":[],"playlists":[],"volume":1.0,"repeat":"Off","shuffle":false}"#;
        let back: AppState = serde_json::from_str(json).unwrap();
        assert!(back.hide_duplicates);
    }

    // Previously, a non-UTF-8 path made `serde_json::to_string` fail, which broke
    // saving the *entire* library. It must now round-trip losslessly instead.
    #[cfg(unix)]
    #[test]
    fn non_utf8_path_round_trips_losslessly() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let mut bytes = vec![0xE5, 0xED, 0x90]; // invalid UTF-8
        bytes.extend_from_slice(b".mp3");
        let path = PathBuf::from("/music").join(OsStr::from_bytes(&bytes));
        assert!(path.to_str().is_none());

        let t = sample_track(path.clone());
        let json = serde_json::to_string(&t).expect("non-UTF-8 path must serialize");
        let back: Track = serde_json::from_str(&json).unwrap();
        assert_eq!(back.path, path, "path must survive a save/load cycle");
    }

    #[cfg(unix)]
    #[test]
    fn state_with_non_utf8_paths_saves() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let p = PathBuf::from("/m").join(OsStr::from_bytes(&[0xFF, 0xFE, b'.', b'm', b'p', b'3']));
        let mut state = AppState::default();
        state.library.push(sample_track(p.clone()));
        state.playlists.push(Playlist {
            name: "pl".into(),
            track_paths: vec![p.clone()],
        });
        state.scanned_dirs.push(PathBuf::from("/m").join(OsStr::from_bytes(&[0xFF, 0xFE])));

        let json = serde_json::to_string_pretty(&state).expect("state must serialize");
        let back: AppState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.library[0].path, p);
        assert_eq!(back.playlists[0].track_paths[0], p);
    }
}
