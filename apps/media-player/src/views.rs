//! Shared library views: poster cards and grids, wide cards and rows of them,
//! episode rows, and the header of a detail view.

use crate::library::art::{self, Art};
use crate::library::store::{self, Show};
use crate::library::{Kind, Video};
use crate::{fmt, menu, player, widgets};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use std::path::PathBuf;
use std::rc::Rc;

fn boxed<T: 'static>(obj: &glib::Object) -> Option<std::cell::Ref<'_, T>> {
    obj.downcast_ref::<glib::BoxedAnyObject>().map(|b| b.borrow::<T>())
}

/// A thin bar along the bottom of a picture showing how far in a video is.
pub use nyx_ui::media::{progress_strip, set_progress, badge, set_badge, hover_play};

/// Something a poster grid holds.
#[derive(Clone)]
pub enum Item {
    Video(Rc<Video>),
    Show(Rc<Show>),
}

/// A poster with its title and a second line.
#[derive(Clone)]
pub struct PosterCard {
    pub root: gtk::Box,
    art: Art,
    strip: gtk::ProgressBar,
    badge: gtk::Label,
    title: gtk::Label,
    meta: gtk::Label,
}

impl PosterCard {
    pub fn new(width: i32) -> PosterCard {
        let art = Art::poster(width);
        let nyx_ui::media::PosterCard { root, strip, badge, title, meta } = nyx_ui::media::PosterCard::new(&art.root, width);
        PosterCard { root, art, strip, badge, title, meta }
    }

    pub fn bind(&self, item: &Item) {
        match item {
            Item::Video(v) => {
                self.art.set_key(v.card_art());
                self.title.set_text(&v.title);
                self.title.set_tooltip_text(Some(&v.label()));
                let mut meta = Vec::new();
                if let Some(y) = v.year {
                    meta.push(y.to_string());
                }
                if v.duration > 0.0 {
                    meta.push(fmt::runtime(v.duration));
                }
                self.meta.set_text(&meta.join(" · "));
                set_progress(&self.strip, v.progress());
                set_badge(&self.badge, v.watch.get().watched, 0);
            }
            Item::Show(s) => {
                self.art.set_key(&s.art());
                self.title.set_text(&s.name);
                self.title.set_tooltip_text(Some(&s.name));
                let seasons = s.seasons.iter().filter(|x| x.number.is_some_and(|n| n > 0)).count();
                let mut meta = Vec::new();
                if let Some(y) = s.year {
                    meta.push(y.to_string());
                }
                meta.push(if seasons > 1 {
                    fmt::count(seasons, "season", "seasons")
                } else {
                    fmt::count(s.count(), "episode", "episodes")
                });
                self.meta.set_text(&meta.join(" · "));
                set_progress(&self.strip, 0.0);
                let unwatched = s.unwatched();
                set_badge(&self.badge, unwatched == 0, unwatched);
            }
        }
    }
}

/// A virtualized grid of poster cards.
#[derive(Clone)]
pub struct PosterGrid {
    pub root: gtk::ScrolledWindow,
    store: gio::ListStore,
}

impl PosterGrid {
    pub fn new(on_open: impl Fn(Item) + 'static) -> PosterGrid {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            let card = PosterCard::new(150);
            // Right-click: the menu for what this cell shows now.
            let click = gtk::GestureClick::new();
            click.set_button(gdk::BUTTON_SECONDARY);
            let weak = item.downgrade();
            click.connect_pressed(move |g, _, x, y| {
                let Some(item) = weak.upgrade() else { return };
                let Some(obj) = item.item() else { return };
                let Some(it) = boxed::<Item>(&obj).map(|i| i.clone()) else { return };
                if let Some(w) = g.widget() {
                    item_menu(&w, x, y, &it);
                }
            });
            card.root.add_controller(click);
            item.set_child(Some(&card.root));
            // Keep the parts with the item so bind can reach them.
            unsafe { item.set_data("card", card) };
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            let Some(obj) = item.item() else { return };
            let Some(it) = boxed::<Item>(&obj) else { return };
            if let Some(card) = unsafe { item.data::<PosterCard>("card") } {
                unsafe { card.as_ref() }.bind(&it);
            }
        });
        let model = gtk::NoSelection::new(Some(store.clone()));
        let view = gtk::GridView::new(Some(model), Some(factory));
        view.add_css_class("poster-grid");
        view.set_min_columns(2);
        view.set_max_columns(12);
        view.set_single_click_activate(true);
        let s = store.clone();
        view.connect_activate(move |_, pos| {
            if let Some(obj) = s.item(pos)
                && let Some(it) = boxed::<Item>(&obj).map(|i| i.clone())
            {
                on_open(it);
            }
        });
        let root = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .child(&view)
            .vexpand(true)
            .build();
        PosterGrid { root, store }
    }

    pub fn set(&self, items: Vec<Item>) {
        let objs: Vec<glib::BoxedAnyObject> = items.into_iter().map(glib::BoxedAnyObject::new).collect();
        self.store.splice(0, self.store.n_items(), &objs);
    }

    /// Re-bind every card (watched marks changed).
    pub fn refresh(&self) {
        let n = self.store.n_items();
        self.store.items_changed(0, n, n);
    }
}

/// The right-click menu for a grid item.
pub fn item_menu(anchor: &gtk::Widget, x: f64, y: f64, item: &Item) {
    match item {
        Item::Video(v) => menu::video_menu(anchor, x, y, vec![v.clone()], Vec::new()),
        Item::Show(s) => menu::show_menu(anchor, x, y, s.clone()),
    }
}

/// A 16:9 card: a frame, a progress strip, a title and a second line.
pub fn wide_card(v: &Rc<Video>, width: i32, subtitle: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("flat");
    b.add_css_class("wide-card");
    let col = widgets::vbox(6);
    let art = Art::wide(width);
    art.set_key(v.wide_art());
    let strip = progress_strip();
    set_progress(&strip, v.progress());
    art.root.add_overlay(&strip);
    let tick = badge();
    set_badge(&tick, v.watch.get().watched, 0);
    art.root.add_overlay(&tick);
    art.root.add_overlay(&hover_play());
    col.append(&art.root);
    let (title, second) = match v.kind {
        Kind::Episode => (v.show.clone(), v.episode_line()),
        _ => (v.title.clone(), v.year.map(|y| y.to_string()).unwrap_or_default()),
    };
    let t = widgets::label(&title, "card-title");
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    t.set_max_width_chars(1);
    t.set_size_request(width, -1);
    col.append(&t);
    let s = widgets::label(if subtitle.is_empty() { &second } else { subtitle }, "card-meta");
    s.set_ellipsize(gtk::pango::EllipsizeMode::End);
    s.set_max_width_chars(1);
    s.set_size_request(width, -1);
    col.append(&s);
    b.set_child(Some(&col));
    b.set_tooltip_text(Some(&v.label()));
    let v2 = v.clone();
    b.connect_clicked(move |_| play_video(&v2));
    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_SECONDARY);
    let v3 = v.clone();
    click.connect_pressed(move |g, _, x, y| {
        if let Some(w) = g.widget() {
            menu::video_menu(&w, x, y, vec![v3.clone()], Vec::new());
        }
    });
    b.add_controller(click);
    b
}

/// A titled row of cards that scrolls sideways (wheel, touchpad, keys).
pub fn card_row(title: &str) -> (gtk::Box, gtk::Box) {
    let group = widgets::vbox(8);
    group.add_css_class("card-row");
    let heading = widgets::label(title, "shelf-title");
    heading.set_xalign(0.0);
    group.append(&heading);
    let cards = widgets::hbox(14);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::External)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .child(&cards)
        .build();
    // A vertical wheel scrolls the page, not the row.
    scroll.set_propagate_natural_height(true);
    group.append(&scroll);
    (group, cards)
}

/// Play one video: in its show's order for episodes (so the next one
/// follows), on its own otherwise.
pub fn play_video(v: &Rc<Video>) {
    if v.kind == Kind::Episode
        && let Some(show) = store::show_of(v)
    {
        let eps: Vec<PathBuf> = show.episodes().map(|e| e.path.clone()).collect();
        if let Some(i) = eps.iter().position(|p| *p == v.path) {
            player::play_paths(eps[i..].to_vec(), 0);
            crate::window::navigate("now-playing");
            return;
        }
    }
    player::play_paths(vec![v.path.clone()], 0);
    crate::window::navigate("now-playing");
}

/// One episode in a show's list: a frame, its number and name, details and plot.
pub fn episode_row(v: &Rc<Video>) -> gtk::Box {
    let row = widgets::hbox(16);
    row.add_css_class("episode-row");
    let art = Art::wide(176);
    art.set_key(v.wide_art());
    let strip = progress_strip();
    set_progress(&strip, v.progress());
    art.root.add_overlay(&strip);
    art.root.add_overlay(&hover_play());
    row.append(&art.root);

    let text = widgets::vbox(3);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let top = widgets::hbox(8);
    let num = v.episode.map(|n| format!("{n}. ")).unwrap_or_default();
    let title = widgets::label(&format!("{num}{}", v.episode_name()), "episode-title");
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_hexpand(true);
    top.append(&title);
    if v.watch.get().watched {
        let w = widgets::label("Watched", "tag");
        w.set_valign(gtk::Align::Center);
        top.append(&w);
    }
    text.append(&top);
    let mut meta = Vec::new();
    if v.duration > 0.0 {
        meta.push(if v.in_progress() { fmt::left(v.duration - v.watch.get().position) } else { fmt::runtime(v.duration) });
    }
    let res = v.resolution();
    if !res.is_empty() {
        meta.push(res);
    }
    if let Some(r) = v.rating {
        meta.push(format!("★ {r:.1}"));
    }
    let m = widgets::label(&meta.join(" · "), "dim");
    m.add_css_class("mono");
    m.add_css_class("episode-meta");
    text.append(&m);
    if !v.plot.is_empty() {
        let p = widgets::label(&v.plot, "episode-plot");
        p.set_wrap(true);
        p.set_lines(2);
        p.set_ellipsize(gtk::pango::EllipsizeMode::End);
        p.set_max_width_chars(1);
        p.set_hexpand(true);
        text.append(&p);
    }
    row.append(&text);

    let click = gtk::GestureClick::new();
    click.set_button(0);
    let v2 = v.clone();
    click.connect_released(move |g, _, x, y| {
        let Some(w) = g.widget() else { return };
        if g.current_button() == gdk::BUTTON_SECONDARY {
            let v3 = v2.clone();
            let up_to: menu::Extra = (
                "Mark watched up to here",
                Rc::new(move || {
                    if let Some(show) = store::show_of(&v3) {
                        let eps: Vec<Rc<Video>> = show.episodes().cloned().collect();
                        if let Some(i) = eps.iter().position(|e| e.path == v3.path) {
                            store::set_watched(&eps[..=i], true);
                        }
                    }
                }),
            );
            menu::video_menu(&w, x, y, vec![v2.clone()], vec![up_to]);
        } else if g.current_button() == gdk::BUTTON_PRIMARY {
            play_video(&v2);
        }
    });
    row.add_controller(click);
    row.set_cursor_from_name(Some("pointer"));
    row
}

/// A back button for detail views.
pub fn back_button(label: &str, on_back: impl Fn() + 'static) -> gtk::Button {
    let b = widgets::labeled_button("go-previous-symbolic", label);
    b.add_css_class("flat");
    b.add_css_class("back-button");
    b.set_halign(gtk::Align::Start);
    b.connect_clicked(move |_| on_back());
    b
}

/// Play · Shuffle · Add to queue, for a list of videos.
pub fn play_buttons(paths: impl Fn() -> Vec<PathBuf> + 'static) -> gtk::Box {
    let row = widgets::hbox(8);
    let paths = Rc::new(paths);
    let play = widgets::labeled_button("media-playback-start-symbolic", "Play");
    play.add_css_class("suggested-action");
    let p = paths.clone();
    play.connect_clicked(move |_| {
        player::set_shuffle(false);
        player::play_paths(p(), 0);
        crate::window::navigate("now-playing");
    });
    let shuffle = widgets::labeled_button("media-playlist-shuffle-symbolic", "Shuffle");
    let p = paths.clone();
    shuffle.connect_clicked(move |_| {
        player::shuffle_paths(p());
        crate::window::navigate("now-playing");
    });
    let queue = widgets::labeled_button("list-add-symbolic", "Add to the queue");
    let p = paths.clone();
    queue.connect_clicked(move |_| menu::enqueue(p()));
    row.append(&play);
    row.append(&shuffle);
    row.append(&queue);
    row
}

/// The header of a movie, show, folder or playlist view: picture, kicker,
/// title, a line of details, a description and actions.
pub struct Header {
    /// The header over its backdrop (when it has one).
    pub root: gtk::Overlay,
    backdrop: gtk::Picture,
    pub title: gtk::Label,
    /// Year, runtime, quality, rating… as small chips.
    pub meta: gtk::Box,
    pub genres: gtk::Label,
    pub plot: gtk::Label,
    pub actions: gtk::Box,
}

pub fn detail_header(art: Art, kicker: &str) -> Header {
    let hero = gtk::Overlay::new();
    hero.add_css_class("detail-hero");
    hero.set_overflow(gtk::Overflow::Hidden);
    let backdrop = gtk::Picture::new();
    backdrop.set_content_fit(gtk::ContentFit::Cover);
    backdrop.set_can_shrink(true);
    backdrop.add_css_class("detail-backdrop");
    // An unmeasured overlay, so a big picture can't make the header taller.
    hero.set_child(Some(&widgets::vbox(0)));
    hero.add_overlay(&backdrop);
    // Keeps the text readable: solid behind the text, fading towards the right.
    let scrim = widgets::hbox(0);
    scrim.add_css_class("detail-scrim");
    scrim.set_can_target(false);
    hero.add_overlay(&scrim);
    let root = widgets::hbox(24);
    root.add_css_class("detail-header");
    hero.add_overlay(&root);
    // The header sizes the hero; the backdrop fills whatever that is.
    hero.set_measure_overlay(&root, true);
    root.append(&art.root);
    let text = widgets::vbox(6);
    text.set_valign(gtk::Align::Start);
    text.set_hexpand(true);
    if !kicker.is_empty() {
        text.append(&widgets::label(&kicker.to_uppercase(), "group-title"));
    }
    let title = widgets::label("", "detail-title");
    title.set_wrap(true);
    title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    title.set_lines(3);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&title);
    let meta = widgets::hbox(6);
    meta.add_css_class("detail-meta");
    text.append(&meta);
    let genres = widgets::label("", "detail-genres");
    genres.set_wrap(true);
    text.append(&genres);
    let plot = widgets::label("", "detail-plot");
    plot.set_wrap(true);
    plot.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    plot.set_max_width_chars(80);
    text.append(&plot);
    let actions = widgets::hbox(8);
    actions.add_css_class("detail-actions");
    actions.set_margin_top(8);
    text.append(&actions);
    root.append(&text);
    Header { root: hero, backdrop, title, meta, genres, plot, actions }
}

impl Header {
    pub fn set_text(&self, title: &str, meta: &[String], genres: &str, plot: &str) {
        self.title.set_text(title);
        while let Some(c) = self.meta.first_child() {
            self.meta.remove(&c);
        }
        for part in meta {
            let chip = widgets::label(part, "chip");
            if part.starts_with('★') {
                chip.add_css_class("rating");
            }
            self.meta.append(&chip);
        }
        self.meta.set_visible(!meta.is_empty());
        self.genres.set_text(genres);
        self.genres.set_visible(!genres.is_empty());
        self.plot.set_text(plot);
        self.plot.set_visible(!plot.is_empty());
    }

    /// Show a wide picture behind the header; "" for none.
    pub fn set_backdrop(&self, key: &str) {
        let (hero, picture) = (self.root.clone(), self.backdrop.clone());
        art::load(key, false, move |tex| {
            picture.set_paintable(tex.as_ref());
            if tex.is_some() {
                hero.add_css_class("has-backdrop");
            } else {
                hero.remove_css_class("has-backdrop");
            }
        });
    }
}

/// ["2019", "2 h 12 min", "1080p", "★ 7.8"]
pub fn video_meta(v: &Video) -> Vec<String> {
    let mut parts = Vec::new();
    if let Some(y) = v.year {
        parts.push(y.to_string());
    }
    if v.duration > 0.0 {
        parts.push(fmt::runtime(v.duration));
    }
    let r = v.resolution();
    if !r.is_empty() {
        parts.push(r);
    }
    if let Some(r) = v.rating {
        parts.push(format!("★ {r:.1}"));
    }
    parts
}
