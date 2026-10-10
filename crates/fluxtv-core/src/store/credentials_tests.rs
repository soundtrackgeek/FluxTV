use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::*;

#[derive(Default)]
struct TestVault {
    values: Mutex<HashMap<String, String>>,
    fail_set: AtomicBool,
    fail_delete: AtomicBool,
    bad_read: AtomicBool,
    writes: AtomicUsize,
    fail_after: Mutex<Option<usize>>,
}

impl CredentialStore for TestVault {
    fn get(&self, key: &str) -> Result<String> {
        if self.bad_read.load(Ordering::SeqCst) {
            return Ok("wrong value".into());
        }
        self.values
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or_else(|| Error::Io(std::io::Error::other("missing test credential")))
    }
    fn set(&self, key: &str, value: &str) -> Result<()> {
        if self.fail_set.load(Ordering::SeqCst)
            || self.fail_after.lock().unwrap().is_some_and(|limit| self.writes.load(Ordering::SeqCst) >= limit)
        {
            return Err(Error::Io(std::io::Error::other("test vault unavailable")));
        }
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.values.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        if self.fail_delete.load(Ordering::SeqCst) {
            return Err(Error::Io(std::io::Error::other("test vault unavailable")));
        }
        self.values.lock().unwrap().remove(key);
        Ok(())
    }
}

fn input(kind: SourceKind) -> NewSource {
    NewSource {
        name: "Provider".into(),
        kind,
        url: if kind == SourceKind::M3u {
            "https://example.invalid/list?token=playlist-secret"
        } else {
            "https://example.invalid"
        }
        .into(),
        username: Some("test-user".into()),
        password: (kind == SourceKind::Xtream).then(|| "password-secret-æ🔒".into()),
        epg_url: Some("https://example.invalid/epg?token=explicit-guide-secret".into()),
    }
}

fn assert_no_plaintext(store: &Store, id: i64, m3u: bool) {
    let conn = store.conn();
    let (url, password, epg): (String, Option<String>, Option<String>) = conn
        .query_row("SELECT url, password, epg_url FROM sources WHERE id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    assert_eq!(password, None);
    assert_eq!(epg, None);
    if m3u {
        assert_eq!(url, "");
    }
    let guides: i64 =
        conn.query_row("SELECT count(*) FROM meta WHERE key GLOB 'setting.m3u_epg_url.*'", [], |r| r.get(0)).unwrap();
    assert_eq!(guides, 0);
}

#[test]
fn secure_sources_and_discovered_guides_survive_reopen_and_edits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secure.db");
    let vault = Arc::new(TestVault::default());
    let store = Store::open_with_credentials(&path, vault.clone()).unwrap();
    let xtream = input(SourceKind::Xtream);
    let m3u = input(SourceKind::M3u);
    let xid = store.insert_source(&xtream).unwrap().id;
    let mid = store.insert_source(&m3u).unwrap().id;
    let guide = format!("m3u_epg_url.{mid}");
    store.set_setting(&guide, Some("https://example.invalid/discovered?token=discovered-guide-secret")).unwrap();
    assert_no_plaintext(&store, xid, false);
    assert_no_plaintext(&store, mid, true);
    drop(store);
    let writes = vault.writes.load(Ordering::SeqCst);
    let store = Store::open_with_credentials(&path, vault.clone()).unwrap();
    assert_eq!(vault.writes.load(Ordering::SeqCst), writes, "migration must run only once");
    assert_eq!(store.source(xid).unwrap().password, xtream.password);
    assert_eq!(store.source(mid).unwrap().url, m3u.url);
    assert_eq!(store.source(mid).unwrap().epg_url, m3u.epg_url);
    assert!(store.setting(&guide).unwrap().unwrap().contains("discovered-guide-secret"));
    assert_eq!(store.sources().unwrap().len(), 2);
    let mut edit = xtream;
    edit.name = "Renamed".into();
    edit.password = None;
    assert_eq!(store.update_source(xid, &edit).unwrap().password.as_deref(), Some("password-secret-æ🔒"));
    edit.password = Some("replacement-password".into());
    store.update_source(xid, &edit).unwrap();
    assert_eq!(store.source(xid).unwrap().password, edit.password);
    let mut edit = m3u;
    edit.name = "Renamed playlist".into();
    edit.epg_url = None;
    store.update_source(mid, &edit).unwrap();
    assert!(store.source(mid).unwrap().epg_url.is_none());
    assert!(store.setting(&guide).unwrap().is_some(), "renaming must retain the discovered guide");
    store.set_setting(&guide, Some("https://example.invalid/new-guide?token=new-secret")).unwrap();
    assert_eq!(store.setting(&guide).unwrap().as_deref(), Some("https://example.invalid/new-guide?token=new-secret"));
    edit.url = "https://example.invalid/other?token=other-secret".into();
    store.update_source(mid, &edit).unwrap();
    assert!(store.setting(&guide).unwrap().is_none(), "changing playlist must clear its discovered guide");
    store.delete_source(xid).unwrap();
    store.delete_source(mid).unwrap();
    assert!(vault.values.lock().unwrap().is_empty());
    let reused = store.insert_source(&input(SourceKind::Xtream)).unwrap();
    assert_eq!(reused.password.as_deref(), Some("password-secret-æ🔒"));
    store.delete_source(reused.id).unwrap();
    assert!(Store::open(&path).is_ok());
}

#[test]
fn legacy_migration_scrubs_database_and_wal_and_keeps_catalog() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    let old = Store::open(&path).unwrap();
    let xid = old.insert_source(&input(SourceKind::Xtream)).unwrap().id;
    let mid = old.insert_source(&input(SourceKind::M3u)).unwrap().id;
    old.set_setting(
        &format!("m3u_epg_url.{mid}"),
        Some("https://example.invalid/discovered?token=discovered-guide-secret"),
    )
    .unwrap();
    old.set_setting("m3u_epg_url.999", Some("https://example.invalid/deleted?token=orphan-secret")).unwrap();
    old.set_favorite(xid, ContentKind::Live, "123", true).unwrap();
    old.set_setting("external_player", Some("vlc.exe")).unwrap();
    drop(old);
    let vault = Arc::new(TestVault::default());
    let store = Store::open_with_credentials(&path, vault.clone()).unwrap();
    assert_no_plaintext(&store, xid, false);
    assert_no_plaintext(&store, mid, true);
    assert_eq!(store.source(xid).unwrap().password, input(SourceKind::Xtream).password);
    assert_eq!(store.source(mid).unwrap().url, input(SourceKind::M3u).url);
    assert_eq!(store.favorite_ids(xid, ContentKind::Live).unwrap(), vec!["123"]);
    assert_eq!(store.setting("external_player").unwrap().as_deref(), Some("vlc.exe"));
    assert!(Store::open(&path).is_err(), "secured databases must never open without a vault");
    drop(store);
    for file in std::fs::read_dir(dir.path()).unwrap() {
        let bytes = std::fs::read(file.unwrap().path()).unwrap();
        for secret in
            ["password-secret", "playlist-secret", "explicit-guide-secret", "discovered-guide-secret", "orphan-secret"]
        {
            assert!(
                !bytes.windows(secret.len()).any(|window| window == secret.as_bytes()),
                "plaintext remains: {secret}"
            );
        }
    }
}

#[test]
fn vault_failure_preserves_legacy_plaintext_for_retry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    let old = Store::open(&path).unwrap();
    let id = old.insert_source(&input(SourceKind::M3u)).unwrap().id;
    drop(old);
    let vault = Arc::new(TestVault::default());
    vault.fail_set.store(true, Ordering::SeqCst);
    assert!(Store::open_with_credentials(&path, vault.clone()).is_err());
    let old = Store::open(&path).unwrap();
    assert_eq!(old.source(id).unwrap().url, input(SourceKind::M3u).url);
    assert_eq!(old.source(id).unwrap().epg_url, input(SourceKind::M3u).epg_url);
    drop(old);
    vault.fail_set.store(false, Ordering::SeqCst);
    let migrated = Store::open_with_credentials(&path, vault.clone()).unwrap();
    assert_no_plaintext(&migrated, id, true);
}

#[test]
fn failed_add_edit_and_verification_do_not_change_working_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Arc::new(TestVault::default());
    let store = Store::open_with_credentials(dir.path().join("secure.db"), vault.clone()).unwrap();
    let original = input(SourceKind::Xtream);
    let id = store.insert_source(&original).unwrap().id;
    let count = vault.values.lock().unwrap().len();
    let mut edit = original.clone();
    edit.password = Some("replacement-password".into());
    vault.fail_set.store(true, Ordering::SeqCst);
    assert!(store.update_source(id, &edit).is_err());
    assert!(store.insert_source(&input(SourceKind::M3u)).is_err());
    vault.fail_set.store(false, Ordering::SeqCst);
    assert_eq!(store.sources().unwrap().len(), 1);
    assert_eq!(store.source(id).unwrap().password, original.password);
    vault.bad_read.store(true, Ordering::SeqCst);
    assert!(store.insert_source(&input(SourceKind::M3u)).is_err());
    vault.bad_read.store(false, Ordering::SeqCst);
    assert_eq!(vault.values.lock().unwrap().len(), count);
    store.conn().execute_batch("CREATE TRIGGER fail_reference BEFORE INSERT ON source_secrets BEGIN SELECT RAISE(ABORT, 'test failure'); END;").unwrap();
    assert!(store.update_source(id, &edit).is_err());
    assert_eq!(store.source(id).unwrap().password, original.password);
    assert_eq!(vault.values.lock().unwrap().len(), count);
    let key =
        vault.values.lock().unwrap().iter().find(|(_, v)| v.as_str() == "password-secret-æ🔒").map(|(k, _)| k.clone());
    // Read failures must surface rather than quietly falling back to a cached password.
    if let Some(key) = key {
        vault.values.lock().unwrap().remove(&key);
    }
    assert!(store.source(id).is_err());
}

#[test]
fn deletion_cleanup_retries_on_next_launch_and_ids_are_isolated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secure.db");
    let vault = Arc::new(TestVault::default());
    let store = Store::open_with_credentials(&path, vault.clone()).unwrap();
    let id = store.insert_source(&input(SourceKind::Xtream)).unwrap().id;
    vault.fail_delete.store(true, Ordering::SeqCst);
    store.delete_source(id).unwrap();
    assert!(store.sources().unwrap().is_empty());
    assert!(!vault.values.lock().unwrap().is_empty());
    assert!(store.set_setting(&format!("m3u_epg_url.{id}"), Some("https://example.invalid/guide")).is_err());
    let next = store.insert_source(&input(SourceKind::M3u)).unwrap();
    assert_eq!(next.id, id, "exercise SQLite source id reuse");
    assert_eq!(next.password, None);
    let other = Store::open_with_credentials(dir.path().join("other.db"), vault.clone()).unwrap();
    let mut other_input = input(SourceKind::Xtream);
    other_input.password = Some("other-password".into());
    let other_id = other.insert_source(&other_input).unwrap().id;
    assert_eq!(other_id, id);
    assert_eq!(other.source(other_id).unwrap().password, other_input.password);
    drop(store);
    vault.fail_delete.store(false, Ordering::SeqCst);
    let store = Store::open_with_credentials(&path, vault.clone()).unwrap();
    assert_eq!(store.source(next.id).unwrap().url, input(SourceKind::M3u).url);
    assert_eq!(
        store.conn().query_row("SELECT count(*) FROM credential_cleanup", [], |r| r.get::<_, i64>(0)).unwrap(),
        0
    );
    store.delete_source(next.id).unwrap();
    other.delete_source(other_id).unwrap();
    // One intentionally failed rollback cleanup was unreferenced; production logs this failure.
}

#[test]
fn interrupted_migration_resumes_without_overwriting_committed_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("partial.db");
    let old = Store::open(&path).unwrap();
    let xid = old.insert_source(&input(SourceKind::Xtream)).unwrap().id;
    let mid = old.insert_source(&input(SourceKind::M3u)).unwrap().id;
    drop(old);
    let vault = Arc::new(TestVault::default());
    *vault.fail_after.lock().unwrap() = Some(2); // first source saved, second source fails
    assert!(Store::open_with_credentials(&path, vault.clone()).is_err());
    let conn = Connection::open(&path).unwrap();
    let migrated: Option<String> =
        conn.query_row("SELECT password FROM sources WHERE id = ?1", [xid], |r| r.get(0)).unwrap();
    assert_eq!(migrated, None);
    let remaining: String = conn.query_row("SELECT url FROM sources WHERE id = ?1", [mid], |r| r.get(0)).unwrap();
    assert_eq!(remaining, input(SourceKind::M3u).url);
    drop(conn);
    *vault.fail_after.lock().unwrap() = None;
    let resumed = Store::open_with_credentials(&path, vault.clone()).unwrap();
    assert_eq!(vault.writes.load(Ordering::SeqCst), 4, "already migrated secrets must not be rewritten");
    assert_eq!(resumed.source(xid).unwrap().password, input(SourceKind::Xtream).password);
    assert_eq!(resumed.source(mid).unwrap().url, input(SourceKind::M3u).url);
    assert_no_plaintext(&resumed, xid, false);
    assert_no_plaintext(&resumed, mid, true);
}
