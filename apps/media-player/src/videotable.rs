//! The video table used by All videos, folders, playlists, the queue and
//! search: sortable columns, multi-select, a right-click menu, double-click to
//! play, and drag-and-drop onto playlists in the sidebar.

use crate::library::art::Art;
use crate::library::{Kind, Video, store};
use crate::sections::playlist;
use crate::{fmt, menu, player, widgets, window};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Col {
    /// A marker on the video that's playing.
    Indicator,
    /// Position in the list (playlists, queue); a marker when playing.
    Num,
    Title,
    Kind,
    /// The kind with a movie's year: "Movie · 2021".
    KindYear,
    Year,
    Time,
    Size,
    Added,
    /// Watched, or how far in.
    Status,
    /// Watched, a bar of how far in, or "Not yet".
    Watched,
}

impl Col {
    fn title(self) -> &'static str {
        match self {
            Col::Indicator | Col::Num => "",
            Col::Title => "Title",
            Col::Kind | Col::KindYear => "Kind",
            Col::Year => "Year",
            Col::Time => "Length",
            Col::Size => "Size",
            Col::Added => "Added",
            Col::Status => "Seen",
            Col::Watched => "Watched",
        }
    }

    fn hidden_when_narrow(self) -> bool {
        matches!(self, Col::Kind | Col::KindYear | Col::Year | Col::Size | Col::Added)
    }
}

pub struct Row {
    pub video: Rc<Video>,
    /// Position in the list given to `set` (queue index, playlist position).
    pub index: usize,
}

/// Extra menu items acting on the selected rows' indexes.
pub type Extra = (&'static str, Rc<dyn Fn(Vec<usize>)>);

pub struct Options {
    pub cols: &'static [Col],
    pub sortable: bool,
    /// Numbers in `Num` are positions (1, 2, 3…).
    pub positions: bool,
    /// Instead of playing the list from the activated row.
    pub on_activate: Option<Rc<dyn Fn(usize)>>,
    pub extra: Vec<Extra>,
    /// A small frame before each title.
    pub thumbs: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            cols: &[Col::Indicator, Col::Title, Col::Kind, Col::Year, Col::Time, Col::Added, Col::Status],
            sortable: true,
            positions: false,
            on_activate: None,
            extra: Vec::new(),
            thumbs: false,
        }
    }
}

#[derive(Clone)]
pub struct VideoTable {
    pub root: gtk::Box,
    pub view: gtk::ColumnView,
    store: gio::ListStore,
    sorted: gtk::SortListModel,
    selection: gtk::MultiSelection,
    columns: Rc<RefCell<Vec<(Col, gtk::ColumnViewColumn)>>>,
}

thread_local! {
    static TABLES: RefCell<Vec<glib::WeakRef<gtk::ColumnView>>> = const { RefCell::new(Vec::new()) };
}

fn row_of(obj: &glib::Object) -> Option<std::cell::Ref<'_, Row>> {
    obj.downcast_ref::<glib::BoxedAnyObject>().map(|b| b.borrow::<Row>())
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Movie => "Movie",
        Kind::Episode => "Episode",
        Kind::Other => "Home video",
    }
}

fn kind_year(v: &Video) -> String {
    match (v.kind, v.year) {
        (Kind::Movie, Some(y)) => format!("Movie · {y}"),
        (k, _) => kind_name(k).to_string(),
    }
}

/// Library order: shows by name then episode, everything else by title.
fn title_order(a: &Video, b: &Video) -> Ordering {
    let key = |v: &Video| {
        if v.kind == Kind::Episode {
            (store::sort_key(&v.show), v.season.unwrap_or(u32::MAX), v.episode.unwrap_or(u32::MAX))
        } else {
            (store::sort_key(&v.title), 0, 0)
        }
    };
    key(a).cmp(&key(b)).then_with(|| a.path.cmp(&b.path))
}

fn compare(col: Col, a: &Video, b: &Video) -> Ordering {
    match col {
        Col::Title => title_order(a, b),
        Col::Kind => kind_name(a.kind).cmp(kind_name(b.kind)).then_with(|| title_order(a, b)),
        Col::KindYear => kind_name(a.kind).cmp(kind_name(b.kind)).then_with(|| a.year.cmp(&b.year)).then_with(|| title_order(a, b)),
        Col::Year => a.year.cmp(&b.year).then_with(|| title_order(a, b)),
        Col::Time => a.duration.total_cmp(&b.duration),
        Col::Size => a.size.cmp(&b.size),
        Col::Added => a.added.cmp(&b.added).then_with(|| title_order(a, b)),
        Col::Status | Col::Watched => status_rank(a).total_cmp(&status_rank(b)),
        Col::Indicator | Col::Num => Ordering::Equal,
    }
}

fn status_rank(v: &Video) -> f64 {
    if v.watch.get().watched { 2.0 } else { v.progress() }
}

fn status(v: &Video) -> String {
    if v.watch.get().watched {
        "✓".into()
    } else if v.in_progress() {
        format!("{}%", (v.progress() * 100.0).round())
    } else {
        String::new()
    }
}

/// "Mar 4" this year, "Mar 4, 2025" before, from Unix seconds.
fn date(secs: i64) -> String {
    if secs <= 0 {
        return String::new();
    }
    let Ok(d) = glib::DateTime::from_unix_local(secs) else { return String::new() };
    let this_year = glib::DateTime::now_local().is_ok_and(|now| now.year() == d.year());
    d.format(if this_year { "%b %-d" } else { "%b %-d, %Y" }).map(|s| s.to_string()).unwrap_or_default()
}

/// The Watched cell: a tick and "Watched", a bar of how far in, or "Not yet".
fn watched_cell() -> gtk::Stack {
    let stack = gtk::Stack::new();
    stack.set_hhomogeneous(false);
    stack.set_halign(gtk::Align::Start);
    let done = widgets::hbox(5);
    done.add_css_class("cell-watched");
    done.append(&gtk::Image::from_icon_name("object-select-symbolic"));
    done.append(&gtk::Label::new(Some("Watched")));
    stack.add_named(&done, Some("watched"));
    let bar = gtk::ProgressBar::new();
    bar.add_css_class("cell-progress");
    bar.set_size_request(90, -1);
    bar.set_valign(gtk::Align::Center);
    stack.add_named(&bar, Some("started"));
    stack.add_named(&widgets::label("Not yet", "cell-faint"), Some("not-yet"));
    stack
}

fn bind_watched(stack: &gtk::Stack, v: &Video) {
    if v.watch.get().watched {
        stack.set_visible_child_name("watched");
    } else if v.in_progress() {
        if let Some(bar) = stack.child_by_name("started").and_downcast::<gtk::ProgressBar>() {
            bar.set_fraction(v.progress().max(0.04));
        }
        stack.set_visible_child_name("started");
    } else {
        stack.set_visible_child_name("not-yet");
    }
}

fn is_playing(v: &Video) -> bool {
    player::current().is_some_and(|c| c.path == v.path)
}

impl VideoTable {
    pub fn new(opts: Options) -> VideoTable {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let sorted = gtk::SortListModel::new(Some(store.clone()), None::<gtk::Sorter>);
        let selection = gtk::MultiSelection::new(Some(sorted.clone()));
        let view = gtk::ColumnView::new(Some(selection.clone()));
        view.add_css_class("video-table");
        view.set_reorderable(false);
        view.set_show_column_separators(false);
        let opts = Rc::new(opts);

        let table = VideoTable {
            root: widgets::vbox(0),
            view: view.clone(),
            store,
            sorted: sorted.clone(),
            selection,
            columns: Rc::default(),
        };

        let mut default_sort: Option<gtk::ColumnViewColumn> = None;
        for &col in opts.cols {
            let factory = gtk::SignalListItemFactory::new();
            let t = table.clone();
            let o = opts.clone();
            factory.connect_setup(move |_, item| {
                let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
                let child: gtk::Widget = match col {
                    Col::Indicator => {
                        let i = gtk::Image::from_icon_name("media-playback-start-symbolic");
                        i.add_css_class("accent-text");
                        i.upcast()
                    }
                    Col::Watched => watched_cell().upcast(),
                    Col::Title if o.thumbs => {
                        let line = widgets::hbox(12);
                        let art = Art::new(60, 34, true, "nmp-video-symbolic");
                        art.root.add_css_class("cell-thumb");
                        line.append(&art.root);
                        let l = widgets::label("", "cell-title");
                        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                        l.set_xalign(0.0);
                        l.set_hexpand(true);
                        line.append(&l);
                        // Kept with the row so bind can reach the picture.
                        unsafe { item.set_data("art", art) };
                        line.upcast()
                    }
                    _ => {
                        let l = widgets::label("", "");
                        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                        if matches!(col, Col::Kind | Col::KindYear | Col::Size | Col::Added) {
                            l.set_xalign(0.0);
                        }
                        if matches!(col, Col::Num | Col::Year | Col::Time | Col::Size | Col::Added | Col::Status) {
                            l.add_css_class("mono");
                        }
                        if matches!(col, Col::Num | Col::Year | Col::Time | Col::Added | Col::Status) && !o.thumbs {
                            l.add_css_class("mono");
                            l.add_css_class("cell-dim");
                            l.set_xalign(1.0);
                        }
                        l.upcast()
                    }
                };
                child.set_hexpand(true);
                t.attach_row_gestures(&child, item, &o);
                item.set_child(Some(&child));
            });
            let o = opts.clone();
            factory.connect_bind(move |_, item| {
                let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
                let (Some(obj), Some(child)) = (item.item(), item.child()) else { return };
                let Some(row) = row_of(&obj) else { return };
                let v = &row.video;
                if col == Col::Indicator {
                    child.set_opacity(if is_playing(v) { 1.0 } else { 0.0 });
                    return;
                }
                if let Some(stack) = child.downcast_ref::<gtk::Stack>() {
                    bind_watched(stack, v);
                    return;
                }
                if let Some(art) = unsafe { item.data::<Art>("art") } {
                    unsafe { art.as_ref() }.set_key(v.wide_art());
                }
                let label = match child.downcast_ref::<gtk::Label>() {
                    Some(l) => Some(l.clone()),
                    None => child.last_child().and_downcast::<gtk::Label>(),
                };
                let Some(l) = label.as_ref() else { return };
                let text = match col {
                    Col::Num => {
                        if is_playing(v) {
                            "▶".to_string()
                        } else if o.positions {
                            (row.index + 1).to_string()
                        } else {
                            String::new()
                        }
                    }
                    // With a Kind column beside it, the year is there already.
                    Col::Title if o.thumbs && v.kind == Kind::Movie => v.title.clone(),
                    Col::Title => v.label(),
                    Col::Kind => kind_name(v.kind).to_string(),
                    Col::KindYear => kind_year(v),
                    Col::Year => v.year.map(|y| y.to_string()).unwrap_or_default(),
                    Col::Time => {
                        if v.duration > 0.0 {
                            fmt::time(v.duration)
                        } else {
                            String::new()
                        }
                    }
                    Col::Size => {
                        if v.size > 0 {
                            fmt::size(v.size)
                        } else {
                            String::new()
                        }
                    }
                    Col::Added => date(v.added),
                    Col::Status => status(v),
                    Col::Indicator | Col::Watched => String::new(),
                };
                l.set_text(&text);
                if col == Col::Num || col == Col::Title {
                    if is_playing(v) {
                        l.add_css_class("accent-text");
                    } else {
                        l.remove_css_class("accent-text");
                    }
                }
                if col == Col::Title {
                    l.set_tooltip_text(Some(&v.path.to_string_lossy()));
                }
            });
            let c = gtk::ColumnViewColumn::new(Some(col.title()), Some(factory));
            c.set_resizable(!matches!(col, Col::Indicator | Col::Num));
            match col {
                Col::Indicator => c.set_fixed_width(34),
                Col::Num => c.set_fixed_width(48),
                Col::Kind => c.set_fixed_width(90),
                Col::KindYear => c.set_fixed_width(128),
                Col::Year => c.set_fixed_width(70),
                Col::Time if opts.thumbs => c.set_fixed_width(96),
                Col::Time => c.set_fixed_width(84),
                Col::Size => c.set_fixed_width(82),
                Col::Added if opts.thumbs => c.set_fixed_width(100),
                Col::Added => c.set_fixed_width(120),
                Col::Status => c.set_fixed_width(64),
                Col::Watched => c.set_fixed_width(120),
                Col::Title => c.set_expand(true),
            }
            if opts.sortable && !matches!(col, Col::Indicator | Col::Num) {
                c.set_sorter(Some(&gtk::CustomSorter::new(move |a, b| match (row_of(a), row_of(b)) {
                    (Some(a), Some(b)) => compare(col, &a.video, &b.video).into(),
                    _ => gtk::Ordering::Equal,
                })));
                if col == Col::Title {
                    default_sort = Some(c.clone());
                }
            }
            view.append_column(&c);
            table.columns.borrow_mut().push((col, c));
        }
        if opts.sortable {
            sorted.set_sorter(view.sorter().as_ref());
            if let Some(c) = default_sort {
                view.sort_by_column(Some(&c), gtk::SortType::Ascending);
            }
        }

        let t = table.clone();
        let o = opts.clone();
        view.connect_activate(move |_, pos| t.activate(pos, &o));

        // Q adds the selected videos to the queue; W marks them watched (or not).
        let keys = gtk::EventControllerKey::new();
        let t = table.clone();
        keys.connect_key_pressed(move |_, key, _, mods| {
            if !mods.is_empty() {
                return glib::Propagation::Proceed;
            }
            let videos = t.selected();
            if videos.is_empty() {
                return glib::Propagation::Proceed;
            }
            match key {
                gdk::Key::q | gdk::Key::Q => menu::enqueue(videos.iter().map(|v| v.path.clone()).collect()),
                gdk::Key::w | gdk::Key::W => menu::toggle_watched(&videos),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        view.add_controller(keys);

        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .child(&view)
            .vexpand(true)
            .build();
        table.root.add_css_class("table-card");
        table.root.set_overflow(gtk::Overflow::Hidden);
        table.root.set_vexpand(true);
        table.root.append(&scroll);

        // Re-bind when the video changes, so the playing marker follows, and
        // when watched marks change.
        let weak = table.store.downgrade();
        player::subscribe(&view, move |e| {
            if e == player::Event::Track
                && let Some(s) = weak.upgrade()
            {
                let n = s.n_items();
                s.items_changed(0, n, n);
            }
        });
        let weak = table.store.downgrade();
        store::subscribe(&view, move |c| {
            if c == store::Change::Watch
                && let Some(s) = weak.upgrade()
            {
                let n = s.n_items();
                s.items_changed(0, n, n);
            }
        });
        TABLES.with(|t| t.borrow_mut().push(view.downgrade()));
        apply_narrow(&view, window::narrow());
        table
    }

    pub fn set(&self, videos: &[Rc<Video>]) {
        let objs: Vec<glib::BoxedAnyObject> =
            videos.iter().enumerate().map(|(index, v)| glib::BoxedAnyObject::new(Row { video: v.clone(), index })).collect();
        self.store.splice(0, self.store.n_items(), &objs);
    }

    pub fn len(&self) -> u32 {
        self.store.n_items()
    }

    /// Sort by a column, as clicking its heading would.
    pub fn sort_by(&self, col: Col, order: gtk::SortType) {
        let c = self.columns.borrow().iter().find(|(k, _)| *k == col).map(|(_, c)| c.clone());
        // Only this column: the earlier sorts would otherwise stay on as tie-breakers, arrows and all.
        self.view.sort_by_column(None, order);
        self.view.sort_by_column(c.as_ref(), order);
    }

    /// The selected videos, in view order.
    fn selected(&self) -> Vec<Rc<Video>> {
        let set = self.selection.selection();
        let mut out = Vec::new();
        if let Some((iter, first)) = gtk::BitsetIter::init_first(&set) {
            for i in std::iter::once(first).chain(iter) {
                if let Some(v) = self.sorted.item(i).and_then(|o| row_of(&o).map(|r| r.video.clone())) {
                    out.push(v);
                }
            }
        }
        out
    }

    /// Paths in the order they're shown.
    pub fn paths(&self) -> Vec<PathBuf> {
        (0..self.sorted.n_items())
            .filter_map(|i| self.sorted.item(i))
            .filter_map(|o| row_of(&o).map(|r| r.video.path.clone()))
            .collect()
    }

    pub fn focus(&self) {
        if self.sorted.n_items() > 0 {
            self.view.scroll_to(0, None, gtk::ListScrollFlags::FOCUS | gtk::ListScrollFlags::SELECT, None);
        }
        self.view.grab_focus();
    }

    fn activate(&self, pos: u32, opts: &Options) {
        let Some(obj) = self.sorted.item(pos) else { return };
        let Some((index, video)) = row_of(&obj).map(|r| (r.index, r.video.clone())) else { return };
        if let Some(f) = &opts.on_activate {
            f(index);
        } else if opts.positions {
            player::play_paths(self.paths(), pos as usize);
            window::navigate("now-playing");
        } else {
            crate::views::play_video(&video);
        }
    }

    /// Selected rows (in view order) — or just the clicked one if it isn't selected.
    fn targets(&self, clicked: u32) -> Vec<(u32, Rc<Video>, usize)> {
        let collect = |i: u32| self.sorted.item(i).and_then(|o| row_of(&o).map(|r| (i, r.video.clone(), r.index)));
        if !self.selection.is_selected(clicked) {
            self.selection.select_item(clicked, true);
            return collect(clicked).into_iter().collect();
        }
        let set = self.selection.selection();
        let mut out = Vec::new();
        if let Some((iter, first)) = gtk::BitsetIter::init_first(&set) {
            out.extend(collect(first));
            for i in iter {
                out.extend(collect(i));
            }
        }
        out
    }

    fn attach_row_gestures(&self, child: &gtk::Widget, item: &gtk::ListItem, opts: &Rc<Options>) {
        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_SECONDARY);
        let t = self.clone();
        let o = opts.clone();
        let item_weak = item.downgrade();
        let w = child.downgrade();
        click.connect_pressed(move |_, _, x, y| {
            let (Some(item), Some(w)) = (item_weak.upgrade(), w.upgrade()) else { return };
            let pos = item.position();
            if pos == gtk::INVALID_LIST_POSITION {
                return;
            }
            let targets = t.targets(pos);
            let videos: Vec<Rc<Video>> = targets.iter().map(|(_, v, _)| v.clone()).collect();
            let indexes: Vec<usize> = targets.iter().map(|(_, _, i)| *i).collect();
            let extra: Vec<menu::Extra> = o
                .extra
                .iter()
                .map(|(label, f)| {
                    let (f, idx) = (f.clone(), indexes.clone());
                    (*label, Rc::new(move || f(idx.clone())) as Rc<dyn Fn()>)
                })
                .collect();
            menu::video_menu(&w, x, y, videos, extra);
        });
        child.add_controller(click);

        // Drag videos onto a playlist in the sidebar.
        let drag = gtk::DragSource::new();
        drag.set_actions(gdk::DragAction::COPY);
        let t = self.clone();
        let item_weak = item.downgrade();
        drag.connect_prepare(move |_, _, _| {
            let item = item_weak.upgrade()?;
            let pos = item.position();
            if pos == gtk::INVALID_LIST_POSITION {
                return None;
            }
            let paths: Vec<String> = t.targets(pos).into_iter().map(|(_, v, _)| v.path.to_string_lossy().into_owned()).collect();
            Some(gdk::ContentProvider::for_value(&playlist::drag_payload(&paths).to_value()))
        });
        child.add_controller(drag);
    }
}

fn apply_narrow(view: &gtk::ColumnView, narrow: bool) {
    let cols = view.columns();
    for i in 0..cols.n_items() {
        if let Some(c) = cols.item(i).and_downcast::<gtk::ColumnViewColumn>() {
            let title = c.title().map(|t| t.to_string()).unwrap_or_default();
            if [Col::Kind, Col::KindYear, Col::Year, Col::Size, Col::Added].iter().any(|col| col.hidden_when_narrow() && col.title() == title) {
                c.set_visible(!narrow);
            }
        }
    }
}

/// Hide the less important columns in a half-screen window.
pub fn set_narrow(narrow: bool) {
    TABLES.with(|t| {
        t.borrow_mut().retain(|w| w.upgrade().is_some());
        for w in t.borrow().iter() {
            if let Some(v) = w.upgrade() {
                apply_narrow(&v, narrow);
            }
        }
    });
}
