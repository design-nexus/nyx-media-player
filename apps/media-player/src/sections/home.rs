//! Home: Continue watching and Next up as lists with the time left and Resume,
//! and Recently added beside them.

use super::{library_stack, scan_banner};
use crate::fmt;
use crate::library::art::Art;
use crate::library::{Kind, Video, store};
use crate::views;
use crate::widgets::{self, Page};
use gtk::glib;
use gtk::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

/// Rows in each list before "See all".
const CONTINUE: usize = 6;
const NEXT: usize = 5;
const RECENT: usize = 8;

/// "S2 · E5", "E5", or "".
fn short_code(v: &Video) -> String {
    match (v.season, v.episode) {
        (Some(s), Some(e)) => format!("S{s} · E{e}"),
        (None, Some(e)) => format!("E{e}"),
        _ => String::new(),
    }
}

/// The two lines a list shows for a video: the show or title, then the
/// episode or "Movie · 2021".
fn lines(v: &Video) -> (String, String) {
    match v.kind {
        Kind::Episode => {
            let code = short_code(v);
            let name = v.episode_name();
            let second = if name.is_empty() || name == v.code() { code } else if code.is_empty() { name } else { format!("{code} · {name}") };
            (v.show.clone(), second)
        }
        Kind::Movie => (v.title.clone(), v.year.map(|y| format!("Movie · {y}")).unwrap_or_else(|| "Movie".into())),
        Kind::Other => {
            let folder = v.path.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            (v.title.clone(), if folder.is_empty() { "Home video".into() } else { folder })
        }
    }
}

/// When it joined the library: "Today", "Yesterday", "Mon", "Nov 2", "Nov 2, 2025".
fn when(added: i64, now: &glib::DateTime) -> String {
    let Ok(then) = glib::DateTime::from_unix_local(added) else { return String::new() };
    let day = |d: &glib::DateTime| glib::DateTime::from_local(d.year(), d.month(), d.day_of_month(), 0, 0, 0.0).ok();
    let days = match (day(now), day(&then)) {
        (Some(a), Some(b)) => (a.to_unix() - b.to_unix()).div_euclid(86_400),
        _ => i64::MAX,
    };
    let format = match days {
        0 => return "Today".into(),
        1 => return "Yesterday".into(),
        2..=6 => "%a",
        _ if then.year() == now.year() => "%b %-d",
        _ => "%b %-d, %Y",
    };
    then.format(format).map(|s| s.to_string()).unwrap_or_default()
}

/// A list's heading, with a count or a link at its right.
fn heading(title: &str, end: &gtk::Widget) -> gtk::Box {
    let row = widgets::hbox(8);
    row.add_css_class("home-heading");
    let t = widgets::label(title, "home-heading-title");
    t.set_hexpand(true);
    t.set_xalign(0.0);
    row.append(&t);
    end.set_valign(gtk::Align::Center);
    row.append(end);
    row
}

fn link(text: &str, go: impl Fn() + 'static) -> gtk::Button {
    let b = gtk::Button::with_label(text);
    b.add_css_class("flat");
    b.add_css_class("home-link");
    b.connect_clicked(move |_| go());
    b
}

fn list() -> gtk::Box {
    let b = widgets::vbox(0);
    b.add_css_class("card");
    b.add_css_class("home-list");
    b
}

/// A row: the frame with how far in, two lines, then whatever goes at the end.
/// Click plays it; right-click opens its menu.
fn video_row(v: &Rc<Video>) -> (gtk::Box, gtk::Box) {
    let row = widgets::hbox(12);
    row.add_css_class("home-row");
    let art = Art::new(104, 58, true, "nmp-video-symbolic");
    art.root.add_css_class("row-thumb");
    art.set_key(v.wide_art());
    let strip = views::progress_strip();
    views::set_progress(&strip, v.progress());
    art.root.add_overlay(&strip);
    row.append(&art.root);
    let (title, sub) = lines(v);
    let text = widgets::vbox(2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let t = widgets::label(&title, "row-title");
    t.set_xalign(0.0);
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&t);
    let s = widgets::label(&sub, "row-sub");
    s.set_xalign(0.0);
    s.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&s);
    row.append(&text);
    let end = widgets::hbox(10);
    end.set_valign(gtk::Align::Center);
    row.append(&end);
    row.set_tooltip_text(Some(&v.label()));
    row.set_cursor_from_name(Some("pointer"));
    let click = gtk::GestureClick::new();
    let v2 = v.clone();
    click.connect_released(move |g, n, _, _| {
        if n == 1 && g.current_button() == gtk::gdk::BUTTON_PRIMARY {
            views::play_video(&v2);
        }
    });
    row.add_controller(click);
    let v3 = v.clone();
    nyx_ui::menu::on_right_click(&row, move |w, x, y| crate::menu::video_menu(w, x, y, vec![v3.clone()], Vec::new()));
    (row, end)
}

fn faint(text: &str) -> gtk::Label {
    let l = widgets::label(text, "row-faint");
    l.add_css_class("mono");
    l
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

/// A recently added movie or video, or a show's new episodes counted once.
fn recent_row(v: &Rc<Video>, now: &glib::DateTime) -> gtk::Box {
    let (title, mut sub) = lines(v);
    let mut secs = v.duration;
    // A show's episodes added together count as one row.
    if v.kind == Kind::Episode
        && let Some(show) = store::show_of(v)
    {
        let batch: Vec<_> = show.episodes().filter(|e| (e.added - v.added).abs() < 86_400).collect();
        if batch.len() > 1 {
            let seasons: std::collections::BTreeSet<_> = batch.iter().map(|e| e.season).collect();
            sub = match (seasons.len(), v.season) {
                (1, Some(n)) if n > 0 => format!("Season {n} · {}", fmt::count(batch.len(), "episode", "episodes")),
                _ => fmt::count(batch.len(), "new episode", "new episodes"),
            };
            secs = batch.iter().map(|e| e.duration).sum();
        }
    }
    let row = widgets::hbox(12);
    row.add_css_class("recent-row");
    let text = widgets::vbox(1);
    text.set_hexpand(true);
    let t = widgets::label(&title, "recent-title");
    t.set_xalign(0.0);
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&t);
    let s = widgets::label(&sub, "recent-sub");
    s.set_xalign(0.0);
    s.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&s);
    row.append(&text);
    if secs > 0.0 {
        let d = widgets::label(&fmt::total(secs), "recent-length");
        d.add_css_class("mono");
        d.set_valign(gtk::Align::Center);
        row.append(&d);
    }
    let w = widgets::label(&when(v.added, now), "recent-when");
    w.set_width_chars(9);
    w.set_xalign(1.0);
    w.set_valign(gtk::Align::Center);
    row.append(&w);
    row.set_tooltip_text(Some(&v.label()));
    row.set_cursor_from_name(Some("pointer"));
    let click = gtk::GestureClick::new();
    let v2 = v.clone();
    click.connect_released(move |g, n, _, _| {
        if n != 1 || g.current_button() != gtk::gdk::BUTTON_PRIMARY {
            return;
        }
        match v2.kind {
            Kind::Episode => crate::window::navigate(&format!("tv:{}", v2.show)),
            _ => views::play_video(&v2),
        }
    });
    row.add_controller(click);
    let v3 = v.clone();
    nyx_ui::menu::on_right_click(&row, move |w, x, y| crate::menu::video_menu(w, x, y, vec![v3.clone()], Vec::new()));
    row
}

pub fn build(page: &Page) {
    page.body.append(&scan_banner());
    let cols = widgets::hbox(20);
    cols.add_css_class("home-cols");

    // ----- Continue watching and Next up -----
    let main = widgets::vbox(0);
    main.set_hexpand(true);
    main.add_css_class("home-main");
    let cont_count = widgets::label("", "home-count");
    cont_count.add_css_class("mono");
    let cont_head = heading("Continue watching", cont_count.upcast_ref());
    let cont = list();
    main.append(&cont_head);
    main.append(&cont);
    let following = link("Shows you're following", || crate::window::navigate("tv"));
    let next_head = heading("Next up", following.upcast_ref());
    next_head.add_css_class("later");
    let next = list();
    main.append(&next_head);
    main.append(&next);
    let nothing = widgets::empty_state(
        "nmp-home-symbolic",
        "Nothing to pick up yet",
        "Videos you start show up here with the time left, and the next episode of each show you're watching.",
        None,
    );
    nothing.add_css_class("home-nothing");
    main.append(&nothing);
    cols.append(&main);

    // ----- Recently added and the drop tip -----
    let side = widgets::vbox(0);
    side.add_css_class("home-side");
    side.set_size_request(340, -1);
    let see_all = link("See all", || {
        super::videos::sort_by("added");
        crate::window::navigate("videos");
    });
    let recent_head = heading("Recently added", see_all.upcast_ref());
    let recent = list();
    side.append(&recent_head);
    side.append(&recent);
    let tip = widgets::hbox(8);
    tip.add_css_class("home-tip");
    let icon = gtk::Image::from_icon_name("folder-symbolic");
    icon.set_valign(gtk::Align::Start);
    tip.append(&icon);
    let tip_text = gtk::Label::new(None);
    tip_text.set_markup("Drop videos, folders or M3U playlists here to play them; hold <b>Shift</b> to queue them.");
    tip_text.set_wrap(true);
    tip_text.set_xalign(0.0);
    tip_text.set_hexpand(true);
    tip.append(&tip_text);
    side.append(&tip);
    cols.append(&side);

    // Narrow windows stack Recently added under the lists.
    let follow = {
        let (cols, side) = (cols.clone(), side.clone());
        move |body: &gtk::Box| {
            let narrow = body.has_css_class("narrow");
            cols.set_orientation(if narrow { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal });
            side.set_size_request(if narrow { -1 } else { 340 }, -1);
        }
    };
    follow(&page.body);
    page.body.connect_css_classes_notify(follow);

    let empty = widgets::empty_state(
        "nmp-home-symbolic",
        "Nothing here yet",
        "Videos you start, shows you're following and what you add show up here.",
        None,
    );
    page.body.append(&library_stack(&cols, || !store::videos().is_empty(), empty));

    let fill = move || {
        let now = glib::DateTime::now_local().ok();
        for b in [&cont, &next, &recent] {
            clear(b);
        }
        let c = store::continue_watching();
        cont_count.set_text(&fmt::thousands(c.len()));
        for (i, v) in c.iter().take(CONTINUE).enumerate() {
            let (row, end) = video_row(v);
            end.append(&faint(&fmt::left(v.duration - v.watch.get().position)));
            let resume = widgets::labeled_button("media-playback-start-symbolic", "Resume");
            resume.add_css_class("resume");
            if i == 0 {
                row.add_css_class("lead");
                resume.add_css_class("suggested-action");
            }
            let v2 = v.clone();
            resume.connect_clicked(move |_| views::play_video(&v2));
            end.append(&resume);
            cont.append(&row);
        }
        cont_head.set_visible(!c.is_empty());
        cont.set_visible(!c.is_empty());
        let n = store::next_up();
        for v in n.iter().take(NEXT) {
            let (row, end) = video_row(v);
            if v.episode == Some(1) && v.season.is_some_and(|s| s > 1) {
                let tag = widgets::tag("New season", "accent");
                tag.set_valign(gtk::Align::Center);
                end.append(&tag);
            }
            if v.duration > 0.0 {
                end.append(&faint(&fmt::total(v.duration)));
            }
            next.append(&row);
        }
        next_head.set_visible(!n.is_empty());
        next.set_visible(!n.is_empty());
        if c.is_empty() {
            next_head.remove_css_class("later");
        } else {
            next_head.add_css_class("later");
        }
        nothing.set_visible(c.is_empty() && n.is_empty());
        if let Some(now) = &now {
            for v in store::recently_added().iter().take(RECENT) {
                recent.append(&recent_row(v, now));
            }
        }
    };
    let fill = Rc::new(fill);
    fill();
    // Positions move every few seconds while playing; rebuild when the page
    // is next shown rather than under the pointer.
    let dirty = Rc::new(Cell::new(false));
    let (f, d, body) = (fill.clone(), dirty.clone(), page.body.clone());
    store::subscribe(&page.body, move |c| match c {
        store::Change::Library => f(),
        store::Change::Watch if !body.is_mapped() => d.set(true),
        // Not while playing: positions move every few seconds.
        store::Change::Watch if crate::player::state() != crate::player::State::Playing => f(),
        _ => {}
    });
    page.body.connect_map(move |_| {
        if dirty.replace(false) {
            fill();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_rows() {
        let ep = Video {
            kind: Kind::Episode,
            show: "Orbit Station".into(),
            season: Some(2),
            episode: Some(5),
            title: "The Long Dark".into(),
            ..Default::default()
        };
        assert_eq!(lines(&ep), ("Orbit Station".into(), "S2 · E5 · The Long Dark".into()));
        let movie = Video { kind: Kind::Movie, title: "Kite Season".into(), year: Some(2024), ..Default::default() };
        assert_eq!(lines(&movie).1, "Movie · 2024");
    }

    #[test]
    fn says_when() {
        let now = glib::DateTime::from_local(2026, 11, 13, 9, 0, 0.0).unwrap();
        let at = |d: i32, h: i32| glib::DateTime::from_local(2026, 11, d, h, 0, 0.0).unwrap().to_unix();
        assert_eq!(when(at(13, 1), &now), "Today");
        assert_eq!(when(at(12, 23), &now), "Yesterday");
        assert_eq!(when(at(9, 12), &now), "Mon");
        assert_eq!(when(at(2, 12), &now), "Nov 2");
    }
}
