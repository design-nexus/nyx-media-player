//! Well-known locations. Every path honours the XDG overrides so the whole app
//! can be pointed at a scratch copy of `~/.config` for testing.

pub use nyx_core::paths::*;

use std::path::PathBuf;

/// The library database.
pub fn library_db() -> PathBuf {
    data_dir().join("library.db")
}

/// Posters and frames, scaled once.
pub fn art_dir() -> PathBuf {
    cache_dir().join("art")
}

/// Seek-bar preview strips.
pub fn scrub_dir() -> PathBuf {
    cache_dir().join("scrub")
}

/// A folder from `user-dirs.dirs` (`XDG_VIDEOS_DIR`…), or `~/<fallback>`.
fn user_dir(key: &str, fallback: &str) -> PathBuf {
    let dirs = config_home().join("user-dirs.dirs");
    if let Ok(text) = std::fs::read_to_string(dirs) {
        for line in text.lines() {
            if let Some(v) = line.trim().strip_prefix(key).and_then(|v| v.strip_prefix('=')) {
                let v = v.trim_matches('"').replace("$HOME", &home().to_string_lossy());
                if !v.is_empty() {
                    return PathBuf::from(v);
                }
            }
        }
    }
    home().join(fallback)
}

/// The user's videos folder (`XDG_VIDEOS_DIR`), or `~/Videos`.
pub fn videos_dir() -> PathBuf {
    user_dir("XDG_VIDEOS_DIR", "Videos")
}

/// Where frame captures go: `XDG_PICTURES_DIR`, or `~/Pictures`.
pub fn pictures_dir() -> PathBuf {
    user_dir("XDG_PICTURES_DIR", "Pictures")
}

/// The play queue and position, so playback can resume.
pub fn state_file() -> PathBuf {
    data_dir().join("state.json")
}
