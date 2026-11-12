use crate::library::{db, store};
use crate::player::{self, scrub};
use crate::widgets::{self, Page};
use crate::{cmd, fmt, paths, prefs, window};
use gtk::prelude::*;
use gtk::gio;
use std::path::PathBuf;

/// Pick a folder and add it to the library (the empty states use this too).
pub fn choose_folder() {
    let dialog = gtk::FileDialog::builder().title("Choose a videos folder").modal(true).build();
    let start = paths::videos_dir();
    if start.is_dir() {
        dialog.set_initial_folder(Some(&gio::File::for_path(start)));
    }
    dialog.select_folder(window::window().as_ref(), gio::Cancellable::NONE, |res| {
        let Ok(file) = res else { return };
        let Some(path) = file.path() else { return };
        add_folder(path);
    });
}

fn add_folder(path: PathBuf) {
    let text = path.to_string_lossy().into_owned();
    let mut already = false;
    prefs::update(|p| {
        // Drop folders that no longer exist (the default ~/Videos when it was never made).
        p.library_folders.retain(|f| std::path::Path::new(f).is_dir());
        if p.library_folders.iter().any(|f| path.starts_with(f)) {
            already = true;
        } else {
            // A parent folder replaces the folders inside it.
            p.library_folders.retain(|f| !std::path::Path::new(f).starts_with(&path));
            p.library_folders.push(text.clone());
        }
    });
    prefs::flush();
    if already {
        window::toast(&format!("{} is already in your library.", paths::pretty(&path)));
        return;
    }
    store::rescan(true);
    store::reload();
    refresh_folders();
}

thread_local! {
    static FOLDERS: std::cell::RefCell<Option<gtk::Box>> = const { std::cell::RefCell::new(None) };
}

fn refresh_folders() {
    let Some(list) = FOLDERS.with(|f| f.borrow().clone()) else { return };
    while let Some(c) = list.first_child() {
        list.remove(&c);
    }
    let videos = store::videos();
    let folders = prefs::get().library_folders;
    for f in &folders {
        let path = PathBuf::from(f);
        let n = videos.iter().filter(|t| t.path.starts_with(&path)).count();
        let desc = if path.is_dir() { fmt::count(n, "video", "videos") } else { "This folder doesn't exist.".to_string() };
        let f2 = f.clone();
        let remove = widgets::two_click("Remove", "Click again to remove", move || {
            prefs::update(|p| p.library_folders.retain(|x| *x != f2));
            prefs::flush();
            store::rescan(true);
            refresh_folders();
        });
        remove.set_tooltip_text(Some("Take this folder's videos out of the library. Nothing is deleted from disk."));
        let row = widgets::row(&paths::pretty(&path), &desc, Some(remove.upcast_ref()));
        if let Some(title) = row.first_child().and_then(|t| t.first_child()) {
            title.add_css_class("mono");
        }
        list.append(&row);
    }
    if folders.is_empty() {
        list.append(&widgets::row("No folders", "Add the folder your videos are in.", None));
    }
}

const LANGUAGES: &[(&str, &str)] = &[
    ("", "The file's default"),
    ("en,eng", "English"),
    ("es,spa", "Spanish"),
    ("fr,fre,fra", "French"),
    ("de,ger,deu", "German"),
    ("it,ita", "Italian"),
    ("pt,por", "Portuguese"),
    ("nl,dut,nld", "Dutch"),
    ("sv,swe", "Swedish"),
    ("no,nor,nob", "Norwegian"),
    ("da,dan", "Danish"),
    ("fi,fin", "Finnish"),
    ("pl,pol", "Polish"),
    ("ru,rus", "Russian"),
    ("ja,jpn", "Japanese"),
    ("ko,kor", "Korean"),
    ("zh,chi,zho", "Chinese"),
];

const TMDB_LANGUAGES: &[(&str, &str)] = &[
    ("en-US", "English (US)"),
    ("en-GB", "English (UK)"),
    ("es-ES", "Spanish"),
    ("fr-FR", "French"),
    ("de-DE", "German"),
    ("it-IT", "Italian"),
    ("pt-BR", "Portuguese (Brazil)"),
    ("nl-NL", "Dutch"),
    ("sv-SE", "Swedish"),
    ("pl-PL", "Polish"),
    ("ja-JP", "Japanese"),
    ("ko-KR", "Korean"),
    ("zh-CN", "Chinese"),
];

/// Look everything up on TMDB again, then rescan (which runs the lookups).
fn refresh_tmdb() {
    cmd::background(
        || db::open().and_then(|c| db::reset_tmdb(&c)),
        |r| match r {
            Ok(()) => store::rescan(false),
            Err(e) => window::toast(&format!("Couldn't reset the details: {e}")),
        },
    );
}

fn dir_size(dir: &std::path::Path) -> u64 {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

pub fn build(page: &Page) {
    let p = prefs::get();

    // ----- Library -----
    let g = page.group("Library");
    let list = widgets::vbox(6);
    g.add(&list);
    FOLDERS.with(|f| *f.borrow_mut() = Some(list.clone()));
    refresh_folders();
    store::subscribe(&list, |c| {
        if c == store::Change::Library {
            refresh_folders();
        }
    });
    let (r, _) =
        widgets::button_row("Add a folder", "Videos anywhere under it join the library.", "Add folder…", |_| choose_folder());
    g.add(&r);
    let (scan_row, scan_btn) =
        widgets::button_row("Rescan", "Read new and changed files now.", "Rescan now", |_| store::rescan(true));
    g.add(&scan_row);
    {
        let desc =
            scan_row.first_child().and_then(|t| t.first_child()).and_then(|t| t.next_sibling()).and_downcast::<gtk::Label>();
        let b = scan_btn.clone();
        let refresh = move || {
            let busy = store::scanning();
            b.set_sensitive(busy.is_none() && store::busy().is_none());
            if let Some(d) = &desc {
                d.set_text(&match (busy, store::busy()) {
                    (Some((done, total)), _) if total > 0 => {
                        format!("Reading {} of {}…", fmt::thousands(done), fmt::thousands(total))
                    }
                    (Some(_), _) => "Looking for videos…".into(),
                    (None, Some(b)) => format!("Working in the background: {b}."),
                    (None, None) => format!(
                        "Read new and changed files now. {} in the library.",
                        fmt::count(store::videos().len(), "video", "videos")
                    ),
                });
            }
        };
        refresh();
        store::subscribe(&scan_btn, move |c| {
            if c != store::Change::Watch {
                refresh()
            }
        });
    }
    let (r, _) = widgets::switch_row(
        "Watch for changes",
        "Notice videos being added, removed or renamed within a minute.",
        p.watch,
        |on| prefs::update(|p| p.watch = on),
    );
    g.add(&r);
    let (r, _) = widgets::switch_row(
        "Pictures from the videos",
        "Grab a frame for videos with no poster or thumbnail beside them.",
        p.frames,
        |on| {
            prefs::update(|p| p.frames = on);
            if on {
                store::rescan(false);
            }
        },
    );
    g.add(&r);
    g.note("Names come from file and folder names, then <tt>.nfo</tt> files, then TMDB if it's on. Pictures named <tt>poster.jpg</tt>, <tt>&lt;name&gt;-poster.jpg</tt> or <tt>&lt;name&gt;-thumb.jpg</tt> beside a video are used first.");

    // ----- Playback -----
    let g = page.group("Playback");
    let (r, _) = widgets::switch_row(
        "Resume where I left off",
        "Start each video where it was stopped, and keep the queue between sessions.",
        p.resume,
        |on| {
            prefs::update(|p| {
                p.resume = on;
                p.restore_queue = on;
            })
        },
    );
    g.add(&r);
    let (r, _) = widgets::switch_row(
        "Play the next episode",
        "When an episode ends with nothing else queued, the next one starts.",
        p.autoplay_next,
        |on| prefs::update(|p| p.autoplay_next = on),
    );
    g.add(&r);
    let (row, _) = widgets::choice_row("Show Up next", "Show the next video before the current one ends. Off hides the card without changing autoplay.",
        widgets::opts(&[("0", "Off"), ("10", "10 seconds before the end"), ("20", "20 seconds before the end"), ("30", "30 seconds before the end")]),
        &p.up_next_seconds.to_string(), |id| prefs::update(|p| p.up_next_seconds = id.parse().unwrap_or(20)));
    g.add(&row);
    let current = format!("{}", p.skip_short as i64);
    g.add(&widgets::segmented_row(
        "Skip",
        "How far ← and → jump. Shift jumps further.",
        widgets::opts(&[("5", "5 s"), ("10", "10 s"), ("15", "15 s")]),
        &current,
        |v| {
            let s: f64 = v.parse().unwrap_or(5.0);
            prefs::update(|p| {
                p.skip_short = s;
                p.skip_long = s * 6.0;
            });
        },
    ));
    let speeds: Vec<(String, String)> = player::SPEEDS.iter().map(|s| (fmt::speed(*s), format!("{}×", fmt::speed(*s)))).collect();
    let (r, _) = widgets::choice_row("Speed", "How fast videos start playing.", speeds, &fmt::speed(p.speed), |v| {
        let s: f64 = v.parse().unwrap_or(1.0);
        prefs::update(|p| p.speed = s);
        player::set_speed(s);
    });
    g.add(&r);
    let (r, _) = widgets::switch_row(
        "Hardware decoding",
        "Decode on the graphics card when it can: smoother 4K and less battery. Turn off if pictures look wrong.",
        p.hwdec,
        |on| {
            prefs::update(|p| p.hwdec = on);
            player::set_hwdec(on);
        },
    );
    g.add(&r);
    let (r, _) = widgets::switch_row(
        "Reverse touchpad seeking",
        "Swipe left to go forward. Turn on if seeking feels backwards with natural scrolling.",
        p.swipe_reverse,
        |on| prefs::update(|p| p.swipe_reverse = on),
    );
    g.add(&r);
    let (r, _) = widgets::switch_row("Notifications", "Say what's starting while this window isn't in front.", p.notify, |on| {
        prefs::update(|p| p.notify = on)
    });
    g.add(&r);

    g.note("Press <b>F1</b> anywhere to see the shortcuts. Media keys work through MPRIS. From a terminal or a binding: <tt>media-player --play-pause</tt>, <tt>--next</tt>, <tt>--previous</tt>.");

    // ----- Subtitles and sound -----
    let g = page.group("Subtitles and sound");
    let (r, _) = widgets::switch_row(
        "Show subtitles",
        "Turn subtitles on when a video has them. Each video remembers what you pick for it.",
        p.subtitles,
        |on| prefs::update(|p| p.subtitles = on),
    );
    g.add(&r);
    let langs: Vec<(String, String)> = LANGUAGES.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    let (r, _) = widgets::choice_row(
        "Subtitle language",
        "Pick subtitles in this language when there's a choice.",
        langs.clone(),
        &p.sub_lang,
        |v| {
            prefs::update(|p| p.sub_lang = v);
            player::apply_languages();
        },
    );
    g.add(&r);
    let (r, _) = widgets::choice_row(
        "Sound language",
        "Pick the sound track in this language when there's a choice.",
        langs,
        &p.audio_lang,
        |v| {
            prefs::update(|p| p.audio_lang = v);
            player::apply_languages();
        },
    );
    g.add(&r);
    let size = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.5, 2.5, 0.05);
    size.set_draw_value(false);
    size.set_size_request(180, -1);
    size.set_value(p.sub_scale);
    let readout = widgets::label(&format!("{:.0}%", p.sub_scale * 100.0), "value-readout");
    readout.set_width_chars(5);
    readout.set_xalign(1.0);
    let r2 = readout.clone();
    size.connect_value_changed(move |s| {
        player::set_sub_scale(s.value());
        r2.set_text(&format!("{:.0}%", s.value() * 100.0));
    });
    let size_box = widgets::hbox(10);
    size_box.append(&size);
    size_box.append(&readout);
    g.add(&widgets::row("Subtitle size", "Text subtitles; picture subtitles keep their size.", Some(size_box.upcast_ref())));
    let (r, _) = widgets::switch_row(
        "Subtitle background",
        "Put text subtitles on a dark box, easier to read over bright pictures.",
        p.sub_background,
        player::set_sub_background,
    );
    g.add(&r);
    let (r, _) = widgets::switch_row(
        "Night mode",
        "Even out loud and quiet parts so dialogue is clear without loud scenes jumping out (N).",
        p.night_mode,
        player::set_night_mode,
    );
    g.add(&r);
    let (r, _) = widgets::switch_row(
        "Volume boost",
        "Let the volume go up to 150% for quiet videos. Sound may distort near the top.",
        p.volume_boost,
        player::set_volume_boost,
    );
    g.add(&r);

    // ----- Online details -----
    let g = page.group("Online details");
    let tmdb_rows = widgets::vbox(6);
    tmdb_rows.set_visible(p.tmdb);
    let tr = tmdb_rows.clone();
    let (r, _) = widgets::switch_row(
        "Look things up on TMDB",
        "Posters, plots, ratings and episode names from The Movie Database. Needs your own free API key.",
        p.tmdb,
        move |on| {
            prefs::update(|p| p.tmdb = on);
            tr.set_visible(on);
            if on && !crate::secrets::tmdb_key().is_empty() {
                refresh_tmdb();
            }
        },
    );
    g.add(&r);
    let key_box = widgets::hbox(8);
    let key = gtk::PasswordEntry::new();
    key.set_show_peek_icon(true);
    key.set_text(&crate::secrets::tmdb_key());
    key.set_size_request(240, -1);
    key.add_css_class("mono");
    let save = gtk::Button::with_label("Use key");
    let k = key.clone();
    let apply = move || {
        let text = k.text().trim().to_string();
        if text == crate::secrets::tmdb_key() {
            return;
        }
        if let Err(e) = crate::secrets::set_tmdb_key(&text) {
            window::toast(&e);
            return;
        }
        if !text.is_empty() {
            window::toast("Looking up your library on TMDB…");
            refresh_tmdb();
        }
    };
    let a = apply.clone();
    save.connect_clicked(move |_| a());
    key.connect_activate(move |_| apply());
    key_box.append(&key);
    key_box.append(&save);
    tmdb_rows.append(&widgets::row(
        "API key",
        "Get one free at <a href=\"https://www.themoviedb.org/settings/api\">themoviedb.org</a>. A v3 key or a v4 read token both work.",
        Some(key_box.upcast_ref()),
    ));
    let tl: Vec<(String, String)> = TMDB_LANGUAGES.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    let (r, _) = widgets::choice_row("Language", "The language for titles and plots.", tl, &p.tmdb_language, |v| {
        prefs::update(|p| p.tmdb_language = v);
        refresh_tmdb();
    });
    tmdb_rows.append(&r);
    let (r, _) = widgets::button_row(
        "Look everything up again",
        "Fetch every movie and show again, after renaming files or picking matches.",
        "Refresh",
        |_| {
            window::toast("Looking up your library on TMDB…");
            refresh_tmdb();
        },
    );
    tmdb_rows.append(&r);
    let attribution = widgets::label("This product uses the TMDB API but is not endorsed or certified by TMDB.", "group-note");
    attribution.set_wrap(true);
    tmdb_rows.append(&attribution);
    g.add(&tmdb_rows);

    // ----- Seek bar previews -----
    let g = page.group("Seek bar previews");
    let (r, _) = widgets::switch_row(
        "Show previews",
        "A picture of the frame under the pointer on the seek bar. Made the first time a video plays.",
        p.scrub,
        |on| prefs::update(|p| p.scrub = on),
    );
    g.add(&r);
    let (clear_row, clear_btn) = widgets::button_row("Clear previews", "", "Clear", |_| {});
    let desc = clear_row.first_child().and_then(|t| t.first_child()).and_then(|t| t.next_sibling()).and_downcast::<gtk::Label>();
    let show_size = {
        let desc = desc.clone();
        move || {
            let d = desc.clone();
            cmd::background(
                || dir_size(&paths::scrub_dir()),
                move |bytes| {
                    if let Some(d) = &d {
                        d.set_text(&format!("They take {} on disk and come back as videos play.", fmt::size(bytes as i64)));
                        d.set_visible(true);
                    }
                },
            );
        }
    };
    show_size();
    clear_btn.connect_clicked(move |_| {
        scrub::clear_cache();
        show_size();
        window::toast("Previews cleared.");
    });
    g.add(&clear_row);
}
