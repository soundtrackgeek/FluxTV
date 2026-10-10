//! SQLite persistence: sources, catalog cache, EPG, favorites, history and settings.
//!
//! One database file per installation. Catalog and EPG tables are pure caches that are
//! replaced on refresh; favorites and history are keyed by channel id (not foreign keys)
//! so they survive a refresh even if the provider briefly drops a channel.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

mod credentials;
#[cfg(test)]
mod credentials_tests;
pub use credentials::CredentialStore;
use credentials::CredentialWrites;

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::error::{Error, Result};
use crate::models::*;
use crate::search;
use crate::time::now;

const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE TABLE IF NOT EXISTS sources (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    url TEXT NOT NULL,
    username TEXT,
    password TEXT,
    epg_url TEXT,
    created_at INTEGER NOT NULL,
    catalog_updated_at INTEGER,
    epg_updated_at INTEGER
);

-- Platform credential references; no secret values belong in these tables.
CREATE TABLE IF NOT EXISTS source_secrets (
    source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    field TEXT NOT NULL,
    credential_key TEXT NOT NULL UNIQUE,
    PRIMARY KEY (source_id, field)
);
CREATE TABLE IF NOT EXISTS credential_cleanup (credential_key TEXT PRIMARY KEY);

CREATE TABLE IF NOT EXISTS categories (
    source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    id TEXT NOT NULL,
    name TEXT NOT NULL,
    position INTEGER NOT NULL,
    PRIMARY KEY (source_id, kind, id)
);

CREATE TABLE IF NOT EXISTS channels (
    source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    id TEXT NOT NULL,
    num INTEGER NOT NULL,
    name TEXT NOT NULL,
    name_norm TEXT NOT NULL,
    logo TEXT,
    category_id TEXT NOT NULL,
    epg_id TEXT,          -- as declared by the provider
    epg_match TEXT,       -- resolved XMLTV id actually used for lookups
    catchup_days INTEGER NOT NULL DEFAULT 0,
    container TEXT,
    url TEXT,
    position INTEGER NOT NULL,
    PRIMARY KEY (source_id, kind, id)
);
CREATE INDEX IF NOT EXISTS channels_by_category ON channels(source_id, kind, category_id, position);

CREATE TABLE IF NOT EXISTS epg_channels (
    source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    id TEXT NOT NULL,
    names TEXT NOT NULL,  -- display names joined by '\n'
    icon TEXT,
    PRIMARY KEY (source_id, id)
);

CREATE TABLE IF NOT EXISTS programmes (
    source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    channel TEXT NOT NULL,
    start INTEGER NOT NULL,
    stop INTEGER NOT NULL,
    title TEXT NOT NULL,
    subtitle TEXT,
    description TEXT,
    category TEXT,
    icon TEXT,
    episode TEXT,
    PRIMARY KEY (source_id, channel, start)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS favorites (
    source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    added_at INTEGER NOT NULL,
    PRIMARY KEY (source_id, kind, channel_id)
);

CREATE TABLE IF NOT EXISTS history (
    source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    watched_at INTEGER NOT NULL,
    PRIMARY KEY (source_id, kind, channel_id)
);
"#;

/// Longest programme we expect; bounds the "what is on now" index scan.
const MAX_PROGRAMME_SECS: i64 = 12 * 3600;

/// Thread-safe handle to the database. Reads and small writes share one connection;
/// bulk imports open a dedicated connection so the UI stays responsive (WAL mode).
pub struct Store {
    path: Option<PathBuf>,
    conn: Mutex<Connection>,
    /// Passwords held only in memory (the Apple apps keep them in the Keychain and hand
    /// them over at launch). Used when the database row has no password.
    runtime_passwords: Mutex<HashMap<i64, String>>,
    credentials: Option<Arc<dyn CredentialStore>>,
}

fn configure(conn: &Connection) -> Result<()> {
    conn.busy_timeout(std::time::Duration::from_secs(30))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;")?;
    Ok(())
}

const CHANNEL_COLUMNS: &str = "c.id, c.kind, c.num, c.name, c.logo, c.category_id, c.epg_match, c.catchup_days, \
     c.container, c.url, (f.channel_id IS NOT NULL) AS fav";

fn channel_from_row(r: &Row<'_>) -> rusqlite::Result<Channel> {
    Ok(Channel {
        id: r.get(0)?,
        kind: ContentKind::parse(&r.get::<_, String>(1)?).unwrap_or(ContentKind::Live),
        num: r.get(2)?,
        name: r.get(3)?,
        logo: r.get(4)?,
        category_id: r.get(5)?,
        epg_id: r.get(6)?,
        catchup_days: r.get(7)?,
        container: r.get(8)?,
        url: r.get(9)?,
        favorite: r.get(10)?,
    })
}

fn programme_from_row(r: &Row<'_>) -> rusqlite::Result<Programme> {
    Ok(Programme {
        channel: r.get(0)?,
        start: r.get(1)?,
        stop: r.get(2)?,
        title: r.get(3)?,
        subtitle: r.get(4)?,
        description: r.get(5)?,
        category: r.get(6)?,
        icon: r.get(7)?,
        episode: r.get(8)?,
    })
}

fn source_from_row(r: &Row<'_>) -> rusqlite::Result<Source> {
    Ok(Source {
        id: r.get(0)?,
        name: r.get(1)?,
        kind: SourceKind::parse(&r.get::<_, String>(2)?).unwrap_or(SourceKind::M3u),
        url: r.get(3)?,
        username: r.get(4)?,
        password: r.get(5)?,
        epg_url: r.get(6)?,
        created_at: r.get(7)?,
        catalog_updated_at: r.get(8)?,
        epg_updated_at: r.get(9)?,
    })
}

const PROGRAMME_COLUMNS: &str = "channel, start, stop, title, subtitle, description, category, icon, episode";

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_impl(path.as_ref(), None)
    }

    /// Open with platform credential storage and migrate legacy plaintext on startup.
    pub fn open_with_credentials(path: impl AsRef<Path>, credentials: Arc<dyn CredentialStore>) -> Result<Self> {
        Self::open_impl(path.as_ref(), Some(credentials))
    }

    fn open_impl(path: &Path, credentials: Option<Arc<dyn CredentialStore>>) -> Result<Self> {
        let path = path.to_path_buf();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(&path)?;
        configure(&conn)?;
        let store = Self { path: Some(path), conn: Mutex::new(conn), runtime_passwords: Mutex::default(), credentials };
        store.migrate()?;
        store.migrate_credentials()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        configure(&conn)?;
        let store = Self { path: None, conn: Mutex::new(conn), runtime_passwords: Mutex::default(), credentials: None };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.conn();
        conn.execute_batch(SCHEMA)?;
        conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES ('schema_version', ?1)",
            [SCHEMA_VERSION.to_string()],
        )?;
        Ok(())
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        // A panic while holding the lock leaves the connection itself usable.
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run `f` on a dedicated connection (file databases) or the shared one (in-memory).
    fn with_bulk_conn<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        match &self.path {
            Some(p) => {
                let mut c = Connection::open(p)?;
                configure(&c)?;
                f(&mut c)
            }
            None => f(&mut self.conn()),
        }
    }

    // ----- settings -----------------------------------------------------------------------

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        if let Some(id) = Self::guide_setting_id(key) {
            let conn = self.conn();
            if let Some(value) = self.secret_setting(&conn, id)? {
                return Ok(Some(value));
            }
        }
        Ok(self
            .conn()
            .query_row("SELECT value FROM meta WHERE key = ?1", [format!("setting.{key}")], |r| r.get(0))
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: Option<&str>) -> Result<()> {
        let k = format!("setting.{key}");
        if let (Some(vault), Some(id)) = (&self.credentials, Self::guide_setting_id(key)) {
            let mut conn = self.conn();
            let tx = conn.transaction()?;
            let mut writes = CredentialWrites::new(vault.clone());
            writes.write(&tx, id, "discovered_epg", value)?;
            tx.execute("DELETE FROM meta WHERE key = ?1", [k])?;
            tx.commit()?;
            writes.commit();
            self.cleanup_credentials(&conn)?;
            return Ok(());
        }
        match value {
            Some(v) => self.conn().execute("INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)", params![k, v])?,
            None => self.conn().execute("DELETE FROM meta WHERE key = ?1", [k])?,
        };
        Ok(())
    }

    // ----- sources ------------------------------------------------------------------------

    pub fn insert_source(&self, s: &NewSource) -> Result<Source> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let secure = self.credentials.is_some();
        tx.execute(
            "INSERT INTO sources(name, kind, url, username, password, epg_url, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![s.name, s.kind.as_str(), if secure && s.kind == SourceKind::M3u { "" } else { &s.url },
                s.username, if secure { None } else { s.password.as_deref() },
                if secure { None } else { s.epg_url.as_deref() }, now()],
        )?;
        let id = tx.last_insert_rowid();
        let mut writes = self.credentials.as_ref().map(|v| CredentialWrites::new(v.clone()));
        if let Some(writes) = &mut writes {
            writes.source(&tx, id, s)?;
        }
        tx.commit()?;
        if let Some(writes) = writes {
            writes.commit();
        }
        drop(conn);
        self.source(id)
    }

    pub fn update_source(&self, id: i64, s: &NewSource) -> Result<Source> {
        let mut conn = self.conn();
        let existing = conn.query_row(
            "SELECT id, name, kind, url, username, password, epg_url, created_at, catalog_updated_at, epg_updated_at FROM sources WHERE id = ?1",
            [id], source_from_row,
        ).optional()?.ok_or_else(|| Error::NotFound(format!("source {id}")))?;
        let existing = self.hydrate_secrets(&conn, existing)?;
        let origin_changed = existing.kind != s.kind || existing.url != s.url;
        let tx = conn.transaction()?;
        let secure = self.credentials.is_some();
        let changed = tx.execute(
            "UPDATE sources SET name = ?2, kind = ?3, url = ?4, username = ?5,
                 password = CASE WHEN ?8 THEN NULL ELSE COALESCE(?6, password) END, epg_url = ?7 WHERE id = ?1",
            params![
                id,
                s.name,
                s.kind.as_str(),
                if secure && s.kind == SourceKind::M3u { "" } else { &s.url },
                s.username,
                if secure { None } else { s.password.as_deref() },
                if secure { None } else { s.epg_url.as_deref() },
                secure
            ],
        )?;
        if changed == 0 {
            return Err(Error::NotFound(format!("source {id}")));
        }
        let mut writes = self.credentials.as_ref().map(|v| CredentialWrites::new(v.clone()));
        if let Some(writes) = &mut writes {
            // Blank Xtream passwords mean unchanged, including direct Store callers.
            if s.kind != SourceKind::Xtream || s.password.as_deref().is_some_and(|p| !p.is_empty()) {
                writes.write(
                    &tx,
                    id,
                    "password",
                    if s.kind == SourceKind::Xtream { s.password.as_deref() } else { None },
                )?;
            }
            writes.write(&tx, id, "url", (s.kind == SourceKind::M3u).then_some(s.url.as_str()))?;
            writes.write(&tx, id, "epg_url", s.epg_url.as_deref())?;
            // A discovered guide belongs to the old playlist, not to the new provider URL.
            if origin_changed {
                writes.write(&tx, id, "discovered_epg", None)?;
            }
        }
        tx.commit()?;
        if let Some(writes) = writes {
            writes.commit();
        }
        if secure { self.set_runtime_password(id, None); }
        self.cleanup_credentials(&conn)?;
        drop(conn);
        self.source(id)
    }

    pub fn sources(&self) -> Result<Vec<Source>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, name, kind, url, username, password, epg_url, created_at, catalog_updated_at, epg_updated_at
             FROM sources ORDER BY id",
        )?;
        let rows = stmt.query_map([], source_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(|s| self.hydrate_secrets(&conn, s)).collect()
    }

    pub fn source(&self, id: i64) -> Result<Source> {
        let conn = self.conn();
        let source = conn
            .query_row(
                "SELECT id, name, kind, url, username, password, epg_url, created_at, catalog_updated_at, epg_updated_at
                 FROM sources WHERE id = ?1",
                [id],
                source_from_row,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("source {id}")))?;
        self.hydrate_secrets(&conn, source)
    }

    /// Provide a source's password for this session without writing it to the database.
    pub fn set_runtime_password(&self, id: i64, password: Option<String>) {
        let mut map = self.runtime_passwords.lock().unwrap_or_else(|e| e.into_inner());
        match password.filter(|p| !p.is_empty()) {
            Some(p) => map.insert(id, p),
            None => map.remove(&id),
        };
    }

    fn with_runtime_password(&self, mut s: Source) -> Source {
        if s.password.is_none() {
            s.password = self.runtime_passwords.lock().unwrap_or_else(|e| e.into_inner()).get(&s.id).cloned();
        }
        s
    }

    pub fn delete_source(&self, id: i64) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO credential_cleanup SELECT credential_key FROM source_secrets WHERE source_id = ?1",
            [id],
        )?;
        tx.execute("DELETE FROM meta WHERE key = ?1", [format!("setting.m3u_epg_url.{id}")])?;
        tx.execute("DELETE FROM sources WHERE id = ?1", [id])?;
        tx.commit()?;
        self.set_runtime_password(id, None);
        self.cleanup_credentials(&conn)
    }

    // ----- catalog ------------------------------------------------------------------------

    /// Replace the catalog (categories + channels) for the given kinds of a source.
    pub fn replace_catalog(
        &self,
        source_id: i64,
        kinds: &[ContentKind],
        categories: &[Category],
        channels: &[Channel],
    ) -> Result<()> {
        self.with_bulk_conn(|c| {
            let tx = c.transaction()?;
            for k in kinds {
                tx.execute("DELETE FROM categories WHERE source_id = ?1 AND kind = ?2", params![source_id, k.as_str()])?;
                tx.execute("DELETE FROM channels WHERE source_id = ?1 AND kind = ?2", params![source_id, k.as_str()])?;
            }
            {
                let mut ins = tx.prepare(
                    "INSERT OR IGNORE INTO categories(source_id, kind, id, name, position) VALUES (?1, ?2, ?3, ?4, ?5)",
                )?;
                for (i, cat) in categories.iter().enumerate() {
                    ins.execute(params![source_id, cat.kind.as_str(), cat.id, cat.name, i as i64])?;
                }
                let mut ins = tx.prepare(
                    "INSERT OR REPLACE INTO channels(source_id, kind, id, num, name, name_norm, logo, category_id,
                         epg_id, epg_match, catchup_days, container, url, position)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10, ?11, ?12, ?13)",
                )?;
                for (i, ch) in channels.iter().enumerate() {
                    ins.execute(params![
                        source_id,
                        ch.kind.as_str(),
                        ch.id,
                        ch.num,
                        ch.name,
                        search::normalize(&ch.name),
                        ch.logo,
                        ch.category_id,
                        ch.epg_id,
                        ch.catchup_days,
                        ch.container,
                        ch.url,
                        i as i64
                    ])?;
                }
            }
            tx.execute("UPDATE sources SET catalog_updated_at = ?2 WHERE id = ?1", params![source_id, now()])?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn categories(&self, source_id: i64, kind: ContentKind) -> Result<Vec<Category>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT cat.id, cat.name, COUNT(ch.id)
             FROM categories cat
             LEFT JOIN channels ch ON ch.source_id = cat.source_id AND ch.kind = cat.kind AND ch.category_id = cat.id
             WHERE cat.source_id = ?1 AND cat.kind = ?2
             GROUP BY cat.id ORDER BY cat.position",
        )?;
        let rows = stmt
            .query_map(params![source_id, kind.as_str()], |r| {
                Ok(Category { id: r.get(0)?, name: r.get(1)?, kind, channel_count: r.get(2)? })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn channels_in_category(&self, source_id: i64, kind: ContentKind, category_id: &str) -> Result<Vec<Channel>> {
        let conn = self.conn();
        let sql = format!(
            "SELECT {CHANNEL_COLUMNS} FROM channels c
             LEFT JOIN favorites f ON f.source_id = c.source_id AND f.kind = c.kind AND f.channel_id = c.id
             WHERE c.source_id = ?1 AND c.kind = ?2 AND c.category_id = ?3 ORDER BY c.position"
        );
        let mut stmt = conn.prepare_cached(&sql)?;
        let rows = stmt
            .query_map(params![source_id, kind.as_str(), category_id], channel_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn channel(&self, source_id: i64, kind: ContentKind, id: &str) -> Result<Channel> {
        let conn = self.conn();
        let sql = format!(
            "SELECT {CHANNEL_COLUMNS} FROM channels c
             LEFT JOIN favorites f ON f.source_id = c.source_id AND f.kind = c.kind AND f.channel_id = c.id
             WHERE c.source_id = ?1 AND c.kind = ?2 AND c.id = ?3"
        );
        conn.query_row(&sql, params![source_id, kind.as_str(), id], channel_from_row)
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("channel {id}")))
    }

    pub fn channel_count(&self, source_id: i64, kind: ContentKind) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) FROM channels WHERE source_id = ?1 AND kind = ?2",
            params![source_id, kind.as_str()],
            |r| r.get(0),
        )?)
    }

    /// Ranked search over channel names. All query words must match.
    pub fn search_channels(&self, source_id: i64, kind: ContentKind, query: &str, limit: usize) -> Result<Vec<Channel>> {
        let tokens = search::tokens(query);
        let Some(pivot) = search::most_selective_token(&tokens) else {
            return Ok(Vec::new());
        };
        let conn = self.conn();
        let sql = format!(
            "SELECT {CHANNEL_COLUMNS}, c.name_norm FROM channels c
             LEFT JOIN favorites f ON f.source_id = c.source_id AND f.kind = c.kind AND f.channel_id = c.id
             WHERE c.source_id = ?1 AND c.kind = ?2 AND (c.name_norm LIKE ?3 ESCAPE '\\' OR REPLACE(c.name_norm, ' ', '') LIKE ?3 ESCAPE '\\')"
        );
        let like = format!("%{}%", pivot.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        let mut stmt = conn.prepare_cached(&sql)?;
        let mut scored: Vec<(i64, i64, Channel)> = Vec::new();
        let mut rows = stmt.query(params![source_id, kind.as_str(), like])?;
        let mut pos = 0i64;
        while let Some(r) = rows.next()? {
            let norm: String = r.get(11)?;
            if let Some(mut s) = search::score(&norm, &tokens) {
                let ch = channel_from_row(r)?;
                if ch.favorite {
                    s += 15;
                }
                scored.push((s, pos, ch));
            }
            pos += 1;
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        Ok(scored.into_iter().take(limit).map(|(_, _, c)| c).collect())
    }

    // ----- favorites & history ------------------------------------------------------------

    /// Toggle a favorite; returns the new state.
    pub fn toggle_favorite(&self, source_id: i64, kind: ContentKind, channel_id: &str) -> Result<bool> {
        let conn = self.conn();
        let removed = conn.execute(
            "DELETE FROM favorites WHERE source_id = ?1 AND kind = ?2 AND channel_id = ?3",
            params![source_id, kind.as_str(), channel_id],
        )?;
        if removed > 0 {
            return Ok(false);
        }
        conn.execute(
            "INSERT INTO favorites(source_id, kind, channel_id, added_at) VALUES (?1, ?2, ?3, ?4)",
            params![source_id, kind.as_str(), channel_id, now()],
        )?;
        Ok(true)
    }

    /// Set a favorite explicitly (used to restore favorites from a backup).
    pub fn set_favorite(&self, source_id: i64, kind: ContentKind, channel_id: &str, favorite: bool) -> Result<()> {
        let conn = self.conn();
        if favorite {
            conn.execute(
                "INSERT OR IGNORE INTO favorites(source_id, kind, channel_id, added_at) VALUES (?1, ?2, ?3, ?4)",
                params![source_id, kind.as_str(), channel_id, now()],
            )?;
        } else {
            conn.execute(
                "DELETE FROM favorites WHERE source_id = ?1 AND kind = ?2 AND channel_id = ?3",
                params![source_id, kind.as_str(), channel_id],
            )?;
        }
        Ok(())
    }

    /// Favorite channel ids in the order they were added, including channels missing from
    /// the current catalog.
    pub fn favorite_ids(&self, source_id: i64, kind: ContentKind) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT channel_id FROM favorites WHERE source_id = ?1 AND kind = ?2 ORDER BY added_at, rowid",
        )?;
        let rows = stmt
            .query_map(params![source_id, kind.as_str()], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(rows)
    }

    pub fn favorites(&self, source_id: i64, kind: ContentKind) -> Result<Vec<Channel>> {
        let conn = self.conn();
        let sql = format!(
            "SELECT {CHANNEL_COLUMNS} FROM favorites f
             JOIN channels c ON f.source_id = c.source_id AND f.kind = c.kind AND f.channel_id = c.id
             WHERE f.source_id = ?1 AND f.kind = ?2 ORDER BY f.added_at, c.position"
        );
        let mut stmt = conn.prepare_cached(&sql)?;
        let rows = stmt
            .query_map(params![source_id, kind.as_str()], channel_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn record_watch(&self, source_id: i64, kind: ContentKind, channel_id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT OR REPLACE INTO history(source_id, kind, channel_id, watched_at) VALUES (?1, ?2, ?3, ?4)",
            params![source_id, kind.as_str(), channel_id, now()],
        )?;
        // Keep history bounded.
        conn.execute(
            "DELETE FROM history WHERE source_id = ?1 AND kind = ?2 AND channel_id NOT IN
               (SELECT channel_id FROM history WHERE source_id = ?1 AND kind = ?2 ORDER BY watched_at DESC LIMIT 200)",
            params![source_id, kind.as_str()],
        )?;
        Ok(())
    }

    pub fn recent(&self, source_id: i64, kind: ContentKind, limit: usize) -> Result<Vec<Channel>> {
        let conn = self.conn();
        let sql = format!(
            "SELECT {CHANNEL_COLUMNS} FROM history h
             JOIN channels c ON h.source_id = c.source_id AND h.kind = c.kind AND h.channel_id = c.id
             LEFT JOIN favorites f ON f.source_id = c.source_id AND f.kind = c.kind AND f.channel_id = c.id
             WHERE h.source_id = ?1 AND h.kind = ?2 ORDER BY h.watched_at DESC LIMIT ?3"
        );
        let mut stmt = conn.prepare_cached(&sql)?;
        let rows = stmt
            .query_map(params![source_id, kind.as_str(), limit as i64], channel_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn clear_history(&self, source_id: i64) -> Result<()> {
        self.conn().execute("DELETE FROM history WHERE source_id = ?1", [source_id])?;
        Ok(())
    }

    // ----- EPG ----------------------------------------------------------------------------

    /// Import a guide. `feed` receives an `EpgSink` to push channels/programmes into; everything
    /// happens in one transaction so readers never see a half-imported guide.
    /// Programmes that ended before `keep_after` are dropped.
    pub fn import_epg<F>(&self, source_id: i64, keep_after: i64, feed: F) -> Result<(usize, usize)>
    where
        F: FnOnce(&mut EpgSink<'_>) -> Result<()>,
    {
        self.with_bulk_conn(|c| {
            let tx = c.transaction()?;
            tx.execute("DELETE FROM epg_channels WHERE source_id = ?1", [source_id])?;
            tx.execute("DELETE FROM programmes WHERE source_id = ?1", [source_id])?;
            let (channels, programmes) = {
                let mut sink = EpgSink {
                    source_id,
                    keep_after,
                    channel_stmt: tx.prepare(
                        "INSERT OR REPLACE INTO epg_channels(source_id, id, names, icon) VALUES (?1, ?2, ?3, ?4)",
                    )?,
                    programme_stmt: tx.prepare(
                        "INSERT OR REPLACE INTO programmes(source_id, channel, start, stop, title, subtitle, description, category, icon, episode)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    )?,
                    channels: 0,
                    programmes: 0,
                    error: None,
                };
                feed(&mut sink)?;
                if let Some(e) = sink.error.take() {
                    return Err(e);
                }
                (sink.channels, sink.programmes)
            };
            tx.execute("UPDATE sources SET epg_updated_at = ?2 WHERE id = ?1", params![source_id, now()])?;
            tx.commit()?;
            Ok((channels, programmes))
        })
    }

    /// Resolve which XMLTV channel each catalog channel should use.
    ///
    /// 1. The provider's `epg_id`/`tvg-id` if the guide knows it.
    /// 2. Otherwise an exact match of the normalized channel name against guide display names.
    /// 3. Otherwise a looser match with country prefixes and quality tags (`HD`, `RAW`...) removed.
    ///
    /// Returns how many channels were matched by name.
    pub fn resolve_epg_matches(&self, source_id: i64) -> Result<usize> {
        self.with_bulk_conn(|c| {
            let tx = c.transaction()?;
            let mut known: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut exact: HashMap<String, String> = HashMap::new();
            let mut loose: HashMap<String, String> = HashMap::new();
            {
                let mut stmt = tx.prepare("SELECT id, names FROM epg_channels WHERE source_id = ?1")?;
                let mut rows = stmt.query([source_id])?;
                while let Some(r) = rows.next()? {
                    let id: String = r.get(0)?;
                    let names: String = r.get(1)?;
                    for n in names.split('\n').filter(|n| !n.is_empty()) {
                        exact.entry(search::normalize(n)).or_insert_with(|| id.clone());
                        let key = search::epg_match_key(n);
                        if !key.is_empty() {
                            loose.entry(key).or_insert_with(|| id.clone());
                        }
                    }
                    known.insert(id);
                }
            }
            if known.is_empty() {
                // No guide yet: keep provider ids as-is.
                tx.execute("UPDATE channels SET epg_match = epg_id WHERE source_id = ?1", [source_id])?;
                tx.commit()?;
                return Ok(0);
            }

            let mut updates: Vec<(String, String, Option<String>)> = Vec::new();
            let mut by_name = 0;
            {
                let mut stmt = tx.prepare(
                    "SELECT kind, id, name, name_norm, epg_id, epg_match FROM channels WHERE source_id = ?1 AND kind = 'live'",
                )?;
                let mut rows = stmt.query([source_id])?;
                while let Some(r) = rows.next()? {
                    let kind: String = r.get(0)?;
                    let id: String = r.get(1)?;
                    let name: String = r.get(2)?;
                    let norm: String = r.get(3)?;
                    let epg_id: Option<String> = r.get(4)?;
                    let current: Option<String> = r.get(5)?;
                    let resolved = match epg_id.filter(|e| known.contains(e)) {
                        Some(e) => Some(e),
                        None => {
                            let m = exact.get(&norm).or_else(|| loose.get(&search::epg_match_key(&name))).cloned();
                            if m.is_some() {
                                by_name += 1;
                            }
                            m
                        }
                    };
                    if resolved != current {
                        updates.push((kind, id, resolved));
                    }
                }
            }
            {
                let mut upd = tx.prepare("UPDATE channels SET epg_match = ?4 WHERE source_id = ?1 AND kind = ?2 AND id = ?3")?;
                for (kind, id, m) in &updates {
                    upd.execute(params![source_id, kind, id, m])?;
                }
            }
            tx.commit()?;
            Ok(by_name)
        })
    }

    /// What is on now and next for each requested XMLTV id.
    pub fn now_next(&self, source_id: i64, epg_ids: &[String], at: i64) -> Result<Vec<NowNext>> {
        let conn = self.conn();
        let sql = format!(
            "SELECT {PROGRAMME_COLUMNS} FROM programmes
             WHERE source_id = ?1 AND channel = ?2 AND start >= ?3 AND stop > ?4 ORDER BY start LIMIT 2"
        );
        let mut stmt = conn.prepare_cached(&sql)?;
        let mut out = Vec::with_capacity(epg_ids.len());
        for id in epg_ids {
            let progs = stmt
                .query_map(params![source_id, id, at - MAX_PROGRAMME_SECS, at], programme_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut it = progs.into_iter();
            let (now, next) = match it.next() {
                Some(p) if p.start <= at => (Some(p), it.next()),
                Some(p) => (None, Some(p)),
                None => (None, None),
            };
            out.push(NowNext { epg_id: id.clone(), now, next });
        }
        Ok(out)
    }

    /// Programmes overlapping `[from, to)` for each channel, for the guide grid.
    pub fn guide(&self, source_id: i64, epg_ids: &[String], from: i64, to: i64) -> Result<Vec<GuideRow>> {
        let conn = self.conn();
        let sql = format!(
            "SELECT {PROGRAMME_COLUMNS} FROM programmes
             WHERE source_id = ?1 AND channel = ?2 AND start >= ?3 AND start < ?4 AND stop > ?5 ORDER BY start"
        );
        let mut stmt = conn.prepare_cached(&sql)?;
        let mut out = Vec::with_capacity(epg_ids.len());
        for id in epg_ids {
            let programmes = stmt
                .query_map(params![source_id, id, from - MAX_PROGRAMME_SECS, to, from], programme_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            out.push(GuideRow { epg_id: id.clone(), programmes });
        }
        Ok(out)
    }

    pub fn epg_stats(&self, source_id: i64) -> Result<(i64, i64)> {
        let conn = self.conn();
        let channels = conn.query_row("SELECT COUNT(*) FROM epg_channels WHERE source_id = ?1", [source_id], |r| r.get(0))?;
        let progs = conn.query_row("SELECT COUNT(*) FROM programmes WHERE source_id = ?1", [source_id], |r| r.get(0))?;
        Ok((channels, progs))
    }
}

/// Write handle passed to [`Store::import_epg`] callbacks.
pub struct EpgSink<'a> {
    source_id: i64,
    keep_after: i64,
    channel_stmt: rusqlite::Statement<'a>,
    programme_stmt: rusqlite::Statement<'a>,
    channels: usize,
    programmes: usize,
    error: Option<Error>,
}

impl EpgSink<'_> {
    pub fn channel(&mut self, c: &EpgChannel) {
        if self.error.is_some() {
            return;
        }
        let names = c.display_names.join("\n");
        match self.channel_stmt.execute(params![self.source_id, c.id, names, c.icon]) {
            Ok(_) => self.channels += 1,
            Err(e) => self.error = Some(e.into()),
        }
    }

    pub fn programme(&mut self, p: &Programme) {
        if self.error.is_some() || p.stop <= self.keep_after {
            return;
        }
        match self.programme_stmt.execute(params![
            self.source_id,
            p.channel,
            p.start,
            p.stop,
            p.title,
            p.subtitle,
            p.description,
            p.category,
            p.icon,
            p.episode
        ]) {
            Ok(_) => self.programmes += 1,
            Err(e) => self.error = Some(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_source() -> (Store, i64) {
        let s = Store::open_in_memory().unwrap();
        let src = s
            .insert_source(&NewSource {
                name: "Test".into(),
                kind: SourceKind::Xtream,
                url: "http://h".into(),
                username: Some("u".into()),
                password: Some("p".into()),
                epg_url: None,
            })
            .unwrap();
        (s, src.id)
    }

    fn ch(id: &str, name: &str, cat: &str, epg: Option<&str>) -> Channel {
        Channel {
            id: id.into(),
            kind: ContentKind::Live,
            num: 0,
            name: name.into(),
            logo: None,
            category_id: cat.into(),
            epg_id: epg.map(Into::into),
            catchup_days: 0,
            container: None,
            favorite: false,
            url: None,
        }
    }

    fn prog(channel: &str, start: i64, stop: i64, title: &str) -> Programme {
        Programme {
            channel: channel.into(),
            start,
            stop,
            title: title.into(),
            subtitle: None,
            description: None,
            category: None,
            icon: None,
            episode: None,
        }
    }

    fn seed(s: &Store, id: i64) {
        let cats = vec![
            Category { id: "1".into(), name: "Norway".into(), kind: ContentKind::Live, channel_count: 0 },
            Category { id: "2".into(), name: "UK".into(), kind: ContentKind::Live, channel_count: 0 },
        ];
        let chans = vec![
            ch("10", "NOR| NRK1 HD", "1", Some("nrk1.no")),
            ch("11", "GOLD| NRK1 ᴿᴬᵂ", "1", None),
            ch("12", "NOR| TV2 HD", "1", None),
            ch("20", "UK| ALIBI FHD", "2", None),
        ];
        s.replace_catalog(id, &[ContentKind::Live], &cats, &chans).unwrap();
    }

    #[test]
    fn source_password_is_not_serialized() {
        let (s, id) = store_with_source();
        let src = s.source(id).unwrap();
        assert_eq!(src.password.as_deref(), Some("p"));
        let json = serde_json::to_string(&src).unwrap();
        assert!(!json.contains("password"));
    }

    #[test]
    fn catalog_categories_and_search() {
        let (s, id) = store_with_source();
        seed(&s, id);
        let cats = s.categories(id, ContentKind::Live).unwrap();
        assert_eq!(cats.iter().map(|c| c.channel_count).collect::<Vec<_>>(), vec![3, 1]);
        assert_eq!(s.channels_in_category(id, ContentKind::Live, "1").unwrap().len(), 3);

        let r = s.search_channels(id, ContentKind::Live, "nrk1", 10).unwrap();
        assert_eq!(r.len(), 2);
        let r = s.search_channels(id, ContentKind::Live, "nrk raw", 10).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id, "11");
        assert!(s.search_channels(id, ContentKind::Live, "100%", 10).unwrap().is_empty());
    }

    #[test]
    fn favorites_and_history_survive_refresh() {
        let (s, id) = store_with_source();
        seed(&s, id);
        assert!(s.toggle_favorite(id, ContentKind::Live, "12").unwrap());
        s.record_watch(id, ContentKind::Live, "20").unwrap();
        seed(&s, id);
        let favs = s.favorites(id, ContentKind::Live).unwrap();
        assert_eq!(favs.len(), 1);
        assert!(favs[0].favorite);
        assert_eq!(s.recent(id, ContentKind::Live, 10).unwrap()[0].id, "20");
        assert!(!s.toggle_favorite(id, ContentKind::Live, "12").unwrap());
        assert!(s.favorites(id, ContentKind::Live).unwrap().is_empty());
    }

    #[test]
    fn runtime_passwords_stay_out_of_the_database() {
        let s = Store::open_in_memory().unwrap();
        let src = s
            .insert_source(&NewSource {
                name: "Keychain".into(),
                kind: SourceKind::Xtream,
                url: "http://h".into(),
                username: Some("u".into()),
                password: None,
                epg_url: None,
            })
            .unwrap();
        assert_eq!(src.password, None);
        s.set_runtime_password(src.id, Some("secret".into()));
        assert_eq!(s.source(src.id).unwrap().password.as_deref(), Some("secret"));
        assert_eq!(s.sources().unwrap()[0].password.as_deref(), Some("secret"));
        let stored: Option<String> =
            s.conn().query_row("SELECT password FROM sources WHERE id = ?1", [src.id], |r| r.get(0)).unwrap();
        assert_eq!(stored, None);
    }

    #[test]
    fn favorites_can_be_restored() {
        let (s, id) = store_with_source();
        seed(&s, id);
        s.set_favorite(id, ContentKind::Live, "20", true).unwrap();
        s.set_favorite(id, ContentKind::Live, "gone", true).unwrap();
        s.set_favorite(id, ContentKind::Live, "20", true).unwrap();
        assert_eq!(s.favorite_ids(id, ContentKind::Live).unwrap(), vec!["20".to_string(), "gone".to_string()]);
        assert_eq!(s.favorites(id, ContentKind::Live).unwrap().len(), 1);
        s.set_favorite(id, ContentKind::Live, "gone", false).unwrap();
        assert_eq!(s.favorite_ids(id, ContentKind::Live).unwrap(), vec!["20".to_string()]);
    }

    #[test]
    fn epg_import_matching_and_now_next() {
        let (s, id) = store_with_source();
        seed(&s, id);
        let t = 1_000_000;
        let (c, p) = s
            .import_epg(id, t - 7200, |sink| {
                sink.channel(&EpgChannel { id: "nrk1.no".into(), display_names: vec!["NRK1".into()], icon: None });
                sink.channel(&EpgChannel {
                    id: "alibi.uk".into(),
                    display_names: vec!["Alibi".into(), "UK| ALIBI FHD".into()],
                    icon: None,
                });
                sink.programme(&prog("nrk1.no", t - 20_000, t - 10_000, "Too old"));
                sink.programme(&prog("nrk1.no", t - 1800, t + 1800, "Now"));
                sink.programme(&prog("nrk1.no", t + 1800, t + 3600, "Next"));
                sink.programme(&prog("alibi.uk", t + 600, t + 4200, "Later"));
                Ok(())
            })
            .unwrap();
        assert_eq!((c, p), (2, 3));

        let by_name = s.resolve_epg_matches(id).unwrap();
        // "GOLD| NRK1 ᴿᴬᵂ" matches loosely to NRK1, "UK| ALIBI FHD" exactly.
        assert_eq!(by_name, 2);
        let norway = s.channels_in_category(id, ContentKind::Live, "1").unwrap();
        assert_eq!(norway[1].epg_id.as_deref(), Some("nrk1.no"));
        assert_eq!(norway[2].epg_id, None);

        let nn = s.now_next(id, &["nrk1.no".into(), "alibi.uk".into(), "none".into()], t).unwrap();
        assert_eq!(nn[0].now.as_ref().unwrap().title, "Now");
        assert_eq!(nn[0].next.as_ref().unwrap().title, "Next");
        assert!(nn[1].now.is_none());
        assert_eq!(nn[1].next.as_ref().unwrap().title, "Later");
        assert!(nn[2].now.is_none() && nn[2].next.is_none());

        let g = s.guide(id, &["nrk1.no".into()], t, t + 3000).unwrap();
        assert_eq!(g[0].programmes.len(), 2);
    }
}
