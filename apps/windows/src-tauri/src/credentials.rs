//! Windows Credential Manager adapter. Never fall back to plaintext or keyring's mock store.

use std::sync::Arc;

use fluxtv_core::{Result, store::CredentialStore};

fn storage_error(operation: &str) -> fluxtv_core::Error {
    fluxtv_core::Error::Io(std::io::Error::other(format!(
        "Windows Credential Manager could not {operation} the credential. Check that it is available for this Windows user and try again."
    )))
}

#[cfg(windows)]
const SERVICE: &str = "com.fluxtv.app";

#[cfg(windows)]
struct WindowsCredentials;

#[cfg(windows)]
impl WindowsCredentials {
    fn entry(key: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, key).map_err(|_| storage_error("open"))
    }
}

#[cfg(windows)]
impl CredentialStore for WindowsCredentials {
    fn get(&self, key: &str) -> Result<String> {
        let bytes = Self::entry(key)?.get_secret().map_err(|_| storage_error("read"))?;
        String::from_utf8(bytes).map_err(|_| storage_error("decode"))
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        Self::entry(key)?.set_secret(value.as_bytes()).map_err(|_| storage_error("save"))
    }

    fn delete(&self, key: &str) -> Result<()> {
        match Self::entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(storage_error("delete")),
        }
    }
}

pub fn platform_store() -> Result<Arc<dyn CredentialStore>> {
    #[cfg(windows)]
    {
        Ok(Arc::new(WindowsCredentials))
    }
    #[cfg(not(windows))]
    {
        Err(storage_error("access on this platform"))
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use fluxtv_core::{NewSource, SourceKind, Store};

    /// Uses only synthetic credentials in an isolated temporary database. This exercises
    /// the real native vault, including store reopen and deletion, without an IPTV account.
    #[test]
    fn native_credential_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("native.db");
        let vault = platform_store().unwrap();
        let store = Store::open_with_credentials(&path, vault.clone()).unwrap();
        let input = NewSource {
            name: "Credential test".into(),
            kind: SourceKind::M3u,
            url: "https://example.invalid/playlist?token=synthetic-test-token".into(),
            username: None,
            password: None,
            epg_url: Some("https://example.invalid/guide?token=synthetic-guide-token".into()),
        };
        let id = store.insert_source(&input).unwrap().id;
        store.set_setting(&format!("m3u_epg_url.{id}"), Some("https://example.invalid/discovered?token=test")).unwrap();
        drop(store);
        let store = Store::open_with_credentials(&path, vault.clone()).unwrap();
        assert_eq!(store.source(id).unwrap().url, input.url);
        assert_eq!(store.source(id).unwrap().epg_url, input.epg_url);
        assert!(store.setting(&format!("m3u_epg_url.{id}")).unwrap().is_some());
        let mut input = input;
        input.kind = SourceKind::Xtream;
        input.url = "https://example.invalid".into();
        input.username = Some("test-user".into());
        input.password = Some("synthetic-password-æ🔒".into());
        store.update_source(id, &input).unwrap();
        drop(store);
        let store = Store::open_with_credentials(&path, vault.clone()).unwrap();
        assert_eq!(store.source(id).unwrap().password, input.password);
        let conn = rusqlite::Connection::open(&path).unwrap();
        let keys: Vec<String> = conn
            .prepare("SELECT credential_key FROM source_secrets")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let password: Option<String> = conn.query_row("SELECT password FROM sources", [], |r| r.get(0)).unwrap();
        assert!(password.is_none());
        store.delete_source(id).unwrap();
        for key in keys {
            assert!(vault.get(&key).is_err());
        }
    }

    #[test]
    fn native_migrates_legacy_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.db");
        let input = NewSource {
            name: "Legacy credential test".into(),
            kind: SourceKind::Xtream,
            url: "https://example.invalid".into(),
            username: Some("synthetic-user".into()),
            password: Some("synthetic-legacy-password".into()),
            epg_url: Some("https://example.invalid/epg?token=synthetic-legacy-token".into()),
        };
        let old = Store::open(&path).unwrap();
        let id = old.insert_source(&input).unwrap().id;
        drop(old);
        let vault = platform_store().unwrap();
        let migrated = Store::open_with_credentials(&path, vault.clone()).unwrap();
        assert_eq!(migrated.source(id).unwrap().password, input.password);
        assert_eq!(migrated.source(id).unwrap().epg_url, input.epg_url);
        let conn = rusqlite::Connection::open(&path).unwrap();
        let (password, epg): (Option<String>, Option<String>) = conn
            .query_row("SELECT password, epg_url FROM sources WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!((password, epg), (None, None));
        let keys: Vec<String> = conn
            .prepare("SELECT credential_key FROM source_secrets")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        migrated.delete_source(id).unwrap();
        for key in keys {
            assert!(vault.get(&key).is_err());
        }
    }
}
