//! Media Player — a video player for Omarchy.

mod cmd;
mod control;
mod eq;
mod fmt;
mod inhibit;
mod library;
mod menu;
mod mpris;
mod panel_dialog;
mod paths;
mod player;
mod playerbar;
mod prefs;
mod sections;
mod secrets;
mod settings_dialog;
mod seekbar;
mod theme;
mod videotable;
mod views;
mod widgets;
mod window;

use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::Cell;
use std::path::PathBuf;

pub const APP_ID: &str = "io.github.design_nexus.nyx.MediaPlayer";

const USAGE: &str = "Usage: nyx-media-player [OPTIONS] [FILES…]\n\
\n\
  FILES…          play these videos, folders or .m3u playlists\n\
  --enqueue       add FILES to the queue instead of playing them now\n\
  --fullscreen    play FILES in fullscreen\n\
  --section ID    open (or switch the open window) to a page: home, movies, tv, videos,\n\
                  folders, now-playing, queue, equalizer, settings; or a show (tv:NAME)\n\
                  or a movie (movie:PATH)\n\
  --toggle        close the window if it's open, otherwise open it (for a keybinding)\n\
  --play-pause, --play, --pause, --stop, --next, --previous\n\
                  control playback in the running window\n";

thread_local! {
    static STARTED: Cell<bool> = const { Cell::new(false) };
}

/// One-time setup in the primary instance: playback, library, MPRIS.
/// Video drawing joins once the window exists (see `player::attach`).
fn start(app: &gtk::Application) {
    if STARTED.with(|s| s.replace(true)) {
        return;
    }
    player::init();
    library::store::reload();
    // Let the window appear first, then look for new videos.
    glib::timeout_add_local_once(std::time::Duration::from_millis(600), || library::store::rescan(false));
    library::store::start_watching();
    mpris::start(app);
    inhibit::start();
}

/// Videos from the command line: files, folders (everything inside, in path
/// order) and M3U playlists.
fn expand(files: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for f in files {
        if f.is_dir() {
            let mut inside: Vec<PathBuf> = walkdir::WalkDir::new(f)
                .follow_links(true)
                .into_iter()
                .flatten()
                .filter(|e| e.file_type().is_file() && library::is_video(e.path()))
                .map(|e| e.into_path())
                .collect();
            inside.sort();
            out.extend(inside);
        } else if f.extension().is_some_and(|e| e.eq_ignore_ascii_case("m3u") || e.eq_ignore_ascii_case("m3u8")) {
            if let Ok(bytes) = std::fs::read(f) {
                let base = f.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                out.extend(library::m3u::parse(&String::from_utf8_lossy(&bytes), &base).into_iter().filter(|p| p.exists()));
            }
        } else if f.is_file() {
            out.push(f.clone());
        }
    }
    out
}

fn main() -> glib::ExitCode {
    nyx_core::app::init(nyx_core::app::App { slug: "nyx-media-player", name: "Media Player" });
    nyx_core::migrate::from(&["nexus-media-player"]);
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        return glib::ExitCode::SUCCESS;
    }

    // GTK's Vulkan renderer enumerates every GPU at startup, which wakes a
    // sleeping discrete GPU on hybrid laptops. GL renders only on the one in use.
    if std::env::var_os("GSK_RENDERER").is_none() {
        // SAFETY: still single-threaded; nothing else reads the environment yet.
        unsafe { std::env::set_var("GSK_RENDERER", "ngl") };
    }

    let id = std::env::var("NYX_MEDIA_PLAYER_APP_ID").unwrap_or_else(|_| APP_ID.to_string());
    let app = gtk::Application::builder().application_id(id.as_str()).flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE).build();
    // Nyx Assistant finds and plays videos through this (primary instance only).
    app.connect_startup(|app| nyx_core::control::serve_async(app, control::tools(), control::handle));
    app.connect_command_line(|app, cl| {
        let argv: Vec<String> = cl.arguments().iter().map(|a| a.to_string_lossy().to_string()).collect();
        let has = |flag: &str| argv.iter().any(|a| a == flag);
        let section = argv.iter().position(|a| a == "--section").and_then(|i| argv.get(i + 1)).cloned();
        let mut files = Vec::new();
        let mut skip = true; // argv[0]
        for a in &argv {
            if std::mem::take(&mut skip) {
                continue;
            }
            if a == "--section" {
                skip = true;
                continue;
            }
            if a.starts_with("--") {
                continue;
            }
            // Relative paths resolve against the caller's directory, and URIs work too.
            if let Some(p) = cl.create_file_for_arg(a).path() {
                files.push(p);
            }
        }

        if has("--toggle")
            && let Some(w) = window::window()
            && w.is_visible()
        {
            w.close();
            return glib::ExitCode::SUCCESS;
        }
        start(app);
        let remote = ["--play-pause", "--play", "--pause", "--stop", "--next", "--previous"].iter().any(|f| has(f));
        // Playback commands to a running window shouldn't pop it up.
        if !(remote && window::window().is_some() && files.is_empty() && section.is_none()) {
            window::present(app, section.as_deref());
        }
        if has("--play-pause") {
            player::toggle();
        }
        if has("--play") {
            player::play();
        }
        if has("--pause") {
            player::pause();
        }
        if has("--stop") {
            player::stop();
        }
        if has("--next") {
            player::next();
        }
        if has("--previous") {
            player::previous();
        }
        let videos = expand(&files);
        if !videos.is_empty() {
            if has("--enqueue") {
                let n = videos.len();
                player::enqueue(videos);
                window::toast(&menu::added_text(n, "to the queue"));
            } else {
                player::play_paths(videos, 0);
                window::navigate("now-playing");
                if has("--fullscreen") {
                    window::set_fullscreen(true);
                }
            }
        }
        glib::ExitCode::SUCCESS
    });
    app.connect_shutdown(|_| {
        prefs::flush();
        player::shutdown();
    });
    app.run()
}
