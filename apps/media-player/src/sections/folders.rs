//! Folders: the library as it's laid out on disk. The path runs across the top;
//! a folder lists its folders, then the videos directly in it with how far
//! you've got in each. The sidebar lists the top folders.

use super::{library_stack, scan_banner};
use crate::library::art::Art;
use crate::library::{Kind, Video, store};
use crate::widgets::{self, Page};
use crate::{fmt, menu, paths, player, views, window};
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub struct Node {
    pub path: PathBuf,
    pub name: String,
    pub children: Vec<Rc<Node>>,
    /// Videos in this folder and below.
    pub count: usize,
}

/// Build the folder tree under each library root from the videos' paths.
pub fn tree(roots: &[PathBuf], videos: &[Rc<Video>]) -> Vec<Rc<Node>> {
    fn build(dir: &Path, name: String, counts: &BTreeMap<PathBuf, usize>) -> Node {
        // Direct subfolders that hold videos somewhere below.
        let mut subs: BTreeMap<PathBuf, ()> = BTreeMap::new();
        for d in counts.keys() {
            if let Ok(rest) = d.strip_prefix(dir)
                && let Some(first) = rest.components().next()
            {
                subs.insert(dir.join(first), ());
            }
        }
        let count = counts.iter().filter(|(d, _)| d.starts_with(dir)).map(|(_, n)| n).sum();
        let mut children: Vec<Rc<Node>> = subs
            .into_keys()
            .map(|p| {
                let n = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                Rc::new(build(&p, n, counts))
            })
            .collect();
        children.sort_by_key(|c| c.name.to_lowercase());
        Node { path: dir.to_path_buf(), name, children, count }
    }
    let mut counts: BTreeMap<PathBuf, usize> = BTreeMap::new();
    for t in videos {
        if let Some(d) = t.path.parent() {
            *counts.entry(d.to_path_buf()).or_default() += 1;
        }
    }
    roots.iter().map(|r| Rc::new(build(r, folder_name(r), &counts))).filter(|n| n.count > 0).collect()
}

/// A folder's own name ("Videos"), or the whole path for `/`.
fn folder_name(dir: &Path) -> String {
    dir.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| paths::pretty(dir))
}

/// The folders the sidebar lists: the folders inside the one library folder, or
/// each library folder when there are several (or the one has no folders inside).
pub fn top_folders() -> Vec<Rc<Node>> {
    let nodes = tree(&store::roots(), &store::videos());
    match nodes.as_slice() {
        [only] if !only.children.is_empty() => only.children.clone(),
        _ => nodes,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Sort {
    Name,
    Added,
    Length,
}

thread_local! {
    static BROWSER: RefCell<Option<Rc<Browser>>> = const { RefCell::new(None) };
    /// A folder (and a file in it) asked for before the page was built.
    static WANT: RefCell<Option<(Option<PathBuf>, Option<PathBuf>)>> = const { RefCell::new(None) };
}

/// Open the Folders page at `dir`, picking out `file` in it.
fn open_at(dir: Option<PathBuf>, file: Option<PathBuf>) {
    match BROWSER.with(|b| b.borrow().clone()) {
        Some(b) => {
            *b.focus.borrow_mut() = file;
            b.go(dir);
        }
        None => WANT.with(|w| *w.borrow_mut() = Some((dir, file))),
    }
    window::navigate("folders");
}

/// Open the Folders page at a folder.
pub fn show_dir(dir: &Path) {
    open_at(Some(dir.to_path_buf()), None);
}

/// Open the folder a video is in, with the video picked out.
pub fn show_file(path: &Path) {
    open_at(path.parent().map(Path::to_path_buf), Some(path.to_path_buf()));
}

pub fn build(page: &Page) {
    page.body.append(&scan_banner());
    let browser = Browser::new();
    if let Some((dir, file)) = WANT.with(|w| w.borrow_mut().take()) {
        *browser.focus.borrow_mut() = file;
        browser.go(dir);
    }
    let card = widgets::vbox(0);
    card.add_css_class("folder-card");
    card.append(&browser.root);
    page.body.append(&library_stack(&card, || true, widgets::vbox(0)));
    BROWSER.with(|b| *b.borrow_mut() = Some(browser));
}

pub struct Browser {
    pub root: gtk::Box,
    crumbs: gtk::Box,
    /// Play, Shuffle and Add, for the videos directly in the folder.
    actions: Vec<gtk::Button>,
    rows: gtk::Box,
    scroll: gtk::ScrolledWindow,
    /// The folder shown; `None` is the top.
    current: RefCell<Option<PathBuf>>,
    /// A file to pick out once it's drawn.
    focus: RefCell<Option<PathBuf>>,
    sort: Cell<Sort>,
    /// The videos directly in it, in the order shown.
    open: Rc<RefCell<Vec<PathBuf>>>,
    /// Positions moved while it was hidden or playing: redraw when shown.
    dirty: Cell<bool>,
}

impl Browser {
    pub fn new() -> Rc<Browser> {
        let root = widgets::vbox(0);
        root.set_vexpand(true);
        let path = widgets::hbox(4);
        path.add_css_class("folder-path");
        let crumbs = widgets::hbox(2);
        crumbs.set_hexpand(true);
        crumbs.set_overflow(gtk::Overflow::Hidden);
        path.append(&crumbs);
        let open: Rc<RefCell<Vec<PathBuf>>> = Rc::default();
        let play = nyx_ui::widgets::bar_button("media-playback-start-symbolic", "Play the folder");
        let o = open.clone();
        play.connect_clicked(move |_| {
            player::set_shuffle(false);
            player::play_paths(o.borrow().clone(), 0);
            window::navigate("now-playing");
        });
        let shuffle = nyx_ui::widgets::bar_button("media-playlist-shuffle-symbolic", "Shuffle the folder");
        let o = open.clone();
        shuffle.connect_clicked(move |_| {
            player::shuffle_paths(o.borrow().clone());
            window::navigate("now-playing");
        });
        let queue = nyx_ui::widgets::bar_button("list-add-symbolic", "Add the folder to the queue");
        let o = open.clone();
        queue.connect_clicked(move |_| menu::enqueue(o.borrow().clone()));
        let sort = nyx_ui::widgets::bar_button("view-sort-descending-symbolic", "Sort");
        for b in [&play, &shuffle, &queue, &sort] {
            b.set_valign(gtk::Align::Center);
            path.append(b);
        }
        let rows = widgets::vbox(2);
        rows.add_css_class("folder-list");
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .vexpand(true)
            .child(&rows)
            .build();
        root.append(&path);
        root.append(&scroll);
        // Developer aid, like NMP_SNAPSHOT: NMP_FOLDER=<path> opens the view at that folder.
        let start = std::env::var_os("NMP_FOLDER").map(PathBuf::from);
        let b = Rc::new(Browser {
            root,
            crumbs,
            actions: vec![play, shuffle, queue],
            rows,
            scroll,
            current: RefCell::new(start),
            focus: RefCell::new(None),
            sort: Cell::new(Sort::Name),
            open,
            dirty: Cell::new(false),
        });
        let weak = Rc::downgrade(&b);
        sort.connect_clicked(move |button| {
            let Some(me) = weak.upgrade() else { return };
            let now = me.sort.get();
            let entry = |label: &str, s: Sort| {
                let me = me.clone();
                nyx_ui::menu::check(label, now == s, move |_| {
                    me.sort.set(s);
                    me.show();
                })
            };
            nyx_ui::menu::popup_below(
                button,
                vec![
                    nyx_ui::menu::heading("Sort by"),
                    entry("Name", Sort::Name),
                    entry("Newest first", Sort::Added),
                    entry("Longest first", Sort::Length),
                ],
            );
        });
        let weak = Rc::downgrade(&b);
        store::subscribe(&b.root, move |c| {
            let Some(b) = weak.upgrade() else { return };
            match c {
                store::Change::Library => b.show(),
                // Positions move every few seconds while playing; redraw later.
                store::Change::Watch if !b.root.is_mapped() || player::state() == player::State::Playing => b.dirty.set(true),
                store::Change::Watch => b.show(),
                _ => {}
            }
        });
        let weak = Rc::downgrade(&b);
        player::subscribe(&b.root, move |e| {
            if e == player::Event::Track
                && let Some(b) = weak.upgrade()
            {
                if b.root.is_mapped() { b.show() } else { b.dirty.set(true) }
            }
        });
        let weak = Rc::downgrade(&b);
        b.root.connect_map(move |_| {
            if let Some(b) = weak.upgrade()
                && b.dirty.replace(false)
            {
                b.show();
            }
        });
        b.show();
        b
    }

    fn go(self: &Rc<Self>, dir: Option<PathBuf>) {
        *self.current.borrow_mut() = dir;
        self.scroll.vadjustment().set_value(0.0);
        self.show();
    }

    /// Draw the folder being shown.
    pub fn show(self: &Rc<Self>) {
        let videos = store::videos();
        let nodes = tree(&store::roots(), &videos);
        let single = (nodes.len() == 1).then(|| nodes[0].clone());
        let current = self.current.borrow().clone();
        let mut chain = current.as_deref().map(|dir| chain_to(&nodes, dir)).unwrap_or_default();
        // Gone after a rescan: back to the top (not while the library is still loading).
        if current.is_some() && chain.is_empty() && store::loaded() {
            *self.current.borrow_mut() = None;
            return self.show();
        }
        // With one library folder, that folder is the top.
        if single.is_some() && !chain.is_empty() {
            chain.remove(0);
        }
        window::mark_folder(chain.first().map(|n| n.path.as_path()).or(single.as_ref().map(|n| n.path.as_path())));

        // The path: the library folder (or all of them), then each folder down to this one.
        clear(&self.crumbs);
        let crumb = |text: &str, target: Option<PathBuf>, last: bool| -> gtk::Widget {
            if last {
                return widgets::label(text, "folder-crumb-here").upcast();
            }
            let b = gtk::Button::with_label(text);
            b.add_css_class("flat");
            b.add_css_class("folder-crumb");
            let me = self.clone();
            b.connect_clicked(move |_| me.go(target.clone()));
            b.upcast()
        };
        let top = single.as_ref().map(|n| n.name.clone()).unwrap_or_else(|| "Library folders".into());
        self.crumbs.append(&crumb(&top, None, chain.is_empty()));
        for (i, node) in chain.iter().enumerate() {
            let sep = gtk::Image::from_icon_name("pan-end-symbolic");
            sep.add_css_class("folder-crumb-sep");
            self.crumbs.append(&sep);
            self.crumbs.append(&crumb(&node.name, Some(node.path.clone()), i + 1 == chain.len()));
        }

        // The folders here, then the videos directly in it.
        clear(&self.rows);
        let children = match (chain.last(), &single) {
            (Some(n), _) => n.children.clone(),
            (None, Some(only)) => only.children.clone(),
            (None, None) => nodes.clone(),
        };
        for node in &children {
            let secs: f64 = videos.iter().filter(|v| v.path.starts_with(&node.path)).map(|v| v.duration).sum();
            self.rows.append(&self.folder_row(node, secs));
        }
        let dir = chain.last().map(|n| n.path.clone()).or_else(|| single.map(|n| n.path.clone()));
        let mut here: Vec<Rc<Video>> = match &dir {
            Some(dir) => videos.iter().filter(|v| v.path.parent() == Some(dir.as_path())).cloned().collect(),
            None => Vec::new(),
        };
        match self.sort.get() {
            Sort::Name => here.sort_by(|a, b| a.path.cmp(&b.path)),
            Sort::Added => here.sort_by_key(|v| std::cmp::Reverse(v.added)),
            Sort::Length => here.sort_by(|a, b| b.duration.total_cmp(&a.duration)),
        }
        let playing = player::current().map(|v| v.path.clone());
        let focus = self.focus.borrow_mut().take();
        // Picked out: the file asked for, else the one playing, else the last one started.
        let picked = focus
            .clone()
            .or_else(|| playing.clone().filter(|p| here.iter().any(|v| v.path == *p)))
            .or_else(|| here.iter().filter(|v| v.in_progress()).max_by_key(|v| v.watch.get().last_played).map(|v| v.path.clone()));
        let paths_here: Vec<PathBuf> = here.iter().map(|v| v.path.clone()).collect();
        *self.open.borrow_mut() = paths_here.clone();
        for b in &self.actions {
            b.set_sensitive(!paths_here.is_empty());
        }
        let mut picked_row = None;
        for (i, v) in here.iter().enumerate() {
            let row = file_row(v, playing.as_ref() == Some(&v.path), paths_here.clone(), i);
            if picked.as_ref() == Some(&v.path) {
                row.add_css_class("picked");
                picked_row = Some(row.clone());
            }
            self.rows.append(&row);
        }
        if children.is_empty() && here.is_empty() {
            let note = widgets::label("No videos in this folder.", "dim");
            note.set_margin_top(24);
            self.rows.append(&note);
        }
        if focus.is_some()
            && let Some(row) = picked_row
        {
            gtk::glib::idle_add_local_once(move || {
                row.grab_focus();
            });
        }
    }

    fn folder_row(self: &Rc<Self>, node: &Rc<Node>, secs: f64) -> gtk::Button {
        let button = gtk::Button::new();
        button.add_css_class("flat");
        button.add_css_class("disk-row");
        let line = widgets::hbox(12);
        let slot = widgets::hbox(0);
        slot.add_css_class("folder-slot");
        slot.set_size_request(104, 36);
        slot.set_hexpand(false);
        let icon = gtk::Image::from_icon_name("folder-symbolic");
        icon.add_css_class("folder-glyph");
        // The Folders group's colour.
        icon.add_css_class("section-swatch-teal");
        icon.set_hexpand(true);
        icon.set_halign(gtk::Align::Center);
        slot.append(&icon);
        line.append(&slot);
        let mut sub = fmt::count(node.count, "video", "videos");
        if secs > 0.0 {
            sub.push_str(" · ");
            sub.push_str(&fmt::total(secs));
        }
        line.append(&two_lines(&node.name, &sub));
        let chevron = gtk::Image::from_icon_name("pan-end-symbolic");
        chevron.add_css_class("folder-chevron");
        line.append(&chevron);
        button.set_child(Some(&line));
        button.set_tooltip_text(Some(&paths::pretty(&node.path)));
        let (me, path) = (self.clone(), node.path.clone());
        button.connect_clicked(move |_| me.go(Some(path.clone())));
        button
    }
}

/// A file in the folder: its frame with how far in, its name, what it is, and
/// whether it's watched or playing. Click plays the folder from it.
fn file_row(v: &Rc<Video>, playing: bool, folder: Vec<PathBuf>, index: usize) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.add_css_class("disk-row");
    button.add_css_class("file");
    let line = widgets::hbox(12);
    let art = Art::new(104, 58, true, "nmp-video-symbolic");
    art.root.add_css_class("row-thumb");
    art.set_key(v.wide_art());
    let strip = views::progress_strip();
    views::set_progress(&strip, v.progress());
    art.root.add_overlay(&strip);
    line.append(&art.root);
    let name = v.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| v.title.clone());
    line.append(&two_lines(&name, &file_detail(v)));
    if playing || v.in_progress() {
        let now = gtk::Image::from_icon_name("media-playback-start-symbolic");
        now.add_css_class("folder-now");
        now.set_tooltip_text(Some(if playing { "Playing" } else { "Started" }));
        line.append(&now);
    } else if v.watch.get().watched {
        let ok = gtk::Image::from_icon_name("emblem-ok-symbolic");
        ok.add_css_class("folder-watched");
        ok.set_tooltip_text(Some("Watched"));
        line.append(&ok);
    }
    button.set_child(Some(&line));
    button.set_tooltip_text(Some(&paths::pretty(&v.path)));
    button.connect_clicked(move |_| {
        player::set_shuffle(false);
        player::play_paths(folder.clone(), index);
        window::navigate("now-playing");
    });
    let v2 = v.clone();
    nyx_ui::menu::on_right_click(&button, move |w, x, y| menu::video_menu(w, x, y, vec![v2.clone()], Vec::new()));
    button
}

/// "5 · The Long Dark · 55 min", "Movie · 2021 · 2 h 4 min", "3 min · 310 MB".
fn file_detail(v: &Video) -> String {
    let mut parts = Vec::new();
    match v.kind {
        Kind::Episode => {
            if let Some(e) = v.episode {
                parts.push(e.to_string());
            }
            parts.push(v.episode_name());
        }
        Kind::Movie => {
            parts.push("Movie".to_string());
            if let Some(y) = v.year {
                parts.push(y.to_string());
            }
        }
        Kind::Other => {}
    }
    if v.duration > 0.0 {
        parts.push(fmt::runtime(v.duration));
    }
    if v.kind == Kind::Other && v.size > 0 {
        parts.push(fmt::size(v.size));
    }
    parts.retain(|p| !p.is_empty());
    parts.join(" · ")
}

/// A bold first line over a dim second one.
fn two_lines(title: &str, sub: &str) -> gtk::Box {
    let text = widgets::vbox(2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let t = widgets::label(title, "row-title");
    t.set_xalign(0.0);
    t.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    text.append(&t);
    let s = widgets::label(sub, "row-sub");
    s.set_xalign(0.0);
    s.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&s);
    text
}

/// The folders from a library root down to `dir`, or nothing when it isn't there.
fn chain_to(nodes: &[Rc<Node>], dir: &Path) -> Vec<Rc<Node>> {
    for n in nodes {
        if n.path == dir {
            return vec![n.clone()];
        }
        if dir.starts_with(&n.path) {
            let mut rest = chain_to(&n.children, dir);
            if !rest.is_empty() {
                rest.insert(0, n.clone());
                return rest;
            }
        }
    }
    Vec::new()
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_tree_with_counts() {
        let t = |p: &str| Rc::new(Video { path: p.into(), ..Default::default() });
        let tracks = vec![t("/m/A/x/1.mkv"), t("/m/A/x/2.mkv"), t("/m/A/3.mkv"), t("/m/B/4.mkv")];
        let roots = tree(&[PathBuf::from("/m"), PathBuf::from("/empty")], &tracks);
        assert_eq!(roots.len(), 1);
        let m = &roots[0];
        assert_eq!(m.count, 4);
        assert_eq!(m.name, "m");
        assert_eq!(m.children.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["A", "B"]);
        assert_eq!(m.children[0].count, 3);
        assert_eq!(m.children[0].children[0].name, "x");
        assert_eq!(m.children[0].children[0].count, 2);
    }

    #[test]
    fn describes_files() {
        let ep = Video { kind: Kind::Episode, episode: Some(5), title: "The Long Dark".into(), duration: 3300.0, ..Default::default() };
        assert_eq!(file_detail(&ep), "5 · The Long Dark · 55 min");
        let movie = Video { kind: Kind::Movie, year: Some(2021), duration: 7440.0, ..Default::default() };
        assert_eq!(file_detail(&movie), "Movie · 2021 · 2 h 4 min");
    }
}
