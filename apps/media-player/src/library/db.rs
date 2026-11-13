//! SQLite storage. Every call opens its own connection, so it can run on any
//! thread; WAL mode keeps readers and the scanner out of each other's way.

use super::{Kind, Video, Watch};
use crate::paths;
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS videos (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    year INTEGER,
    show TEXT NOT NULL DEFAULT '',
    season INTEGER,
    episode INTEGER,
    plot TEXT NOT NULL DEFAULT '',
    rating REAL,
    genres TEXT NOT NULL DEFAULT '',
    duration REAL NOT NULL,
    width INTEGER NOT NULL DEFAULT 0,
    height INTEGER NOT NULL DEFAULT 0,
    vcodec TEXT NOT NULL DEFAULT '',
    acodec TEXT NOT NULL DEFAULT '',
    audio_langs TEXT NOT NULL DEFAULT '',
    sub_langs TEXT NOT NULL DEFAULT '',
    poster TEXT NOT NULL DEFAULT '',
    still TEXT NOT NULL DEFAULT '',
    backdrop TEXT NOT NULL DEFAULT '',
    still_tried INTEGER NOT NULL DEFAULT 0,
    tmdb_id INTEGER,
    -- Where the names came from: 'file', 'nfo' or 'tmdb'.
    source TEXT NOT NULL DEFAULT 'file',
    tmdb_done INTEGER NOT NULL DEFAULT 0,
    added INTEGER NOT NULL,
    mtime INTEGER NOT NULL,
    size INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS watch (
    path TEXT PRIMARY KEY,
    position REAL NOT NULL DEFAULT 0,
    watched INTEGER NOT NULL DEFAULT 0,
    last_played INTEGER NOT NULL DEFAULT 0,
    plays INTEGER NOT NULL DEFAULT 0,
    aid INTEGER,
    sid INTEGER,
    sub_delay REAL NOT NULL DEFAULT 0,
    audio_delay REAL NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS shows (
    key TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    folder TEXT NOT NULL DEFAULT '',
    year INTEGER,
    plot TEXT NOT NULL DEFAULT '',
    rating REAL,
    genres TEXT NOT NULL DEFAULT '',
    poster TEXT NOT NULL DEFAULT '',
    backdrop TEXT NOT NULL DEFAULT '',
    tmdb_id INTEGER,
    source TEXT NOT NULL DEFAULT 'file',
    tmdb_done INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS tmdb_cache (
    url TEXT PRIMARY KEY,
    body TEXT NOT NULL,
    fetched INTEGER NOT NULL
);
-- Matches picked by hand with Fix match: 'movie:<path>' or 'show:<key>'.
CREATE TABLE IF NOT EXISTS matches (
    target TEXT PRIMARY KEY,
    tmdb_id INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS playlists (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    created INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS playlist_items (
    playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    pos INTEGER NOT NULL,
    path TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS playlist_items_by_list ON playlist_items(playlist_id, pos);
";

pub fn open() -> Result<Connection> {
    open_at(&paths::library_db())
}

pub fn open_at(path: &Path) -> Result<Connection> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.execute_batch(SCHEMA)?;
    migrate(&conn)?;
    Ok(conn)
}

/// Columns added since the first release, for libraries made before them.
fn migrate(conn: &Connection) -> Result<()> {
    let has = |table: &str, column: &str| -> Result<bool> {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let names = stmt.query_map([], |r| r.get::<_, String>(1))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(names.iter().any(|n| n == column))
    };
    if !has("watch", "audio_delay")? {
        conn.execute_batch("ALTER TABLE watch ADD COLUMN audio_delay REAL NOT NULL DEFAULT 0")?;
    }
    if !has("videos", "backdrop")? {
        // Read movies again on the next scan so their backdrops are found.
        conn.execute_batch(
            "ALTER TABLE videos ADD COLUMN backdrop TEXT NOT NULL DEFAULT '';
             UPDATE videos SET mtime = 0 WHERE kind = 'movie';",
        )?;
    }
    Ok(())
}

fn text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

pub fn load_all(conn: &Connection) -> Result<Vec<Video>> {
    let mut stmt = conn.prepare(
        "SELECT v.id, v.path, v.kind, v.title, v.year, v.show, v.season, v.episode, v.plot, v.rating, v.genres,
                v.duration, v.width, v.height, v.vcodec, v.acodec, v.audio_langs, v.sub_langs, v.poster, v.still,
                v.tmdb_id, v.added, v.mtime, v.size,
                w.position, w.watched, w.last_played, w.plays, w.aid, w.sid, w.sub_delay, v.backdrop, w.audio_delay
         FROM videos v LEFT JOIN watch w ON w.path = v.path",
    )?;
    let rows = stmt.query_map([], |r| {
        let watch = Watch {
            position: r.get::<_, Option<f64>>(24)?.unwrap_or(0.0),
            watched: r.get::<_, Option<bool>>(25)?.unwrap_or(false),
            last_played: r.get::<_, Option<i64>>(26)?.unwrap_or(0),
            plays: r.get::<_, Option<u32>>(27)?.unwrap_or(0),
            aid: r.get(28)?,
            sid: r.get(29)?,
            sub_delay: r.get::<_, Option<f64>>(30)?.unwrap_or(0.0),
            audio_delay: r.get::<_, Option<f64>>(32)?.unwrap_or(0.0),
        };
        Ok(Video {
            id: r.get(0)?,
            path: PathBuf::from(r.get::<_, String>(1)?),
            kind: Kind::from_id(&r.get::<_, String>(2)?),
            title: r.get(3)?,
            year: r.get(4)?,
            show: r.get(5)?,
            season: r.get(6)?,
            episode: r.get(7)?,
            plot: r.get(8)?,
            rating: r.get(9)?,
            genres: r.get(10)?,
            duration: r.get(11)?,
            width: r.get(12)?,
            height: r.get(13)?,
            vcodec: r.get(14)?,
            acodec: r.get(15)?,
            audio_langs: r.get(16)?,
            sub_langs: r.get(17)?,
            poster: r.get(18)?,
            still: r.get(19)?,
            backdrop: r.get(31)?,
            tmdb_id: r.get(20)?,
            added: r.get(21)?,
            mtime: r.get(22)?,
            size: r.get(23)?,
            watch: std::cell::Cell::new(watch),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Path → (mtime, size) for every known file: what the scanner diffs against.
pub fn stamps(conn: &Connection) -> Result<HashMap<PathBuf, (i64, i64)>> {
    let mut stmt = conn.prepare("SELECT path, mtime, size FROM videos")?;
    let rows = stmt.query_map([], |r| Ok((PathBuf::from(r.get::<_, String>(0)?), (r.get(1)?, r.get(2)?))))?;
    Ok(rows.collect::<rusqlite::Result<HashMap<_, _>>>()?)
}

/// Insert or refresh a file's details, keeping when it was added. `source`
/// says where the names came from; TMDB looks again after a change.
pub fn upsert(tx: &Transaction, v: &Video, source: &str) -> Result<()> {
    tx.execute(
        "INSERT INTO videos (path, kind, title, year, show, season, episode, plot, rating, genres, duration, width, height,
                             vcodec, acodec, audio_langs, sub_langs, poster, still, still_tried, tmdb_id, source,
                             tmdb_done, added, mtime, size, backdrop)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, 0, ?20, ?21, 0,
                 ?22, ?23, ?24, ?25)
         ON CONFLICT(path) DO UPDATE SET
            kind = excluded.kind, title = excluded.title, year = excluded.year, show = excluded.show,
            season = excluded.season, episode = excluded.episode, plot = excluded.plot, rating = excluded.rating,
            genres = excluded.genres, duration = excluded.duration, width = excluded.width, height = excluded.height,
            vcodec = excluded.vcodec, acodec = excluded.acodec, audio_langs = excluded.audio_langs,
            sub_langs = excluded.sub_langs, poster = excluded.poster, still = excluded.still, still_tried = 0,
            backdrop = excluded.backdrop,
            tmdb_id = excluded.tmdb_id, source = excluded.source, tmdb_done = 0,
            mtime = excluded.mtime, size = excluded.size",
        params![
            text(&v.path),
            v.kind.id(),
            v.title,
            v.year,
            v.show,
            v.season,
            v.episode,
            v.plot,
            v.rating,
            v.genres,
            v.duration,
            v.width,
            v.height,
            v.vcodec,
            v.acodec,
            v.audio_langs,
            v.sub_langs,
            v.poster,
            v.still,
            v.tmdb_id,
            source,
            v.added,
            v.mtime,
            v.size,
            v.backdrop
        ],
    )?;
    Ok(())
}

pub fn delete(tx: &Transaction, path: &Path) -> Result<()> {
    tx.execute("DELETE FROM videos WHERE path = ?1", [text(path)])?;
    Ok(())
}

/// Videos that still need a frame grabbed: (path, duration).
pub fn missing_stills(conn: &Connection) -> Result<Vec<(PathBuf, f64)>> {
    let mut stmt = conn.prepare("SELECT path, duration FROM videos WHERE still = '' AND still_tried = 0 ORDER BY added DESC")?;
    let rows = stmt.query_map([], |r| Ok((PathBuf::from(r.get::<_, String>(0)?), r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn set_still(conn: &Connection, path: &Path, key: &str) -> Result<()> {
    conn.execute("UPDATE videos SET still = ?2, still_tried = 1 WHERE path = ?1", params![text(path), key])?;
    Ok(())
}

// ---------- Watch state ----------

pub fn save_watch(conn: &Connection, path: &Path, w: &Watch) -> Result<()> {
    conn.execute(
        "INSERT INTO watch (path, position, watched, last_played, plays, aid, sid, sub_delay, audio_delay)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(path) DO UPDATE SET position = excluded.position, watched = excluded.watched,
            last_played = excluded.last_played, plays = excluded.plays, aid = excluded.aid, sid = excluded.sid,
            sub_delay = excluded.sub_delay, audio_delay = excluded.audio_delay",
        params![text(path), w.position, w.watched, w.last_played, w.plays, w.aid, w.sid, w.sub_delay, w.audio_delay],
    )?;
    Ok(())
}

pub fn load_watch(conn: &Connection, path: &Path) -> Result<Option<Watch>> {
    Ok(conn
        .query_row(
            "SELECT position, watched, last_played, plays, aid, sid, sub_delay, audio_delay FROM watch WHERE path = ?1",
            [text(path)],
            |r| {
                Ok(Watch {
                    position: r.get(0)?,
                    watched: r.get(1)?,
                    last_played: r.get(2)?,
                    plays: r.get(3)?,
                    aid: r.get(4)?,
                    sid: r.get(5)?,
                    sub_delay: r.get(6)?,
                    audio_delay: r.get(7)?,
                })
            },
        )
        .optional()?)
}

// ---------- Shows ----------

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShowRow {
    pub key: String,
    pub name: String,
    pub folder: String,
    pub year: Option<i32>,
    pub plot: String,
    pub rating: Option<f64>,
    pub genres: String,
    pub poster: String,
    pub backdrop: String,
    pub tmdb_id: Option<i64>,
}

/// Refresh a show from its folder (name, `tvshow.nfo`, pictures). Details that
/// TMDB filled in stay unless the folder now has its own.
pub fn upsert_show(conn: &Connection, s: &ShowRow, from_nfo: bool) -> Result<()> {
    conn.execute(
        "INSERT INTO shows (key, name, folder, year, plot, rating, genres, poster, backdrop, tmdb_id, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(key) DO UPDATE SET
            folder = excluded.folder,
            name = CASE WHEN ?12 OR shows.source != 'tmdb' THEN excluded.name ELSE shows.name END,
            year = CASE WHEN ?12 OR shows.source != 'tmdb' THEN excluded.year ELSE shows.year END,
            plot = CASE WHEN ?12 OR shows.source != 'tmdb' THEN excluded.plot ELSE shows.plot END,
            rating = CASE WHEN ?12 OR shows.source != 'tmdb' THEN excluded.rating ELSE shows.rating END,
            genres = CASE WHEN ?12 OR shows.source != 'tmdb' THEN excluded.genres ELSE shows.genres END,
            poster = CASE WHEN excluded.poster != '' OR shows.source != 'tmdb' THEN excluded.poster ELSE shows.poster END,
            backdrop = CASE WHEN excluded.backdrop != '' OR shows.source != 'tmdb' THEN excluded.backdrop
                            ELSE shows.backdrop END,
            tmdb_id = COALESCE(excluded.tmdb_id, shows.tmdb_id),
            source = CASE WHEN ?12 THEN 'nfo' ELSE shows.source END",
        params![
            s.key,
            s.name,
            s.folder,
            s.year,
            s.plot,
            s.rating,
            s.genres,
            s.poster,
            s.backdrop,
            s.tmdb_id,
            if from_nfo { "nfo" } else { "file" },
            from_nfo
        ],
    )?;
    Ok(())
}

pub fn shows(conn: &Connection) -> Result<Vec<ShowRow>> {
    let mut stmt = conn.prepare("SELECT key, name, folder, year, plot, rating, genres, poster, backdrop, tmdb_id FROM shows")?;
    let rows = stmt.query_map([], |r| {
        Ok(ShowRow {
            key: r.get(0)?,
            name: r.get(1)?,
            folder: r.get(2)?,
            year: r.get(3)?,
            plot: r.get(4)?,
            rating: r.get(5)?,
            genres: r.get(6)?,
            poster: r.get(7)?,
            backdrop: r.get(8)?,
            tmdb_id: r.get(9)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Drop shows that no longer have any episodes.
pub fn prune_shows(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM shows WHERE key NOT IN (SELECT DISTINCT lower(trim(show)) FROM videos WHERE kind = 'episode')",
        [],
    )?;
    Ok(())
}

// ---------- TMDB ----------

pub fn cache_get(conn: &Connection, url: &str, max_age: i64) -> Result<Option<String>> {
    let now = super::now();
    Ok(conn
        .query_row("SELECT body FROM tmdb_cache WHERE url = ?1 AND fetched > ?2", params![url, now - max_age], |r| r.get(0))
        .optional()?)
}

pub fn cache_put(conn: &Connection, url: &str, body: &str) -> Result<()> {
    conn.execute("INSERT OR REPLACE INTO tmdb_cache (url, body, fetched) VALUES (?1, ?2, ?3)", params![url, body, super::now()])?;
    Ok(())
}

pub fn get_match(conn: &Connection, target: &str) -> Result<Option<i64>> {
    Ok(conn.query_row("SELECT tmdb_id FROM matches WHERE target = ?1", [target], |r| r.get(0)).optional()?)
}

pub fn set_match(conn: &Connection, target: &str, id: i64) -> Result<()> {
    conn.execute("INSERT OR REPLACE INTO matches (target, tmdb_id) VALUES (?1, ?2)", params![target, id])?;
    Ok(())
}

/// What TMDB should look up: videos and shows it hasn't done yet.
pub struct TmdbTodo {
    pub movies: Vec<(PathBuf, String, Option<i32>, Option<i64>)>,
    pub episodes: Vec<(PathBuf, String, Option<u32>, Option<u32>)>,
    pub shows: Vec<(String, String, Option<i32>, Option<i64>)>,
}

pub fn tmdb_todo(conn: &Connection) -> Result<TmdbTodo> {
    let mut stmt =
        conn.prepare("SELECT path, title, year, tmdb_id FROM videos WHERE kind = 'movie' AND tmdb_done = 0 AND source != 'nfo'")?;
    let movies = stmt
        .query_map([], |r| Ok((PathBuf::from(r.get::<_, String>(0)?), r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare(
        "SELECT path, lower(trim(show)), season, episode FROM videos WHERE kind = 'episode' AND tmdb_done = 0 AND source != 'nfo'",
    )?;
    let episodes = stmt
        .query_map([], |r| Ok((PathBuf::from(r.get::<_, String>(0)?), r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare("SELECT key, name, year, tmdb_id FROM shows WHERE tmdb_done = 0 AND source != 'nfo'")?;
    let shows =
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(TmdbTodo { movies, episodes, shows })
}

/// What TMDB says about a video; `None` fields keep what's there.
#[derive(Debug, Default)]
pub struct TmdbVideo {
    pub tmdb_id: Option<i64>,
    pub title: Option<String>,
    pub year: Option<i32>,
    pub plot: Option<String>,
    pub rating: Option<f64>,
    pub genres: Option<String>,
    pub poster: Option<String>,
    pub still: Option<String>,
    pub backdrop: Option<String>,
}

pub fn apply_tmdb_video(conn: &Connection, path: &Path, t: &TmdbVideo) -> Result<()> {
    let found = t.tmdb_id.is_some();
    conn.execute(
        "UPDATE videos SET
            tmdb_id = COALESCE(?2, tmdb_id), title = COALESCE(?3, title), year = COALESCE(?4, year),
            plot = COALESCE(?5, plot), rating = COALESCE(?6, rating), genres = COALESCE(?7, genres),
            poster = CASE WHEN poster = '' OR source = 'tmdb' THEN COALESCE(?8, poster) ELSE poster END,
            still = COALESCE(?9, still),
            backdrop = CASE WHEN backdrop = '' OR source = 'tmdb' THEN COALESCE(?11, backdrop) ELSE backdrop END,
            source = CASE WHEN ?10 THEN 'tmdb' ELSE source END, tmdb_done = 1
         WHERE path = ?1",
        params![text(path), t.tmdb_id, t.title, t.year, t.plot, t.rating, t.genres, t.poster, t.still, found, t.backdrop],
    )?;
    Ok(())
}

pub fn apply_tmdb_show(conn: &Connection, key: &str, t: &TmdbVideo, backdrop: Option<String>) -> Result<()> {
    let found = t.tmdb_id.is_some();
    conn.execute(
        "UPDATE shows SET
            tmdb_id = COALESCE(?2, tmdb_id), name = COALESCE(?3, name), year = COALESCE(?4, year),
            plot = COALESCE(?5, plot), rating = COALESCE(?6, rating), genres = COALESCE(?7, genres),
            poster = CASE WHEN poster = '' OR source = 'tmdb' THEN COALESCE(?8, poster) ELSE poster END,
            backdrop = CASE WHEN backdrop = '' OR source = 'tmdb' THEN COALESCE(?9, backdrop) ELSE backdrop END,
            source = CASE WHEN ?10 THEN 'tmdb' ELSE source END, tmdb_done = 1
         WHERE key = ?1",
        params![key, t.tmdb_id, t.title, t.year, t.plot, t.rating, t.genres, t.poster, backdrop, found],
    )?;
    Ok(())
}

/// Look everything up again (a new key, or Refresh all).
pub fn reset_tmdb(conn: &Connection) -> Result<()> {
    conn.execute("UPDATE videos SET tmdb_done = 0", [])?;
    conn.execute("UPDATE shows SET tmdb_done = 0", [])?;
    Ok(())
}

/// Look one movie or show up again (after Fix match).
pub fn reset_tmdb_for(conn: &Connection, target: &str) -> Result<()> {
    if let Some(path) = target.strip_prefix("movie:") {
        conn.execute("UPDATE videos SET tmdb_done = 0 WHERE path = ?1", [path])?;
    } else if let Some(key) = target.strip_prefix("show:") {
        conn.execute("UPDATE shows SET tmdb_done = 0 WHERE key = ?1", [key])?;
        conn.execute("UPDATE videos SET tmdb_done = 0 WHERE kind = 'episode' AND lower(trim(show)) = ?1", [key])?;
    }
    Ok(())
}

// ---------- Playlists ----------

#[derive(Debug, Clone, PartialEq)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    /// How many videos it holds.
    pub count: usize,
}

pub fn playlists(conn: &Connection) -> Result<Vec<Playlist>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, (SELECT COUNT(*) FROM playlist_items WHERE playlist_id = playlists.id) \
         FROM playlists ORDER BY name COLLATE NOCASE, id",
    )?;
    let rows = stmt.query_map([], |r| Ok(Playlist { id: r.get(0)?, name: r.get(1)?, count: r.get::<_, i64>(2)? as usize }))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn playlist_paths(conn: &Connection, id: i64) -> Result<Vec<PathBuf>> {
    let mut stmt = conn.prepare("SELECT path FROM playlist_items WHERE playlist_id = ?1 ORDER BY pos")?;
    let rows = stmt.query_map([id], |r| Ok(PathBuf::from(r.get::<_, String>(0)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn create_playlist(conn: &Connection, name: &str, items: &[PathBuf]) -> Result<i64> {
    conn.execute("INSERT INTO playlists (name, created) VALUES (?1, ?2)", params![name, super::now()])?;
    let id = conn.last_insert_rowid();
    set_playlist(conn, id, items)?;
    Ok(id)
}

pub fn rename_playlist(conn: &Connection, id: i64, name: &str) -> Result<()> {
    conn.execute("UPDATE playlists SET name = ?2 WHERE id = ?1", params![id, name])?;
    Ok(())
}

pub fn delete_playlist(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM playlists WHERE id = ?1", [id])?;
    Ok(())
}

/// Replace a playlist's contents.
pub fn set_playlist(conn: &Connection, id: i64, items: &[PathBuf]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM playlist_items WHERE playlist_id = ?1", [id])?;
    {
        let mut stmt = tx.prepare("INSERT INTO playlist_items (playlist_id, pos, path) VALUES (?1, ?2, ?3)")?;
        for (i, p) in items.iter().enumerate() {
            stmt.execute(params![id, i as i64, text(p)])?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub fn append_playlist(conn: &Connection, id: i64, items: &[PathBuf]) -> Result<()> {
    let mut all = playlist_paths(conn, id)?;
    all.extend(items.iter().cloned());
    set_playlist(conn, id, &all)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (Connection, PathBuf) {
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("nyx-media-player-test-{}-{n}", std::process::id()));
        let path = dir.join("t.db");
        (open_at(&path).unwrap(), dir)
    }

    #[test]
    fn upsert_keeps_watch_state_and_added() {
        let (mut conn, dir) = temp_db();
        let v = Video { path: "/v/a.mkv".into(), title: "A".into(), added: 10, mtime: 1, size: 2, ..Default::default() };
        let tx = conn.transaction().unwrap();
        upsert(&tx, &v, "file").unwrap();
        tx.commit().unwrap();
        let w = Watch { position: 61.5, plays: 1, sid: Some(0), ..Default::default() };
        save_watch(&conn, &v.path, &w).unwrap();
        let tx = conn.transaction().unwrap();
        upsert(&tx, &Video { title: "A2".into(), added: 99, mtime: 5, ..v.clone() }, "file").unwrap();
        tx.commit().unwrap();
        let all = load_all(&conn).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].title, "A2");
        assert_eq!(all[0].added, 10);
        assert_eq!(all[0].watch.get(), w);
        assert_eq!(stamps(&conn).unwrap()[&v.path], (5, 2));
        assert_eq!(load_watch(&conn, &v.path).unwrap(), Some(w));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tmdb_fills_in_without_overriding_local_posters() {
        let (mut conn, dir) = temp_db();
        let v =
            Video { path: "/v/m.mkv".into(), kind: Kind::Movie, title: "M".into(), poster: "local".into(), ..Default::default() };
        let tx = conn.transaction().unwrap();
        upsert(&tx, &v, "file").unwrap();
        tx.commit().unwrap();
        assert_eq!(tmdb_todo(&conn).unwrap().movies.len(), 1);
        let t = TmdbVideo {
            tmdb_id: Some(7),
            title: Some("Movie".into()),
            plot: Some("Plot.".into()),
            poster: Some("remote".into()),
            ..Default::default()
        };
        apply_tmdb_video(&conn, &v.path, &t).unwrap();
        let got = &load_all(&conn).unwrap()[0];
        assert_eq!((got.title.as_str(), got.plot.as_str(), got.poster.as_str()), ("Movie", "Plot.", "local"));
        assert_eq!(got.tmdb_id, Some(7));
        assert!(tmdb_todo(&conn).unwrap().movies.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn playlists_round_trip() {
        let (conn, dir) = temp_db();
        let id = create_playlist(&conn, "Marathon", &["/a.mkv".into(), "/b.mkv".into()]).unwrap();
        append_playlist(&conn, id, &["/c.mkv".into()]).unwrap();
        assert_eq!(playlist_paths(&conn, id).unwrap(), vec![PathBuf::from("/a.mkv"), "/b.mkv".into(), "/c.mkv".into()]);
        rename_playlist(&conn, id, "Weekend").unwrap();
        assert_eq!(playlists(&conn).unwrap(), vec![Playlist { id, name: "Weekend".into(), count: 3 }]);
        delete_playlist(&conn, id).unwrap();
        assert!(playlists(&conn).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
