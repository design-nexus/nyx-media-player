//! This app's own preferences (`~/.config/nyx-media-player/settings.toml`).

pub use nyx_theme::ThemeMode;

use crate::paths;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EqPreset {
    pub name: String,
    pub preamp: f64,
    pub bands: [f64; 10],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub mode: ThemeMode,
    pub theme: String,
    pub reduce_motion: bool,
    /// The sidebar shows only icons, whatever the window width.
    pub sidebar_collapsed: bool,
    pub glow: bool,
    pub last_section: String,
    /// How the Movies and TV shows grids were last sorted and filtered.
    pub movies_sort: String,
    pub movies_filter: String,
    pub shows_sort: String,
    pub shows_filter: String,
    /// Folders the library is built from.
    pub library_folders: Vec<String>,
    /// Rescan when files are added, removed or renamed.
    pub watch: bool,
    /// Grab a frame from videos that have no picture.
    pub frames: bool,
    /// Pick up each video where it was left.
    pub resume: bool,
    /// Bring back the queue on start.
    pub restore_queue: bool,
    /// Play a show's next episode when one ends.
    pub autoplay_next: bool,
    pub up_next_seconds: u32,
    /// Seconds for ←/→ and Shift+←/→.
    pub skip_short: f64,
    pub skip_long: f64,
    /// Touchpad seeking goes the other way (for natural scrolling).
    pub swipe_reverse: bool,
    pub speed: f64,
    /// Decode on the graphics card when it can.
    pub hwdec: bool,
    /// Preferred track languages, as mpv takes them (`en,eng`); empty for the file's default.
    pub audio_lang: String,
    pub sub_lang: String,
    /// Show subtitles when a video has them.
    pub subtitles: bool,
    pub sub_scale: f64,
    /// Text subtitles on a dark box instead of an outline.
    pub sub_background: bool,
    /// Thumbnails on the seek bar.
    pub scrub: bool,
    /// The seek bar shows the time left instead of the length.
    pub time_left: bool,
    pub notify: bool,
    pub tmdb: bool,
    /// Older versions kept the key here; it moves to the Secret Service on first use.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub tmdb_key: String,
    pub tmdb_language: String,
    /// 0..1 (what the slider shows; mpv's own curve is cubic).
    pub volume: f64,
    pub muted: bool,
    /// Let the volume go up to 150%.
    pub volume_boost: bool,
    /// Even out loud and quiet parts (a compressor after the equalizer).
    pub night_mode: bool,
    pub shuffle: bool,
    /// off, all or one.
    pub repeat: String,
    pub eq_enabled: bool,
    /// The preset the faders came from ("" once they're edited by hand).
    pub eq_preset: String,
    pub eq_preamp: f64,
    pub eq_bands: [f64; 10],
    pub eq_custom: Vec<EqPreset>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            mode: ThemeMode::Omarchy,
            theme: "tokyo-night".into(),
            reduce_motion: false,
            sidebar_collapsed: false,
            glow: true,
            last_section: "home".into(),
            movies_sort: "title".into(),
            movies_filter: "all".into(),
            shows_sort: "title".into(),
            shows_filter: "all".into(),
            library_folders: vec![paths::videos_dir().to_string_lossy().into_owned()],
            watch: true,
            frames: true,
            resume: true,
            restore_queue: true,
            autoplay_next: true,
            up_next_seconds: 20,
            skip_short: 5.0,
            skip_long: 30.0,
            swipe_reverse: false,
            speed: 1.0,
            hwdec: true,
            audio_lang: String::new(),
            sub_lang: String::new(),
            subtitles: true,
            sub_scale: 1.0,
            sub_background: false,
            scrub: true,
            time_left: false,
            notify: false,
            tmdb: false,
            tmdb_key: String::new(),
            tmdb_language: "en-US".into(),
            volume: 1.0,
            muted: false,
            volume_boost: false,
            night_mode: false,
            shuffle: false,
            repeat: "off".into(),
            eq_enabled: false,
            eq_preset: "flat".into(),
            eq_preamp: 0.0,
            eq_bands: [0.0; 10],
            eq_custom: Vec::new(),
        }
    }
}

nyx_core::prefs_store!(Prefs);
