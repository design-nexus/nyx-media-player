//! Optional details from The Movie Database (TMDB), with the user's own API
//! key: posters, plots, ratings, genres and episode names. Off unless a key
//! is set. Answers are cached in the database for a month; `.nfo` files
//! always win over TMDB.

use super::db::{self, TmdbVideo};
use super::scan::Progress;
use super::{art, show_key};
use crate::prefs::Prefs;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

const API: &str = "https://api.themoviedb.org/3";
const IMAGES: &str = "https://image.tmdb.org/t/p";
const MONTH: i64 = 30 * 24 * 3600;

#[derive(Clone)]
pub struct Settings {
    pub key: String,
    pub language: String,
}

impl Settings {
    pub fn from_prefs(p: &Prefs) -> Option<Settings> {
        if !p.tmdb {
            return None;
        }
        let key = crate::secrets::tmdb_key();
        (!key.is_empty()).then(|| Settings { key, language: p.tmdb_language.clone() })
    }
}

#[derive(Debug)]
pub enum Error {
    /// The key was refused.
    Key,
    Network(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Key => write!(f, "TMDB didn't accept the API key"),
            Error::Network(e) => write!(f, "couldn't reach TMDB: {e}"),
        }
    }
}

struct Client {
    s: Settings,
    agent: ureq::Agent,
    conn: rusqlite::Connection,
}

/// `query` is percent-encoded here; keep it to letters, digits and the rest.
fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn year_of(date: Option<&str>) -> Option<i32> {
    date.filter(|d| d.len() >= 4).and_then(|d| d[..4].parse().ok())
}

fn genres(v: &Value) -> Option<String> {
    let g: Vec<&str> = v.get("genres")?.as_array()?.iter().filter_map(|g| g.get("name")?.as_str()).collect();
    (!g.is_empty()).then(|| g.join(", "))
}

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

fn rating(v: &Value) -> Option<f64> {
    v.get("vote_average").and_then(Value::as_f64).filter(|r| *r > 0.0)
}

impl Client {
    fn new(s: &Settings) -> anyhow::Result<Client> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .build()
            .into();
        Ok(Client { s: s.clone(), agent, conn: db::open()? })
    }

    /// GET an API path (with its query), from the cache when it's fresh.
    fn get(&self, path_query: &str) -> Result<Value, Error> {
        let sep = if path_query.contains('?') { '&' } else { '?' };
        let lang = if self.s.language.is_empty() { String::new() } else { format!("{sep}language={}", encode(&self.s.language)) };
        let url = format!("{API}{path_query}{lang}");
        if let Ok(Some(body)) = db::cache_get(&self.conn, &url, MONTH)
            && let Ok(v) = serde_json::from_str(&body)
        {
            return Ok(v);
        }
        // A v4 read token goes in a header; a v3 key in the query.
        let bearer = self.s.key.starts_with("eyJ");
        let full = if bearer {
            url.clone()
        } else {
            format!("{url}{}api_key={}", if url.contains('?') { '&' } else { '?' }, encode(&self.s.key))
        };
        let mut req = self.agent.get(&full).header("Accept", "application/json");
        if bearer {
            req = req.header("Authorization", &format!("Bearer {}", self.s.key));
        }
        // Stay well under TMDB's rate limit.
        std::thread::sleep(Duration::from_millis(60));
        let mut resp = req.call().map_err(|e| Error::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        if status == 401 {
            return Err(Error::Key);
        }
        let body = resp.body_mut().read_to_string().map_err(|e| Error::Network(e.to_string()))?;
        if status == 404 {
            return Ok(Value::Null);
        }
        if status != 200 {
            return Err(Error::Network(format!("HTTP {status}")));
        }
        let _ = db::cache_put(&self.conn, &url, &body);
        serde_json::from_str(&body).map_err(|e| Error::Network(e.to_string()))
    }

    /// Download and cache a TMDB image; returns its art key.
    fn image(&self, path: Option<&str>, size: &str) -> Option<String> {
        let path = path?;
        let url = format!("{IMAGES}/{size}{path}");
        if let Some(k) = art::cached_url(&url) {
            return Some(k);
        }
        let mut resp = self.agent.get(&url).call().ok()?;
        if resp.status().as_u16() != 200 {
            return None;
        }
        let bytes = resp.body_mut().read_to_vec().ok()?;
        Some(art::from_bytes(&url, &bytes)).filter(|k| !k.is_empty())
    }

    fn search(&self, kind: &str, query: &str, year: Option<i32>) -> Result<Vec<Candidate>, Error> {
        let year_param = if kind == "movie" { "year" } else { "first_air_date_year" };
        let mut q = format!("/search/{kind}?query={}&include_adult=false", encode(query));
        if let Some(y) = year {
            q.push_str(&format!("&{year_param}={y}"));
        }
        let v = self.get(&q)?;
        let results = v.get("results").and_then(Value::as_array).cloned().unwrap_or_default();
        Ok(results
            .iter()
            .filter_map(|r| {
                Some(Candidate {
                    id: r.get("id")?.as_i64()?,
                    title: text(r, "title").or_else(|| text(r, "name"))?,
                    year: year_of(r.get("release_date").or(r.get("first_air_date")).and_then(Value::as_str)),
                    overview: text(r, "overview").unwrap_or_default(),
                })
            })
            .collect())
    }

    /// The best match: with the year if there is one, else without.
    fn find(&self, kind: &str, query: &str, year: Option<i32>) -> Result<Option<i64>, Error> {
        if query.trim().is_empty() {
            return Ok(None);
        }
        let mut hits = self.search(kind, query, year)?;
        if hits.is_empty() && year.is_some() {
            hits = self.search(kind, query, None)?;
        }
        Ok(hits.first().map(|c| c.id))
    }

    fn movie(&self, id: i64) -> Result<TmdbVideo, Error> {
        let v = self.get(&format!("/movie/{id}"))?;
        if v.is_null() {
            return Ok(TmdbVideo::default());
        }
        Ok(TmdbVideo {
            tmdb_id: Some(id),
            title: text(&v, "title"),
            year: year_of(v.get("release_date").and_then(Value::as_str)),
            plot: text(&v, "overview"),
            rating: rating(&v),
            genres: genres(&v),
            poster: self.image(v.get("poster_path").and_then(Value::as_str), "w500"),
            still: None,
            backdrop: self.image(v.get("backdrop_path").and_then(Value::as_str), "w1280"),
        })
    }

    fn show(&self, id: i64) -> Result<(TmdbVideo, Option<String>), Error> {
        let v = self.get(&format!("/tv/{id}"))?;
        if v.is_null() {
            return Ok((TmdbVideo::default(), None));
        }
        let t = TmdbVideo {
            tmdb_id: Some(id),
            title: text(&v, "name"),
            year: year_of(v.get("first_air_date").and_then(Value::as_str)),
            plot: text(&v, "overview"),
            rating: rating(&v),
            genres: genres(&v),
            poster: self.image(v.get("poster_path").and_then(Value::as_str), "w500"),
            still: None,
            backdrop: None,
        };
        let backdrop = self.image(v.get("backdrop_path").and_then(Value::as_str), "w1280");
        Ok((t, backdrop))
    }

    fn season(&self, show: i64, season: u32) -> Result<Value, Error> {
        self.get(&format!("/tv/{show}/season/{season}"))
    }
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: i64,
    pub title: String,
    pub year: Option<i32>,
    pub overview: String,
}

/// Search for Fix match (movies: `kind = "movie"`, shows: `"tv"`).
pub fn search(s: &Settings, kind: &str, query: &str, year: Option<i32>) -> Result<Vec<Candidate>, String> {
    let c = Client::new(s).map_err(|e| e.to_string())?;
    c.search(kind, query, year).map_err(|e| e.to_string())
}

/// Look up everything that hasn't been looked up yet.
pub fn run(s: &Settings, send: impl Fn(Progress)) {
    if let Err(e) = run_inner(s, &send) {
        send(Progress::Failed(e.to_string()));
    }
    send(Progress::Online { done: 1, total: 1 });
}

fn run_inner(s: &Settings, send: &impl Fn(Progress)) -> Result<(), Error> {
    let c = Client::new(s).map_err(|e| Error::Network(e.to_string()))?;
    let todo = db::tmdb_todo(&c.conn).map_err(|e| Error::Network(e.to_string()))?;
    let total = todo.movies.len() + todo.shows.len() + todo.episodes.len();
    if total == 0 {
        return Ok(());
    }
    let mut done = 0;
    let tick = |done: usize| {
        if done.is_multiple_of(10) {
            send(Progress::Online { done, total });
            send(Progress::Changed);
        }
    };
    let save = |r: anyhow::Result<()>| r.map_err(|e| Error::Network(e.to_string()));

    for (path, title, year, known) in &todo.movies {
        let target = format!("movie:{}", path.display());
        let id = match db::get_match(&c.conn, &target).ok().flatten().or(*known) {
            Some(id) => Some(id),
            None => c.find("movie", title, *year)?,
        };
        let t = match id {
            Some(id) => c.movie(id)?,
            None => TmdbVideo::default(),
        };
        save(db::apply_tmdb_video(&c.conn, path, &t))?;
        done += 1;
        tick(done);
    }

    let mut show_ids: HashMap<String, i64> = HashMap::new();
    for (key, name, year, known) in &todo.shows {
        let id = match db::get_match(&c.conn, &format!("show:{key}")).ok().flatten().or(*known) {
            Some(id) => Some(id),
            None => c.find("tv", name, *year)?,
        };
        let (t, backdrop) = match id {
            Some(id) => c.show(id)?,
            None => (TmdbVideo::default(), None),
        };
        save(db::apply_tmdb_show(&c.conn, key, &t, backdrop))?;
        if let Some(id) = id {
            show_ids.insert(key.clone(), id);
        }
        done += 1;
        tick(done);
    }
    // Shows done in an earlier run.
    for row in db::shows(&c.conn).unwrap_or_default() {
        if let Some(id) = row.tmdb_id {
            show_ids.entry(row.key).or_insert(id);
        }
    }

    let mut seasons: HashMap<(i64, u32), Value> = HashMap::new();
    for (path, show, season, episode) in &todo.episodes {
        let t = match (show_ids.get(&show_key(show)), season, episode) {
            (Some(&id), Some(s), Some(e)) => {
                let data = match seasons.get(&(id, *s)) {
                    Some(d) => d.clone(),
                    None => {
                        let d = c.season(id, *s)?;
                        seasons.insert((id, *s), d.clone());
                        d
                    }
                };
                let ep = data
                    .get("episodes")
                    .and_then(Value::as_array)
                    .and_then(|eps| eps.iter().find(|x| x.get("episode_number").and_then(Value::as_u64) == Some(*e as u64)));
                match ep {
                    Some(ep) => TmdbVideo {
                        tmdb_id: ep.get("id").and_then(Value::as_i64),
                        title: text(ep, "name"),
                        year: None,
                        plot: text(ep, "overview"),
                        rating: rating(ep),
                        genres: None,
                        poster: None,
                        still: c.image(ep.get("still_path").and_then(Value::as_str), "w780"),
                        backdrop: None,
                    },
                    None => TmdbVideo::default(),
                }
            }
            _ => TmdbVideo::default(),
        };
        save(db::apply_tmdb_video(&c.conn, path, &t))?;
        done += 1;
        tick(done);
    }
    send(Progress::Changed);
    Ok(())
}

/// Remember a hand-picked match and look that movie or show up again.
pub fn set_match(target: &str, id: i64) -> anyhow::Result<()> {
    let conn = db::open()?;
    db::set_match(&conn, target, id)?;
    db::reset_tmdb_for(&conn, target)
}

pub fn movie_target(path: &std::path::Path) -> String {
    format!("movie:{}", path.display())
}

pub fn show_target(key: &str) -> String {
    format!("show:{key}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_queries() {
        assert_eq!(encode("Amélie (2001)"), "Am%C3%A9lie%20%282001%29");
        assert_eq!(encode("a-b_c.d~"), "a-b_c.d~");
    }

    #[test]
    fn reads_years_and_genres() {
        assert_eq!(year_of(Some("2019-05-01")), Some(2019));
        assert_eq!(year_of(Some("")), None);
        let v: Value = serde_json::from_str(r#"{"genres":[{"id":1,"name":"Drama"},{"id":2,"name":"Crime"}]}"#).unwrap();
        assert_eq!(genres(&v).as_deref(), Some("Drama, Crime"));
    }
}
