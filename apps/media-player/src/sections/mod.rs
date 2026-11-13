//! Every page in the sidebar, in order. Playlists are added to the sidebar
//! as they're made (see `playlist`).

use crate::library::store;
use crate::widgets::{self, Page};
use crate::{fmt, paths};
use gtk::prelude::*;

pub mod equalizer;
pub mod folders;
pub mod home;
pub mod movies;
pub mod now_playing;
pub mod playlist;
pub mod queue;
pub mod search;
pub mod settings;
pub mod shows;
pub mod videos;

pub struct Section {
    pub id: &'static str,
    pub title: &'static str,
    pub icon: &'static str,
    pub group: &'static str,
    pub description: &'static str,
    pub build: fn(&Page),
    /// The page manages its own scrolling (tables, grids).
    pub fill: bool,
    /// Listed in the sidebar (search isn't).
    pub nav: bool,
    /// No page padding (the player).
    pub bare: bool,
}

pub fn all() -> Vec<Section> {
    let s = |id, title, icon, group, description, build, fill| Section {
        id,
        title,
        icon,
        group,
        description,
        build,
        fill,
        nav: true,
        bare: false,
    };
    vec![
        s("home", "Home", "nmp-home-symbolic", "Library", "Pick up where you left off, and what's new.", home::build, false),
        s("movies", "Movies", "nmp-movie-symbolic", "Library", "Every movie in your library.", movies::build, true),
        s("tv", "TV shows", "nmp-tv-symbolic", "Library", "Your shows, by season and episode.", shows::build, true),
        s(
            "videos",
            "All videos",
            "nmp-video-symbolic",
            "Library",
            "Everything in your library, in one list.",
            videos::build,
            true,
        ),
        // The sidebar lists the top folders themselves; each opens this page.
        Section {
            nav: false,
            ..s("folders", "Folders", "folder-symbolic", "Folders", "Your videos as they're laid out on disk.", folders::build, true)
        },
        Section {
            bare: true,
            ..s(
                "now-playing",
                "Now playing",
                "media-playback-start-symbolic",
                "Playing",
                "The video that's playing.",
                now_playing::build,
                true,
            )
        },
        s("queue", "Queue", "nmp-queue-symbolic", "Playing", "What's playing now and next.", queue::build, true),
        // Opened from the top bar as a card over the window, not listed in the sidebar.
        Section {
            nav: false,
            ..s(
                "equalizer",
                "Equalizer",
                "nmp-equalizer-symbolic",
                "Sound",
                "Shape the sound with ten bands and a preamp.",
                equalizer::build,
                false,
            )
        },
        // Opened from the top bar as a dialog, not listed in the sidebar.
        Section {
            nav: false,
            ..s(
                "settings",
                "Settings",
                "emblem-system-symbolic",
                "App",
                "Library folders, playback, subtitles, online details and this window.",
                settings::build,
                false,
            )
        },
        Section {
            nav: false,
            ..s(
                "search",
                "Search",
                "system-search-symbolic",
                "Library",
                "Movies, shows and videos matching your search.",
                search::build,
                true,
            )
        },
    ]
}

/// A banner that shows while the library is being scanned.
pub fn scan_banner() -> gtk::Box {
    let b = widgets::banner("", false);
    b.set_visible(false);
    b.set_margin_bottom(14);
    let label = b.last_child().and_downcast::<gtk::Label>();
    let refresh = {
        let b = b.clone();
        move || match store::scanning() {
            Some((done, total)) => {
                b.set_visible(true);
                if let Some(l) = &label {
                    l.set_markup(&if total == 0 {
                        "Looking for videos…".to_string()
                    } else {
                        format!(
                            "Reading {} of {}…",
                            fmt::thousands(done),
                            fmt::count(total, "new or changed video", "new or changed videos")
                        )
                    });
                }
            }
            None => b.set_visible(false),
        }
    };
    refresh();
    store::subscribe(&b, move |c| {
        if c == store::Change::Scan {
            refresh();
        }
    });
    b
}

/// What to show when the library is empty: why, and how to fix it.
pub fn empty_library() -> gtk::Box {
    let roots = store::roots();
    let any = roots.iter().any(|r| r.is_dir());
    let shown = roots.first().map(|r| paths::pretty(r)).unwrap_or_else(|| "~/Videos".into());
    let (title, desc) = if !any || roots.is_empty() {
        ("No videos folder", format!("<tt>{}</tt> doesn't exist. Choose the folder your videos are in.", glib_escape(&shown)))
    } else {
        (
            "No videos yet",
            format!("No videos were found in <tt>{}</tt>. Add some there, or choose another folder.", glib_escape(&shown)),
        )
    };
    widgets::empty_state("folder-videos-symbolic", title, &desc, Some(("Choose folder", Box::new(settings::choose_folder))))
}

pub fn glib_escape(s: &str) -> String {
    gtk::glib::markup_escape_text(s).to_string()
}

/// A library view that swaps between its content, an empty state (`empty`
/// when the library has videos but this view has none) and loading.
pub fn library_stack(content: &impl IsA<gtk::Widget>, has_content: impl Fn() -> bool + 'static, empty: gtk::Box) -> gtk::Stack {
    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    stack.add_css_class("library-stack");
    stack.add_named(content, Some("content"));
    let empty_holder = widgets::vbox(0);
    empty_holder.set_vexpand(true);
    stack.add_named(&empty_holder, Some("empty"));
    stack.add_named(&empty, Some("none-here"));
    let loading = gtk::Spinner::new();
    loading.set_spinning(true);
    loading.set_halign(gtk::Align::Center);
    loading.set_valign(gtk::Align::Center);
    loading.set_size_request(32, 32);
    stack.add_named(&loading, Some("loading"));
    let refresh = {
        let stack = stack.clone();
        move || {
            let replace = |w: gtk::Box| {
                while let Some(c) = empty_holder.first_child() {
                    empty_holder.remove(&c);
                }
                empty_holder.append(&w);
            };
            if !store::loaded() {
                stack.set_visible_child_name("loading");
            } else if let Some(e) = store::error() {
                let msg = format!("<tt>{}</tt>", glib_escape(&e));
                replace(widgets::empty_state("dialog-warning-symbolic", "Couldn't open the library", &msg, None));
                stack.set_visible_child_name("empty");
            } else if store::videos().is_empty() {
                if store::scanning().is_some() {
                    stack.set_visible_child_name("loading");
                } else {
                    replace(empty_library());
                    stack.set_visible_child_name("empty");
                }
            } else if !has_content() {
                stack.set_visible_child_name("none-here");
            } else {
                stack.set_visible_child_name("content");
            }
        }
    };
    refresh();
    store::subscribe(&stack, move |c| {
        if c != store::Change::Watch {
            refresh();
        }
    });
    stack
}
