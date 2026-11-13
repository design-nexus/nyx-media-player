//! Right-click menus for videos and shows, and the Fix match dialog.

use crate::library::store::{self, Show};
use crate::library::{Kind, Video, tmdb};
use crate::sections::{folders, movies, playlist, shows};
use crate::{cmd, fmt, player, prefs, views, widgets, window};
use gtk::prelude::*;
use nyx_ui::menu::{self, Entry};
use std::path::PathBuf;
use std::rc::Rc;

pub type Extra = (&'static str, Rc<dyn Fn()>);

pub fn added_text(n: usize, where_: &str) -> String {
    format!("Added {} {where_}.", fmt::count(n, "video", "videos"))
}

/// "Add to a playlist ›": a new one, or one of the playlists.
fn playlists_sub(paths: Vec<PathBuf>) -> Entry {
    let p = paths.clone();
    let mut entries = vec![menu::item("New playlist…", move || playlist::new_dialog(p.clone())).icon("list-add-symbolic")];
    let lists = store::playlists();
    if !lists.is_empty() {
        entries.push(menu::separator());
    }
    for pl in lists {
        let p = paths.clone();
        entries.push(menu::item(&pl.name, move || playlist::append(pl.id, p.clone())).icon("nmp-playlist-symbolic"));
    }
    menu::sub("Add to a playlist", entries).icon("nmp-playlist-symbolic")
}

/// Mark as watched or unwatched, all together.
pub fn toggle_watched(videos: &[Rc<Video>]) {
    let all_watched = videos.iter().all(|v| v.watch.get().watched);
    store::set_watched(videos, !all_watched);
}

/// Add to the end of the queue, with a toast saying how many.
pub fn enqueue(paths: Vec<PathBuf>) {
    let n = paths.len();
    player::enqueue(paths);
    window::toast(&added_text(n, "to the queue"));
}

/// The menu for one or more videos.
pub fn video_menu(anchor: &gtk::Widget, x: f64, y: f64, videos: Vec<Rc<Video>>, extra: Vec<Extra>) {
    let Some(first) = videos.first().cloned() else { return };
    let paths: Vec<PathBuf> = videos.iter().map(|v| v.path.clone()).collect();
    let single = videos.len() == 1;
    let mut m = Vec::new();
    if single {
        let v = first.clone();
        m.push(
            menu::item(if v.in_progress() { "Resume" } else { "Play" }, move || views::play_video(&v))
                .icon("media-playback-start-symbolic")
                .key("Enter"),
        );
        if first.in_progress() {
            let v = first.clone();
            m.push(
                menu::item("Play from the start", move || {
                    store::update_watch(&v, |w| w.position = 0.0);
                    views::play_video(&v);
                })
                .icon("media-skip-backward-symbolic"),
            );
        }
    } else {
        let p = paths.clone();
        m.push(
            menu::item("Play", move || {
                player::play_paths(p.clone(), 0);
                window::navigate("now-playing");
            })
            .icon("media-playback-start-symbolic")
            .key("Enter"),
        );
    }
    let p = paths.clone();
    m.push(
        menu::item("Play next", move || {
            player::play_next(p.clone());
            window::toast(&added_text(p.len(), "to play next"));
        })
        .icon("nmp-queue-symbolic"),
    );
    let p = paths.clone();
    m.push(menu::item("Add to the queue", move || enqueue(p.clone())).icon("list-add-symbolic").key("Q"));
    m.push(playlists_sub(paths.clone()));
    m.push(menu::separator());
    let all_watched = videos.iter().all(|v| v.watch.get().watched);
    let vs = videos.clone();
    m.push(
        menu::item(if all_watched { "Mark as unwatched" } else { "Mark as watched" }, move || toggle_watched(&vs))
            .icon("object-select-symbolic")
            .key("W"),
    );
    let started: Vec<Rc<Video>> = videos.iter().filter(|v| v.in_progress()).cloned().collect();
    if !started.is_empty() {
        m.push(
            menu::item("Remove from Continue watching", move || {
                for v in &started {
                    store::update_watch(v, |w| w.position = 0.0);
                }
            })
            .icon("window-close-symbolic"),
        );
    }
    let tmdb_on = tmdb::Settings::from_prefs(&prefs::get()).is_some();
    if single && first.id > 0 {
        match first.kind {
            Kind::Episode => {
                let key = first.show_key();
                m.push(menu::item("Go to the show", move || shows::open(&key)).icon("nmp-tv-symbolic"));
                if tmdb_on && let Some(show) = store::show_of(&first) {
                    m.push(
                        menu::item("Fix match…", move || fix_match(&tmdb::show_target(&show.key), "tv", &show.name, show.year))
                            .icon("system-search-symbolic"),
                    );
                }
            }
            Kind::Movie => {
                let path = first.path.clone();
                m.push(menu::item("Movie details", move || movies::open(&path)).icon("help-about-symbolic"));
                if tmdb_on {
                    let v = first.clone();
                    m.push(
                        menu::item("Fix match…", move || fix_match(&tmdb::movie_target(&v.path), "movie", &v.title, v.year))
                            .icon("system-search-symbolic"),
                    );
                }
            }
            Kind::Other => {}
        }
    }
    if !crate::library::is_url(&first.path) {
        if first.id > 0 {
            let path = first.path.clone();
            m.push(menu::item("Show in its folder", move || folders::show_file(&path)).icon("folder-symbolic"));
        }
        let dir = first.path.parent().map(|d| d.to_path_buf());
        m.push(
            menu::item("Open in the file manager", move || {
                if let Some(d) = &dir {
                    cmd::spawn(&["xdg-open", &d.to_string_lossy()]);
                }
            })
            .icon("folder-open-symbolic"),
        );
    }
    if !extra.is_empty() {
        m.push(menu::separator());
        for (label, f) in extra {
            m.push(menu::item(label, move || f()));
        }
    }
    menu::popup_at(anchor, x, y, m);
}

/// The menu for a show.
pub fn show_menu(anchor: &gtk::Widget, x: f64, y: f64, show: Rc<Show>) {
    let mut m = Vec::new();
    if let Some(next) = show.next_episode() {
        let label = format!("{} {}", if next.in_progress() { "Resume" } else { "Play" }, next.code());
        m.push(menu::item(&label, move || views::play_video(&next)).icon("media-playback-start-symbolic"));
    }
    let eps: Vec<Rc<Video>> = show.episodes().cloned().collect();
    let paths: Vec<PathBuf> = eps.iter().map(|e| e.path.clone()).collect();
    let p = paths.clone();
    m.push(
        menu::item("Play from the first episode", move || {
            player::play_paths(p.clone(), 0);
            window::navigate("now-playing");
        })
        .icon("media-skip-backward-symbolic"),
    );
    let p = paths.clone();
    m.push(menu::item("Add to the queue", move || enqueue(p.clone())).icon("list-add-symbolic"));
    m.push(playlists_sub(paths));
    m.push(menu::separator());
    let all_watched = show.unwatched() == 0;
    let e = eps.clone();
    m.push(
        menu::item(if all_watched { "Mark as unwatched" } else { "Mark as watched" }, move || store::set_watched(&e, !all_watched))
            .icon("object-select-symbolic"),
    );
    if tmdb::Settings::from_prefs(&prefs::get()).is_some() {
        let s = show.clone();
        m.push(
            menu::item("Fix match…", move || fix_match(&tmdb::show_target(&s.key), "tv", &s.name, s.year)).icon("system-search-symbolic"),
        );
    }
    if !show.folder.is_empty() {
        let folder = PathBuf::from(&show.folder);
        m.push(menu::item("Show in its folder", move || folders::show_dir(&folder)).icon("folder-symbolic"));
        let dir = show.folder.clone();
        m.push(menu::item("Open in the file manager", move || cmd::spawn(&["xdg-open", &dir])).icon("folder-open-symbolic"));
    }
    menu::popup_at(anchor, x, y, m);
}

/// Search TMDB and pick what a movie or show really is.
pub fn fix_match(target: &str, kind: &'static str, query: &str, year: Option<i32>) {
    let Some(settings) = tmdb::Settings::from_prefs(&prefs::get()) else { return };
    let (dialog, card) = widgets::dialog("Fix match", 520);
    let d = widgets::label("Search TMDB and pick the right one. Its details and pictures replace what's shown now.", "dim");
    d.set_wrap(true);
    card.append(&d);
    let search = gtk::SearchEntry::new();
    search.set_text(query);
    card.append(&search);
    let results = widgets::vbox(4);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .min_content_height(320)
        .child(&results)
        .build();
    card.append(&scroll);
    let buttons = widgets::hbox(8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Not now");
    let d2 = dialog.clone();
    cancel.connect_clicked(move |_| d2.close());
    buttons.append(&cancel);
    card.append(&buttons);

    let target = target.to_string();
    let run = {
        let (results, dialog) = (results.clone(), dialog.clone());
        Rc::new(move |q: String, year: Option<i32>| {
            while let Some(c) = results.first_child() {
                results.remove(&c);
            }
            let spinner = gtk::Spinner::new();
            spinner.set_spinning(true);
            spinner.set_margin_top(20);
            results.append(&spinner);
            let s = settings.clone();
            let (results, dialog, target) = (results.clone(), dialog.clone(), target.clone());
            cmd::background(
                move || tmdb::search(&s, kind, &q, year),
                move |res| {
                    while let Some(c) = results.first_child() {
                        results.remove(&c);
                    }
                    let hits = match res {
                        Ok(h) => h,
                        Err(e) => {
                            results.append(&widgets::label(&e, "danger-text"));
                            return;
                        }
                    };
                    if hits.is_empty() {
                        results.append(&widgets::label("Nothing found. Try fewer words, or the original title.", "dim"));
                    }
                    for h in hits.into_iter().take(12) {
                        let b = gtk::Button::new();
                        b.add_css_class("flat");
                        b.add_css_class("match-row");
                        let col = widgets::vbox(2);
                        let title = match h.year {
                            Some(y) => format!("{} ({y})", h.title),
                            None => h.title.clone(),
                        };
                        col.append(&widgets::label(&title, "settings-option-title"));
                        if !h.overview.is_empty() {
                            let o = widgets::label(&h.overview, "settings-option-description");
                            o.set_wrap(true);
                            o.set_lines(2);
                            o.set_ellipsize(gtk::pango::EllipsizeMode::End);
                            o.set_max_width_chars(60);
                            col.append(&o);
                        }
                        b.set_child(Some(&col));
                        let (dialog, target) = (dialog.clone(), target.clone());
                        b.connect_clicked(move |_| {
                            dialog.close();
                            let t = target.clone();
                            cmd::background(
                                move || tmdb::set_match(&t, h.id),
                                |r| match r {
                                    Ok(()) => {
                                        window::toast("Looking it up again…");
                                        store::rescan(false);
                                    }
                                    Err(e) => window::toast(&format!("Couldn't save the match: {e}")),
                                },
                            );
                        });
                        results.append(&b);
                    }
                },
            );
        })
    };
    run(query.to_string(), year);
    search.connect_activate(move |e| run(e.text().trim().to_string(), None));
    dialog.present();
    search.grab_focus();
}
