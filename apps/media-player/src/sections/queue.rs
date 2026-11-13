//! Queue: what's playing now and next. Double-click jumps; the menu removes.

use crate::library::store;
use crate::videotable::{Col, Options, VideoTable};
use crate::widgets::{self, Page};
use crate::{fmt, player, window};
use gtk::prelude::*;
use std::rc::Rc;

/// Move the selected videos up or down one place, keeping their order.
fn move_by(idx: &[usize], delta: i64) {
    let len = player::queue_items().0.len();
    let mut idx = idx.to_vec();
    idx.sort_unstable();
    if delta > 0 {
        idx.reverse();
    }
    for i in idx {
        let to = i as i64 + delta;
        if to < 0 || to >= len as i64 {
            return;
        }
        player::move_item(i, to as usize);
    }
}

pub fn build(page: &Page) {
    let toolbar = widgets::hbox(10);
    toolbar.add_css_class("toolbar");
    let summary = widgets::label("", "dim");
    summary.add_css_class("mono");
    summary.set_hexpand(true);
    summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
    toolbar.append(&summary);
    let save = nyx_ui::widgets::bar_button("nmp-playlist-symbolic", "Save as playlist");
    save.connect_clicked(|_| {
        let (items, _) = player::queue_items();
        if !items.is_empty() {
            super::playlist::new_dialog(items);
        }
    });
    toolbar.append(&save);
    let shuffle = gtk::ToggleButton::new();
    shuffle.set_icon_name("media-playlist-shuffle-symbolic");
    shuffle.add_css_class("flat");
    shuffle.add_css_class("icon-button");
    shuffle.add_css_class("transport-toggle");
    shuffle.set_tooltip_text(Some("Shuffle"));
    shuffle.set_active(player::shuffle());
    shuffle.connect_toggled(|b| {
        if b.is_active() != player::shuffle() {
            player::set_shuffle(b.is_active());
        }
    });
    toolbar.append(&shuffle);
    let repeat_opts = widgets::opts(&[("off", "No repeat"), ("all", "Repeat all"), ("one", "Repeat one")]);
    let repeat = widgets::dropdown(&repeat_opts, player::repeat().id());
    repeat.set_tooltip_text(Some("Repeat"));
    repeat.connect_selected_notify(move |d| {
        if let Some((id, _)) = repeat_opts.get(d.selected() as usize) {
            player::set_repeat(crate::player::queue::Repeat::from_id(id));
        }
    });
    toolbar.append(&repeat);
    let clear = widgets::two_click("Clear", "Click again to clear", player::clear);
    toolbar.append(&clear);
    page.body.append(&toolbar);

    let table = VideoTable::new(Options {
        cols: &[Col::Num, Col::Title, Col::Time, Col::Status],
        sortable: false,
        positions: true,
        on_activate: Some(Rc::new(player::jump)),
        extra: vec![
            (
                "Remove from queue",
                Rc::new(|mut idx: Vec<usize>| {
                    // Highest first, so earlier removals don't shift later ones.
                    idx.sort_unstable_by(|a, b| b.cmp(a));
                    for i in idx {
                        player::remove(i);
                    }
                }),
            ),
            ("Move up", Rc::new(|idx: Vec<usize>| move_by(&idx, -1))),
            ("Move down", Rc::new(|idx: Vec<usize>| move_by(&idx, 1))),
        ],
        thumbs: false,
    });
    let empty = widgets::empty_state(
        "nmp-queue-symbolic",
        "The queue is empty",
        "Play a show or a playlist, or right-click videos and choose <b>Add to queue</b>.",
        None,
    );
    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    stack.add_named(&table.root, Some("table"));
    stack.add_named(&empty, Some("empty"));
    page.body.append(&stack);

    let refresh = {
        let (table, stack) = (table.clone(), stack.clone());
        move || {
            let (items, cursor) = player::queue_items();
            let tracks: Vec<_> = items.iter().map(|p| store::video_for(p)).collect();
            let secs: f64 = tracks.iter().map(|t| t.duration).sum();
            let left: f64 = cursor.map_or(secs, |c| tracks.iter().skip(c).map(|t| t.duration).sum());
            summary.set_text(&if tracks.is_empty() {
                String::new()
            } else {
                format!("{} · {} · {} left", fmt::count(tracks.len(), "video", "videos"), fmt::total(secs), fmt::total(left))
            });
            table.set(&tracks);
            save.set_sensitive(!tracks.is_empty());
            clear.set_sensitive(!tracks.is_empty());
            stack.set_visible_child_name(if tracks.is_empty() { "empty" } else { "table" });
            if let Some(c) = cursor
                && window::current() == "queue"
                && (c as u32) < table.len()
            {
                table.view.scroll_to(c as u32, None, gtk::ListScrollFlags::NONE, None);
            }
        }
    };
    refresh();
    let r = refresh.clone();
    player::subscribe(&page.body, move |e| {
        if matches!(e, player::Event::Queue | player::Event::Track) {
            r();
        }
    });
}
