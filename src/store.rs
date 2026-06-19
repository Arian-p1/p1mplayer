use crate::model::AppState;
use directories::ProjectDirs;
use std::path::PathBuf;

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
    let path = state_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            eprintln!("p1mplayer: failed to parse {}: {e}", path.display());
            AppState::default()
        }),
        Err(_) => AppState::default(),
    }
}

/// Persist state to disk (best effort; logs on failure).
pub fn save(state: &AppState) {
    let path = state_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(state) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                eprintln!("p1mplayer: failed to save state: {e}");
            }
        }
        Err(e) => eprintln!("p1mplayer: failed to serialize state: {e}"),
    }
}
