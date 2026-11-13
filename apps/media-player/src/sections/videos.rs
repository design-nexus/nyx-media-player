//! All videos: one sortable table of everything, filtered by kind and to what
//! you haven't watched, with how far you've got in each.

use super::{library_stack, scan_banner};
use crate::fmt;
use crate::library::{Kind, Video, store};
use crate::videotable::{Col, Options, VideoTable};
use crate::widgets::{self, Page};
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

thread_local! {
    static FILTER: RefCell<String> = RefCell::new("all".into());
    static UNWATCHED: Cell<bool> = const { Cell::new(false) };
    /// The sort to apply once the page is built ("See all" on Home).
    static WANT_SORT: RefCell<Option<String>> = const { RefCell::new(None) };
    static TABLE: RefCell<Option<(VideoTable, gtk::DropDown)>> = const { RefCell::new(None) };
}

const SORTS: &[(&str, &str)] =
    &[("added", "Date added"), ("title", "Title"), ("length", "Length"), ("size", "Size"), ("kind", "Kind"), ("watched", "Watched")];

fn apply_sort(table: &VideoTable, id: &str) {
    let (col, order) = match id {
        "title" => (Col::Title, gtk::SortType::Ascending),
        "length" => (Col::Time, gtk::SortType::Descending),
        "size" => (Col::Size, gtk::SortType::Descending),
        "kind" => (Col::KindYear, gtk::SortType::Ascending),
        "watched" => (Col::Watched, gtk::SortType::Descending),
        _ => (Col::Added, gtk::SortType::Descending),
    };
    table.sort_by(col, order);
}

/// Show All videos sorted a given way (`added`, `title`…).
pub fn sort_by(id: &str) {
    match TABLE.with(|t| t.borrow().clone()) {
        Some((_, dropdown)) => {
            if let Some(i) = SORTS.iter().position(|(k, _)| *k == id) {
                dropdown.set_selected(i as u32);
            }
        }
        None => WANT_SORT.with(|w| *w.borrow_mut() = Some(id.to_string())),
    }
}

/// "1,204 videos · 2,140 hours"
fn summary(videos: &[Rc<Video>]) -> String {
    let secs: f64 = videos.iter().map(|v| v.duration).sum();
    let mut text = fmt::count(videos.len(), "video", "videos");
    if secs >= 36_000.0 {
        text.push_str(&format!(" · {} hours", fmt::thousands((secs / 3600.0).round() as usize)));
    } else if secs > 0.0 {
        text.push_str(&format!(" · {}", fmt::total(secs)));
    }
    text
}

pub fn build(page: &Page) {
    page.body.append(&scan_banner());
    let content = widgets::vbox(0);
    // The kind filter and Only unwatched; how many and the sort at the right.
    let toolbar = widgets::hbox(10);
    toolbar.add_css_class("toolbar");
    toolbar.add_css_class("page-head");
    let table = VideoTable::new(Options {
        cols: &[Col::Title, Col::KindYear, Col::Time, Col::Size, Col::Added, Col::Watched],
        thumbs: true,
        ..Default::default()
    });
    table.root.add_css_class("all-videos");
    let count = widgets::label("", "page-head-count");
    let refresh = {
        let (table, count) = (table.clone(), count.clone());
        Rc::new(move || {
            let filter = FILTER.with(|f| f.borrow().clone());
            let unwatched = UNWATCHED.with(|u| u.get());
            let shown: Vec<_> = store::videos()
                .into_iter()
                .filter(|v| match filter.as_str() {
                    "movies" => v.kind == Kind::Movie,
                    "episodes" => v.kind == Kind::Episode,
                    "other" => v.kind == Kind::Other,
                    _ => true,
                })
                .filter(|v| !unwatched || !v.watch.get().watched)
                .collect();
            count.set_text(&summary(&shown));
            table.set(&shown);
        })
    };
    let r = refresh.clone();
    let kinds = widgets::segmented(
        &widgets::opts(&[("all", "All"), ("movies", "Movies"), ("episodes", "Episodes"), ("other", "Home videos")]),
        &FILTER.with(|f| f.borrow().clone()),
        move |id| {
            FILTER.with(|f| *f.borrow_mut() = id);
            r();
        },
    );
    toolbar.append(&kinds);
    let only = widgets::hbox(8);
    only.add_css_class("only-unwatched");
    let switch = gtk::Switch::new();
    switch.set_valign(gtk::Align::Center);
    switch.set_active(UNWATCHED.with(|u| u.get()));
    switch.update_property(&[gtk::accessible::Property::Label("Only unwatched")]);
    let r = refresh.clone();
    switch.connect_active_notify(move |s| {
        UNWATCHED.with(|u| u.set(s.is_active()));
        r();
    });
    only.append(&switch);
    only.append(&widgets::label("Only unwatched", "only-unwatched-label"));
    toolbar.append(&only);
    let spacer = widgets::hbox(0);
    spacer.set_hexpand(true);
    toolbar.append(&spacer);
    toolbar.append(&count);
    let sort_opts = widgets::opts(SORTS);
    let start = WANT_SORT.with(|w| w.borrow_mut().take()).unwrap_or_else(|| "added".into());
    let sort = widgets::dropdown(&sort_opts, &start);
    sort.add_css_class("sort-dropdown");
    sort.set_tooltip_text(Some("Sort"));
    let t = table.clone();
    sort.connect_selected_notify(move |d| {
        if let Some((id, _)) = sort_opts.get(d.selected() as usize) {
            apply_sort(&t, id);
        }
    });
    toolbar.append(&sort);
    apply_sort(&table, &start);
    content.append(&toolbar);
    content.append(&table.root);
    TABLE.with(|t| *t.borrow_mut() = Some((table, sort)));
    let empty = widgets::vbox(0);
    page.body.append(&library_stack(&content, || true, empty));
    refresh();
    store::subscribe(&content, move |c| {
        // Positions move every few seconds while playing; watched marks are what matter here.
        let marks = c == store::Change::Watch && UNWATCHED.with(|u| u.get()) && crate::player::state() != crate::player::State::Playing;
        if c == store::Change::Library || marks {
            refresh();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_up_the_library() {
        let v = |secs: f64| Rc::new(Video { duration: secs, ..Default::default() });
        assert_eq!(summary(&[v(1800.0), v(1800.0)]), "2 videos · 1 h 0 min");
        assert_eq!(summary(&vec![v(3600.0); 1204]), "1,204 videos · 1,204 hours");
    }
}
