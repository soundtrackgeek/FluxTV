//! Optional platform credential storage. SQLite contains only random references to secrets.

use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, params};

use super::{Store, source_from_row};
use crate::{Error, NewSource, Result, Source, SourceKind};

/// Implemented by the platform shell, so the core never depends on an OS credential API.
/// Errors must not include secret values. Missing entries must return an error on reads;
/// deleting an already absent entry should succeed.
pub trait CredentialStore: Send + Sync {
    fn get(&self, key: &str) -> Result<String>;
    fn set(&self, key: &str, value: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
}

/// Rolls back newly written credentials if a database transaction or vault operation fails.
pub(super) struct CredentialWrites {
    vault: Arc<dyn CredentialStore>,
    created: Vec<String>,
}

impl CredentialWrites {
    pub fn new(vault: Arc<dyn CredentialStore>) -> Self {
        Self { vault, created: Vec::new() }
    }

    pub fn commit(mut self) {
        self.created.clear();
    }

    pub fn write(&mut self, conn: &Connection, id: i64, field: &str, value: Option<&str>) -> Result<()> {
        let old: Option<String> = conn
            .query_row(
                "SELECT credential_key FROM source_secrets WHERE source_id = ?1 AND field = ?2",
                params![id, field],
                |r| r.get(0),
            )
            .optional()?;
        let key = if let Some(value) = value {
            // Fresh keys keep a failed edit from overwriting the working credentials and
            // isolate databases/installations even when SQLite reuses a source id.
            let key: String = conn.query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))?;
            self.created.push(key.clone());
            self.vault.set(&key, value)?;
            if self.vault.get(&key)? != value {
                return Err(Error::Io(std::io::Error::other("Credential storage verification failed")));
            }
            Some(key)
        } else {
            None
        };
        conn.execute("DELETE FROM source_secrets WHERE source_id = ?1 AND field = ?2", params![id, field])?;
        if let Some(key) = key {
            conn.execute("INSERT INTO source_secrets VALUES (?1, ?2, ?3)", params![id, field, key])?;
        }
        if let Some(old) = old {
            conn.execute("INSERT OR IGNORE INTO credential_cleanup VALUES (?1)", [old])?;
        }
        Ok(())
    }

    pub fn source(&mut self, conn: &Connection, id: i64, source: &NewSource) -> Result<()> {
        self.write(
            conn,
            id,
            "password",
            if source.kind == SourceKind::Xtream { source.password.as_deref().filter(|p| !p.is_empty()) } else { None },
        )?;
        self.write(conn, id, "url", (source.kind == SourceKind::M3u).then_some(source.url.as_str()))?;
        self.write(conn, id, "epg_url", source.epg_url.as_deref())
    }
}

impl Drop for CredentialWrites {
    fn drop(&mut self) {
        for key in &self.created {
            if let Err(e) = self.vault.delete(key) {
                log::warn!("could not clean up an uncommitted credential: {e}");
            }
        }
    }
}

impl Store {
    pub(super) fn hydrate_secrets(&self, conn: &Connection, mut source: Source) -> Result<Source> {
        let mut refs = conn.prepare("SELECT field, credential_key FROM source_secrets WHERE source_id = ?1")?;
        let refs = refs
            .query_map([source.id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if !refs.is_empty() {
            let vault = self.credentials.as_ref().ok_or_else(|| {
                Error::Io(std::io::Error::other("This database requires platform credential storage"))
            })?;
            for (field, key) in refs {
                match field.as_str() {
                    "password" => source.password = Some(vault.get(&key)?),
                    "url" => source.url = vault.get(&key)?,
                    "epg_url" => source.epg_url = Some(vault.get(&key)?),
                    _ => {} // discovered guide URLs are loaded by setting()
                }
            }
        }
        Ok(self.with_runtime_password(source))
    }

    pub(super) fn guide_setting_id(key: &str) -> Option<i64> {
        key.strip_prefix("m3u_epg_url.")?.parse().ok()
    }

    pub(super) fn secret_setting(&self, conn: &Connection, id: i64) -> Result<Option<String>> {
        let key: Option<String> = conn
            .query_row(
                "SELECT credential_key FROM source_secrets WHERE source_id = ?1 AND field = 'discovered_epg'",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        key.map(|key| {
            self.credentials
                .as_ref()
                .ok_or_else(|| Error::Io(std::io::Error::other("This database requires platform credential storage")))?
                .get(&key)
        })
        .transpose()
    }

    /// Retry vault cleanup after a successful edit/deletion and on every launch. Keeping
    /// this queue in SQLite lets a temporarily unavailable vault recover without losing keys.
    pub(super) fn cleanup_credentials(&self, conn: &Connection) -> Result<()> {
        let Some(vault) = &self.credentials else {
            return Ok(());
        };
        let mut stmt = conn.prepare("SELECT credential_key FROM credential_cleanup")?;
        let keys = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for key in keys {
            match vault.delete(&key) {
                Ok(()) => {
                    conn.execute("DELETE FROM credential_cleanup WHERE credential_key = ?1", [key])?;
                }
                Err(e) => log::warn!("credential cleanup will be retried on next launch: {e}"),
            }
        }
        Ok(())
    }

    /// Migrate one source at a time. The original plaintext survives any vault failure,
    /// and a committed source is skipped on retry. Verify writes before clearing SQLite.
    pub(super) fn migrate_credentials(&self) -> Result<()> {
        let Some(vault) = &self.credentials else {
            let secured: bool =
                self.conn().query_row("SELECT EXISTS(SELECT 1 FROM source_secrets)", [], |r| r.get(0))?;
            if secured {
                return Err(Error::Io(std::io::Error::other("This database requires platform credential storage")));
            }
            return Ok(());
        };
        let mut conn = self.conn();
        conn.execute_batch("PRAGMA secure_delete=ON;")?;
        let sources = {
            let mut stmt = conn.prepare(
                "SELECT id, name, kind, url, username, password, epg_url, created_at, catalog_updated_at, epg_updated_at
                 FROM sources",
            )?;
            stmt.query_map([], source_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?
        };
        // The marker is written before any plaintext is removed. A crash before VACUUM
        // leaves it set, so the next launch finishes scrubbing free pages and the WAL.
        let has_plaintext = sources
            .iter()
            .any(|s| s.password.is_some() || s.epg_url.is_some() || (s.kind == SourceKind::M3u && !s.url.is_empty()));
        let has_guide: bool =
            conn.query_row("SELECT EXISTS(SELECT 1 FROM meta WHERE key GLOB 'setting.m3u_epg_url.*')", [], |r| {
                r.get(0)
            })?;
        if has_plaintext || has_guide {
            conn.execute("INSERT OR IGNORE INTO meta VALUES ('credential_scrub_pending', '1')", [])?;
        }
        for source in sources {
            let guide_key = format!("setting.m3u_epg_url.{}", source.id);
            let epg: Option<String> =
                conn.query_row("SELECT value FROM meta WHERE key = ?1", [&guide_key], |r| r.get(0)).optional()?;
            if source.password.is_none()
                && source.epg_url.is_none()
                && (source.kind != SourceKind::M3u || source.url.is_empty())
                && epg.is_none()
            {
                continue;
            }
            let tx = conn.transaction()?;
            let mut writes = CredentialWrites::new(vault.clone());
            if let Some(password) = &source.password {
                writes.write(&tx, source.id, "password", Some(password))?;
            }
            if source.kind == SourceKind::M3u && !source.url.is_empty() {
                writes.write(&tx, source.id, "url", Some(&source.url))?;
            }
            if let Some(epg) = &source.epg_url {
                writes.write(&tx, source.id, "epg_url", Some(epg))?;
            }
            if let Some(epg) = epg {
                writes.write(&tx, source.id, "discovered_epg", Some(&epg))?;
                tx.execute("DELETE FROM meta WHERE key = ?1", [&guide_key])?;
            }
            tx.execute(
                "UPDATE sources SET password = NULL, epg_url = NULL, url = CASE WHEN kind = 'm3u' THEN '' ELSE url END WHERE id = ?1",
                [source.id],
            )?;
            tx.commit()?;
            writes.commit();
        }
        // Older releases left discovered URLs behind when a source was deleted.
        conn.execute(
            "DELETE FROM meta WHERE key GLOB 'setting.m3u_epg_url.*' AND NOT EXISTS
            (SELECT 1 FROM sources WHERE meta.key = 'setting.m3u_epg_url.' || sources.id)",
            [],
        )?;
        // This also removes copies of legacy plaintext from deleted SQLite pages.
        let pending: bool =
            conn.query_row("SELECT EXISTS(SELECT 1 FROM meta WHERE key = 'credential_scrub_pending')", [], |r| {
                r.get(0)
            })?;
        if pending {
            checkpoint(&conn)?;
            conn.execute_batch("VACUUM;")?;
            checkpoint(&conn)?;
            conn.execute("DELETE FROM meta WHERE key = 'credential_scrub_pending'", [])?;
        }
        self.cleanup_credentials(&conn)?;
        drop(conn);
        for source in self.sources()? {
            self.set_runtime_password(source.id, source.password);
        }
        Ok(())
    }
}

fn checkpoint(conn: &Connection) -> Result<()> {
    let busy: i64 = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
    if busy != 0 {
        return Err(Error::Io(std::io::Error::other(
            "Close other FluxTV instances and restart to finish the credential migration",
        )));
    }
    Ok(())
}
