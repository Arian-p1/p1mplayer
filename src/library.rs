use crate::model::Track;
use lofty::file::TaggedFileExt;
use lofty::prelude::{Accessor, AudioFile};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

pub const AUDIO_EXTS: &[&str] = &["mp3", "flac", "ogg", "wav", "m4a", "aac", "opus", "mp4"];

fn is_audio(path: &Path) -> bool {
    // Skip dotfiles, including macOS AppleDouble sidecars (`._foo.mp3`) which carry an
    // audio extension but are not real audio and fail to decode.
    //
    // Check the raw bytes of the name rather than requiring it to be valid UTF-8:
    // filenames in a legacy code page (e.g. Persian names copied from Windows) are
    // not valid UTF-8 on Linux, and must not be silently dropped here.
    match path.file_name() {
        Some(name) => {
            if name.as_encoded_bytes().first() == Some(&b'.') {
                return false;
            }
        }
        None => return false,
    }
    // Extensions are effectively always ASCII, so a UTF-8 check on just the extension
    // is fine even when the rest of the name is not UTF-8.
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Recursively scan one or more directories, delivering tracks in batches through
/// `on_batch` as they are found. Streaming keeps the UI populating progressively
/// instead of waiting for the whole (possibly large) scan to finish.
pub fn scan_stream<F: FnMut(Vec<Track>)>(dirs: &[std::path::PathBuf], covers_dir: &Path, mut on_batch: F) {
    const BATCH: usize = 24;
    let mut batch = Vec::with_capacity(BATCH);
    for dir in dirs {
        for entry in walkdir::WalkDir::new(dir)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if entry.file_type().is_file() && is_audio(path) {
                if let Some(track) = read_track(path, covers_dir) {
                    batch.push(track);
                    if batch.len() >= BATCH {
                        on_batch(std::mem::take(&mut batch));
                    }
                }
            }
        }
    }
    if !batch.is_empty() {
        on_batch(batch);
    }
}

/// Read tags + cover art for one file. Returns `None` only if the file can't be parsed at all.
pub fn read_track(path: &Path, covers_dir: &Path) -> Option<Track> {
    let tagged = match lofty::read_from_path(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("p1mplayer: skipping {}: {e}", path.display());
            return None;
        }
    };

    let duration_secs = tagged.properties().duration().as_secs();
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());

    let stem_fallback = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Unknown".to_string());

    let (title, artist, album, cover_path) = match tag {
        Some(tag) => {
            let title = tag
                .title()
                .map(|c| c.to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or(stem_fallback);
            let artist = tag
                .artist()
                .map(|c| c.to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Unknown Artist".to_string());
            let album = tag.album().map(|c| c.to_string()).unwrap_or_default();
            let cover = extract_cover(tag, path, covers_dir);
            (title, artist, album, cover)
        }
        None => (
            stem_fallback,
            "Unknown Artist".to_string(),
            String::new(),
            None,
        ),
    };

    Some(Track {
        path: path.to_path_buf(),
        title,
        artist,
        album,
        duration_secs,
        cover_path,
    })
}

/// Largest dimension (px) of cached cover thumbnails. Small thumbnails keep both
/// decode time and memory low when a large library is displayed.
const THUMB_SIZE: u32 = 128;

/// Extract the first embedded picture, downscale it to a small PNG thumbnail cached
/// under `covers_dir`, and return its path. Returns `None` if there's no usable art.
fn extract_cover(tag: &lofty::tag::Tag, audio_path: &Path, covers_dir: &Path) -> Option<PathBuf> {
    let pic = tag.pictures().first()?;

    let mut hasher = DefaultHasher::new();
    audio_path.hash(&mut hasher);
    let dest = covers_dir.join(format!("{:016x}.png", hasher.finish()));

    if dest.exists() {
        return Some(dest);
    }

    let img = image::load_from_memory(pic.data()).ok()?;
    // `thumbnail` uses a fast filter and preserves aspect ratio.
    let thumb = img.thumbnail(THUMB_SIZE, THUMB_SIZE);
    if thumb.save(&dest).is_err() {
        return None;
    }
    Some(dest)
}


#[cfg(test)]
mod tests {
    use super::is_audio;
    use std::path::Path;

    #[test]
    fn accepts_plain_and_utf8_names() {
        assert!(is_audio(Path::new("/music/song.mp3")));
        assert!(is_audio(Path::new("/music/هیچکس.mp3"))); // valid UTF-8 Persian
        assert!(is_audio(Path::new("/music/track.FLAC"))); // case-insensitive ext
        assert!(!is_audio(Path::new("/music/cover.jpg"))); // not audio
        assert!(!is_audio(Path::new("/music/.hidden.mp3"))); // dotfile
        assert!(!is_audio(Path::new("/music/._sidecar.mp3"))); // AppleDouble
    }

    // A filename in a legacy code page (here: Windows-1256 bytes for "هیچکس") is not
    // valid UTF-8 on Linux. It must still be recognised as audio rather than dropped.
    #[cfg(unix)]
    #[test]
    fn accepts_non_utf8_names() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        use std::path::PathBuf;

        // Invalid-UTF-8 stem bytes + an ASCII ".mp3" extension.
        let mut bytes = vec![0xE5, 0xED, 0x90, 0x98, 0x9A]; // arbitrary non-UTF-8
        bytes.extend_from_slice(b".mp3");
        let name = OsStr::from_bytes(&bytes);
        assert!(name.to_str().is_none(), "test name should be invalid UTF-8");

        let path = PathBuf::from("/music").join(name);
        assert!(is_audio(&path), "non-UTF-8 audio filename was dropped");

        // A non-UTF-8 dotfile should still be skipped.
        let mut dot = vec![b'.'];
        dot.extend_from_slice(&[0xE5, 0xED]);
        dot.extend_from_slice(b".mp3");
        let dot_path = PathBuf::from("/music").join(OsStr::from_bytes(&dot));
        assert!(!is_audio(&dot_path));
    }
}
