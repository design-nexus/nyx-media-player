//! Media Player's tools on the control interface (`nyx_core::control`), for Nyx
//! Assistant: find movies and shows, play one, and read or steer what's playing.

use crate::library::{Video, store};
use crate::player::{self, State, queue::Repeat};
use gtk::glib;
use gtk::prelude::*;
use nyx_core::control::{Reply, text};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::rc::Rc;

pub fn tools() -> Value {
    json!([
        {"name": "video.search", "description": "Search the Media Player library: movies, shows and episodes matching the words.", "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}},
        {"name": "video.play", "description": "Play a movie, a show (its next episode), an episode or a playlist from the Media Player library, by name.", "inputSchema": {"type": "object", "properties": {"what": {"enum": ["movie", "show", "episode", "playlist"]}, "name": {"type": "string"}}, "required": ["what", "name"]}},
        {"name": "video.status", "description": "What Media Player is playing: title, show, position, playing or paused.", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "video.control", "description": "Play, pause, toggle, next, previous or stop in Media Player.", "inputSchema": {"type": "object", "properties": {"action": {"enum": ["play", "pause", "toggle", "next", "previous", "stop"]}}, "required": ["action"]}},
    ])
}

fn video(v: &Video) -> Value {
    json!({"title": v.title, "year": v.year, "show": v.show, "season": v.season, "episode": v.episode, "minutes": (v.duration / 60.0).round()})
}

/// How well `name` fits `query`: 3 the same, 2 starts with it, 1 contains it.
fn fit(name: &str, query: &str) -> u8 {
    let (n, q) = (name.to_lowercase(), query.trim().to_lowercase());
    if q.is_empty() {
        0
    } else if n == q {
        3
    } else if n.starts_with(&q) {
        2
    } else if n.contains(&q) {
        1
    } else {
        0
    }
}

fn best<T>(items: Vec<T>, name: &str, label: impl Fn(&T) -> String) -> Result<T, String> {
    let top = items.iter().map(|i| fit(&label(i), name)).max().unwrap_or(0);
    if top == 0 {
        return Err(format!("nothing in the library matches “{name}”"));
    }
    let mut found: Vec<T> = items.into_iter().filter(|i| fit(&label(i), name) == top).collect();
    let mut names: Vec<String> = found.iter().take(5).map(&label).collect();
    names.dedup();
    if names.len() > 1 {
        return Err(format!("several match: {}", names.join("; ")));
    }
    Ok(found.remove(0))
}

fn paths(videos: &[Rc<Video>]) -> Vec<PathBuf> {
    videos.iter().map(|v| v.path.clone()).collect()
}

fn status() -> Value {
    let state = match player::state() {
        State::Playing => "playing",
        State::Paused => "paused",
        State::Stopped => "stopped",
    };
    let repeat = match player::repeat() {
        Repeat::Off => "off",
        Repeat::All => "all",
        Repeat::One => "one",
    };
    json!({"state": state, "video": player::current().map(|v| video(&v)), "position": player::position().round(), "duration": player::duration().round(), "repeat": repeat})
}

/// Media Player's description for the control interface (API 2): its actions in the
/// `video` namespace, and D-Bus activation so a call starts it.
pub fn description(app_id: &str) -> Value {
    let mut tools = tools();
    for t in tools.as_array_mut().into_iter().flatten() {
        let risk = if matches!(t["name"].as_str(), Some("video.search" | "video.status")) { "read_only" } else { "writes" };
        t["x-nyx"] = json!({"risk": risk});
        // A single required argument can be given bare: `nyx APP VERB WORDS`.
        if let Some([one]) = t["inputSchema"]["required"].as_array().map(Vec::as_slice) {
            t["x-nyx"]["positional"] = json!([one]);
        }
    }
    tools[0]["x-nyx"]["positional"] = json!(["query"]);
    tools[3]["x-nyx"]["positional"] = json!(["action"]);
    json!({
        "tools": tools,
        "nyx": {"api": 2, "app": app_id, "name": "Media Player", "icon": crate::APP_ID, "namespaces": ["video"],
                "aliases": ["media-player", "videos"], "bus_name": app_id, "activation": "dbus", "default": "video.search"}
    })
}

pub fn handle(name: &str, args: Value, reply: Reply) {
    // Started for this call: the library and playback as the window would have them, and
    // the window itself to play in.
    if let Some(app) = gtk::gio::Application::default().and_then(|a| a.downcast::<gtk::Application>().ok()) {
        crate::start(&app);
        if name == "video.play" && crate::window::window().is_none() {
            crate::window::present(&app, None);
        }
    }
    if !store::loaded() && matches!(name, "video.search" | "video.play") {
        // Just started for this call: the library is a moment from loaded.
        let (name, tries) = (name.to_string(), std::cell::Cell::new(0));
        let pending = std::cell::RefCell::new(Some((args, reply)));
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            tries.set(tries.get() + 1);
            if !store::loaded() && tries.get() < 50 {
                return glib::ControlFlow::Continue;
            }
            if let Some((args, reply)) = pending.borrow_mut().take() {
                if store::loaded() {
                    handle(&name, args, reply);
                } else {
                    reply(Err("the library is still loading; try again in a moment".into()));
                }
            }
            glib::ControlFlow::Break
        });
        return;
    }
    match name {
        "video.search" => reply(text(&args, "query").map(|q| {
            let hit = |s: &str| fit(s, &q) > 0;
            let movies: Vec<Value> = store::movies().iter().filter(|v| hit(&v.title)).take(10).map(|v| video(v)).collect();
            let shows: Vec<Value> = store::shows()
                .iter()
                .filter(|s| hit(&s.name))
                .take(10)
                .map(|s| json!({"name": s.name, "year": s.year, "seasons": s.seasons.len()}))
                .collect();
            let episodes: Vec<Value> =
                store::videos().iter().filter(|v| !v.show.is_empty() && hit(&v.title)).take(10).map(|v| video(v)).collect();
            json!({"movies": movies, "shows": shows, "episodes": episodes})
        })),
        "video.play" => {
            let (what, wanted) = match (text(&args, "what"), text(&args, "name")) {
                (Ok(w), Ok(n)) => (w, n),
                (Err(e), _) | (_, Err(e)) => return reply(Err(e)),
            };
            let play = |label: String, list: Vec<Rc<Video>>, start: usize| -> Result<Value, String> {
                if list.is_empty() {
                    return Err(format!("{label} has nothing to play"));
                }
                player::play_paths(paths(&list), start);
                Ok(json!({"playing": label, "items": list.len()}))
            };
            match what.as_str() {
                "movie" => {
                    reply(best(store::movies(), &wanted, |v| v.title.clone()).and_then(|v| play(v.title.clone(), vec![v], 0)))
                }
                "episode" => reply(
                    best(store::videos().into_iter().filter(|v| !v.show.is_empty()).collect(), &wanted, |v| v.title.clone())
                        .and_then(|v| play(format!("{} ({})", v.title, v.show), vec![v], 0)),
                ),
                "show" => reply(best(store::shows(), &wanted, |s| s.name.clone()).and_then(|s| {
                    let all: Vec<Rc<Video>> = s.episodes().cloned().collect();
                    // Where the person left off, else the first episode.
                    let start = s.next_episode().and_then(|n| all.iter().position(|e| e.path == n.path)).unwrap_or(0);
                    play(s.name.clone(), all, start)
                })),
                "playlist" => match best(store::playlists(), &wanted, |p| p.name.clone()) {
                    Ok(p) => {
                        let label = p.name.clone();
                        store::playlist_videos(p.id, move |list| reply(play(label, list, 0)));
                    }
                    Err(e) => reply(Err(e)),
                },
                other => reply(Err(format!("unknown kind {other}"))),
            }
        }
        "video.status" => reply(Ok(status())),
        "video.control" => reply((|| {
            match text(&args, "action")?.as_str() {
                "play" => player::play(),
                "pause" => player::pause(),
                "toggle" => player::toggle(),
                "next" => player::next(),
                "previous" => player::previous(),
                "stop" => player::stop(),
                other => return Err(format!("unknown action {other}")),
            }
            Ok(status())
        })()),
        _ => reply(Err(format!("unknown tool {name}"))),
    }
}
