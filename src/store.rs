use crate::model::AppState;
use directories::ProjectDirs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", "p1mplayer")
}

/// `~/.config/p1mplayer` on Linux (falls back to the current dir if unavailable).
pub fn config_dir() -> PathBuf {
    project_dirs()
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".p1mplayer"))
}

/// Directory where extracted cover thumbnails are cached.
pub fn covers_dir() -> PathBuf {
    let dir = config_dir().join("covers");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn state_path() -> PathBuf {
    config_dir().join("state.json")
}

/// Load persisted state, or a fresh default if nothing is saved yet.
pub fn load() -> AppState {
    load_from(&state_path())
}

/// Persist state to disk (best effort; logs on failure).
pub fn save(state: &AppState) {
    save_to(&state_path(), state);
}

fn load_from(path: &Path) -> AppState {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            eprintln!("p1mplayer: failed to parse {}: {e}", path.display());
            // Move the unreadable file aside so the next save can't clobber it.
            let backup = backup_path(path);
            match std::fs::rename(path, &backup) {
                Ok(()) => eprintln!("p1mplayer: moved bad state file to {}", backup.display()),
                Err(e) => eprintln!("p1mplayer: failed to back up {}: {e}", path.display()),
            }
            AppState::default()
        }),
        Err(_) => AppState::default(),
    }
}

/// `state.json.bak`, or `state.json.bak.<unix-secs>` if that already exists.
fn backup_path(path: &Path) -> PathBuf {
    let bak = with_suffix(path, ".bak");
    if !bak.exists() {
        return bak;
    }
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    with_suffix(path, &format!(".bak.{secs}"))
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

/// Write to a sibling temp file, sync it, then rename it over `path` so a crash
/// mid-write never leaves a truncated state file behind.
fn save_to(path: &Path, state: &AppState) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let json = match serde_json::to_string_pretty(state) {
        Ok(json) => json,
        Err(e) => {
            eprintln!("p1mplayer: failed to serialize state: {e}");
            return;
        }
    };
    let tmp = with_suffix(path, ".tmp");
    let result = std::fs::File::create(&tmp)
        .and_then(|mut f| {
            f.write_all(json.as_bytes())?;
            f.sync_all()
        })
        .and_then(|()| std::fs::rename(&tmp, path));
    if let Err(e) = result {
        eprintln!("p1mplayer: failed to save state: {e}");
        let _ = std::fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh, unique scratch directory under the system temp dir.
    fn scratch_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "p1mplayer-test-{name}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_state() -> AppState {
        let mut state = AppState::default();
        state.volume = 0.42;
        state.shuffle = !state.shuffle;
        state.scanned_dirs = vec![PathBuf::from("/music/a"), PathBuf::from("/music/b")];
        state
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = scratch_dir("roundtrip");
        let path = dir.join("state.json");
        let state = sample_state();
        save_to(&path, &state);
        let loaded = load_from(&path);
        assert_eq!(
            serde_json::to_string(&loaded).unwrap(),
            serde_json::to_string(&state).unwrap()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_is_backed_up_and_default_returned() {
        let dir = scratch_dir("corrupt");
        let path = dir.join("state.json");
        std::fs::write(&path, "{ not json").unwrap();
        let loaded = load_from(&path);
        assert_eq!(
            serde_json::to_string(&loaded).unwrap(),
            serde_json::to_string(&AppState::default()).unwrap()
        );
        assert!(!path.exists());
        let bak = dir.join("state.json.bak");
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), "{ not json");

        // A second corrupt file must not overwrite the existing backup.
        std::fs::write(&path, "garbage #2").unwrap();
        load_from(&path);
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), "{ not json");
        let extra: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("state.json.bak."))
            .collect();
        assert_eq!(extra.len(), 1);
        assert_eq!(
            std::fs::read_to_string(dir.join(&extra[0])).unwrap(),
            "garbage #2"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_leaves_no_tmp_file() {
        let dir = scratch_dir("notmp");
        let path = dir.join("state.json");
        save_to(&path, &sample_state());
        save_to(&path, &sample_state());
        assert!(path.exists());
        assert!(!dir.join("state.json.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
