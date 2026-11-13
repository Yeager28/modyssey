//! On-disk storage for the saved Steam login.
//!
//! What is kept is Steam's *refresh token* (a long-lived JWT Steam issues
//! after a successful login), never the password. With it a later run can log
//! in without any prompt, until Steam expires or the user revokes it. Steam
//! Guard "remember this computer" data is kept too, so that a fresh password
//! login after expiry does not ask for a code again.
//!
//! Both live in files readable only by the current user, written atomically.

use crate::secrets::{warn_unavailable_once, Vault};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const LOGIN_FILE: &str = "steam_login.json";
const GUARD_FILE: &str = "steam_guard.json";
/// Keyring entry names.
const TOKEN_KEY: &str = "steam_refresh_token";
const GUARD_KEY_PREFIX: &str = "steam_guard:";
const FORMAT_VERSION: u32 = 1;
/// Treat a token this close to expiry as already expired, so a long download
/// does not start on a login that dies halfway through.
const EXPIRY_MARGIN_SECS: u64 = 24 * 60 * 60;

/// A saved Steam login.
#[derive(Clone, Serialize, Deserialize)]
pub struct SavedLogin {
    pub version: u32,
    pub account: String,
    pub steam_id: u64,
    /// Empty in the file when the token is in the keyring.
    #[serde(default)]
    pub refresh_token: String,
    /// Unix time the refresh token expires, read from the token itself.
    pub expires_at: Option<u64>,
    pub saved_at: u64,
}

impl fmt::Debug for SavedLogin {
    // The token is a bearer credential; keep it out of logs and panics.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SavedLogin")
            .field("account", &self.account)
            .field("steam_id", &self.steam_id)
            .field("refresh_token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl SavedLogin {
    pub fn new(account: &str, steam_id: u64, refresh_token: &str) -> Self {
        Self {
            version: FORMAT_VERSION,
            account: account.to_string(),
            steam_id,
            refresh_token: refresh_token.to_string(),
            expires_at: jwt_expiry(refresh_token),
            saved_at: now_secs(),
        }
    }

    /// False once the token has expired or is about to. A token with no
    /// readable expiry is left for Steam to judge.
    pub fn is_usable_at(&self, now: u64) -> bool {
        self.expires_at
            .map(|exp| exp > now.saturating_add(EXPIRY_MARGIN_SECS))
            .unwrap_or(true)
    }

    /// Whole days until expiry, or `None` when unknown. Zero once expired.
    pub fn days_left_at(&self, now: u64) -> Option<u64> {
        self.expires_at.map(|exp| exp.saturating_sub(now) / 86_400)
    }
}

/// The `exp` claim of a JWT, without verifying it (Steam does that).
pub fn jwt_expiry(token: &str) -> Option<u64> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()?
        .get("exp")?
        .as_u64()
}

/// Reads and writes the saved login and Steam Guard data in one directory.
#[derive(Debug, Clone)]
pub struct LoginStore {
    dir: PathBuf,
    vault: Vault,
}

impl LoginStore {
    /// Stores secrets in the system keyring, falling back to owner-only files
    /// when none is available.
    pub fn new(dir: PathBuf) -> Self {
        Self::with_vault(dir, Vault::system())
    }

    pub fn with_vault(dir: PathBuf, vault: Vault) -> Self {
        Self { dir, vault }
    }

    /// Modyssey's usual config directory (`~/.config/modyssey`).
    pub fn default_location() -> Result<Self> {
        Ok(Self::new(crate::settings::Settings::config_dir()?))
    }

    pub fn login_path(&self) -> PathBuf {
        self.dir.join(LOGIN_FILE)
    }

    pub fn guard_path(&self) -> PathBuf {
        self.dir.join(GUARD_FILE)
    }

    fn guard_key(account: &str) -> String {
        format!("{GUARD_KEY_PREFIX}{}", account.to_ascii_lowercase())
    }

    /// The saved login, or `None` if there is none. A file that cannot be
    /// parsed is an error rather than silently ignored, so the user is told.
    ///
    /// The refresh token lives in the keyring; the file holds only who it
    /// belongs to and when it expires. A token found in the file (saved while
    /// no keyring was available, or by an older build) is moved to the keyring.
    pub fn load_login(&self) -> Result<Option<SavedLogin>> {
        let path = self.login_path();
        let mut login: SavedLogin = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("{} is not a valid saved login", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("Reading {}", path.display())),
        };
        if login.version != FORMAT_VERSION {
            bail!(
                "{} has unsupported version {}",
                path.display(),
                login.version
            );
        }

        if login.refresh_token.is_empty() {
            return match self.vault.get(TOKEN_KEY) {
                Ok(Some(token)) if !token.is_empty() => {
                    login.refresh_token = token;
                    Ok(Some(login))
                }
                // The keyring was reset or the entry removed: nothing usable.
                Ok(_) => Ok(None),
                Err(e) => Err(anyhow::Error::from(e).context(
                    "The saved Steam login is kept in the system keyring, which is not available right now",
                )),
            };
        }

        // Token in the file: try to move it into the keyring.
        match self.vault.set_verified(TOKEN_KEY, &login.refresh_token) {
            Ok(()) => {
                if let Err(e) = self.write_login_file(&login, false) {
                    tracing::warn!(
                        "Could not remove the Steam token from {}: {e:#}",
                        path.display()
                    );
                }
            }
            Err(e) => warn_unavailable_once(&e),
        }
        Ok(Some(login))
    }

    pub fn save_login(&self, login: &SavedLogin) -> Result<()> {
        match self.vault.set_verified(TOKEN_KEY, &login.refresh_token) {
            Ok(()) => self.write_login_file(login, false),
            Err(e) => {
                // No keyring: keep the owner-only file rather than lose the login.
                warn_unavailable_once(&e);
                self.write_login_file(login, true)
            }
        }
    }

    fn write_login_file(&self, login: &SavedLogin, include_token: bool) -> Result<()> {
        let mut on_disk = login.clone();
        if !include_token {
            on_disk.refresh_token.clear();
        }
        write_private(&self.login_path(), &serde_json::to_vec_pretty(&on_disk)?)
    }

    /// Forget the login everywhere. Returns whether there was one to forget.
    pub fn clear_login(&self) -> Result<bool> {
        let in_keyring = match self.vault.delete(TOKEN_KEY) {
            Ok(removed) => removed,
            Err(e) => {
                warn_unavailable_once(&e);
                false
            }
        };
        let path = self.login_path();
        let had_file = match std::fs::remove_file(&path) {
            Ok(()) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(e).with_context(|| format!("Removing {}", path.display())),
        };
        Ok(in_keyring || had_file)
    }

    /// Also forget remembered-computer data for `account`, so the next login
    /// asks for a Steam Guard code again. Used by `steam logout --forget-device`.
    pub fn clear_guard(&self, account: Option<&str>) -> Result<()> {
        if let Some(account) = account {
            if let Err(e) = self.vault.delete(&Self::guard_key(account)) {
                warn_unavailable_once(&e);
            }
        }
        match std::fs::remove_file(self.guard_path()) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).context("Removing Steam Guard data"),
        }
    }

    fn guard_map(&self) -> Result<HashMap<String, String>> {
        match std::fs::read_to_string(self.guard_path()) {
            Ok(text) => serde_json::from_str(&text).context("Steam Guard data is not valid JSON"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
            Err(e) => Err(e).context("Reading Steam Guard data"),
        }
    }
}

/// Error type for [`steam_vent::auth::GuardDataStore`], which wants a
/// `std::error::Error` (an `anyhow::Error` is not one).
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct GuardStoreError(String);

impl From<anyhow::Error> for GuardStoreError {
    fn from(e: anyhow::Error) -> Self {
        Self(format!("{e:#}"))
    }
}

impl steam_vent::auth::GuardDataStore for LoginStore {
    type Err = GuardStoreError;

    async fn store(&mut self, account: &str, machine_token: String) -> Result<(), Self::Err> {
        match self
            .vault
            .set_verified(&Self::guard_key(account), &machine_token)
        {
            Ok(()) => Ok(()),
            Err(e) => {
                warn_unavailable_once(&e);
                // A damaged file is replaced rather than blocking the login.
                let mut map = self.guard_map().unwrap_or_default();
                map.insert(account.to_ascii_lowercase(), machine_token);
                let bytes = serde_json::to_vec_pretty(&map).map_err(anyhow::Error::from)?;
                Ok(write_private(&self.guard_path(), &bytes)?)
            }
        }
    }

    async fn load(&mut self, account: &str) -> Result<Option<String>, Self::Err> {
        if let Ok(Some(token)) = self.vault.get(&Self::guard_key(account)) {
            return Ok(Some(token));
        }
        Ok(self.guard_map()?.remove(&account.to_ascii_lowercase()))
    }
}

/// Write `bytes` to `path` so only the current user can read it, replacing any
/// existing file atomically. The mode is set at creation, so the secret is
/// never on disk with looser permissions.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("Path has no parent directory")?;
    std::fs::create_dir_all(dir).with_context(|| format!("Creating {}", dir.display()))?;
    let tmp = path.with_extension("json.tmp");

    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&tmp)
        .with_context(|| format!("Opening {}", tmp.display()))?;
    #[cfg(unix)]
    {
        // A leftover temp file from an older run may exist with other modes.
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .with_context(|| format!("Writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("Replacing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::SecretBackend;
    use steam_vent::auth::GuardDataStore;

    fn jwt(payload: &str) -> String {
        format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(payload.as_bytes()))
    }

    #[test]
    fn reads_jwt_expiry_and_survives_garbage() {
        assert_eq!(
            jwt_expiry(&jwt(r#"{"exp":1900000000,"sub":"1"}"#)),
            Some(1_900_000_000)
        );
        assert_eq!(jwt_expiry(&jwt(r#"{"sub":"1"}"#)), None);
        assert_eq!(jwt_expiry(&jwt("not json")), None);
        assert_eq!(jwt_expiry("onlyonepart"), None);
        assert_eq!(jwt_expiry("a.!!!.c"), None);
        assert_eq!(jwt_expiry(""), None);
    }

    #[test]
    fn usability_respects_the_expiry_margin() {
        let now = 1_000_000;
        let mut login = SavedLogin::new("a", 1, "x");
        login.expires_at = Some(now + EXPIRY_MARGIN_SECS + 1);
        assert!(login.is_usable_at(now));
        login.expires_at = Some(now + EXPIRY_MARGIN_SECS);
        assert!(!login.is_usable_at(now));
        login.expires_at = Some(now - 5);
        assert!(!login.is_usable_at(now));
        assert_eq!(login.days_left_at(now), Some(0));
        login.expires_at = None;
        assert!(login.is_usable_at(now));
        login.expires_at = Some(now + 3 * 86_400 + 10);
        assert_eq!(login.days_left_at(now), Some(3));
    }

    #[test]
    fn debug_output_never_contains_the_token() {
        let login = SavedLogin::new("alice", 7, "SECRET-TOKEN-VALUE");
        let text = format!("{login:?}");
        assert!(!text.contains("SECRET-TOKEN-VALUE"), "{text}");
        assert!(text.contains("alice"));
    }

    fn memory_store(
        dir: &std::path::Path,
    ) -> (LoginStore, std::sync::Arc<crate::secrets::MemoryBackend>) {
        let (vault, backend) = Vault::memory();
        (LoginStore::with_vault(dir.to_path_buf(), vault), backend)
    }

    #[test]
    fn token_goes_to_the_keyring_not_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _backend) = memory_store(&dir.path().join("cfg"));
        assert!(store.load_login().unwrap().is_none());
        assert!(!store.clear_login().unwrap());

        store
            .save_login(&SavedLogin::new("alice", 76561198000000000, "SECRET-TOKEN"))
            .unwrap();

        let text = std::fs::read_to_string(store.login_path()).unwrap();
        assert!(!text.contains("SECRET-TOKEN"), "{text}");
        assert!(text.contains("alice"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(store.login_path())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        let loaded = store.load_login().unwrap().unwrap();
        assert_eq!(
            (loaded.account.as_str(), loaded.steam_id),
            ("alice", 76561198000000000)
        );
        assert_eq!(loaded.refresh_token, "SECRET-TOKEN");

        assert!(store.clear_login().unwrap());
        assert!(store.load_login().unwrap().is_none());
        assert!(!store.login_path().exists());
    }

    #[test]
    fn without_a_keyring_the_token_stays_in_the_private_file_then_migrates() {
        let dir = tempfile::tempdir().unwrap();
        let (store, backend) = memory_store(dir.path());
        backend.set_unavailable(true);
        store
            .save_login(&SavedLogin::new("alice", 1, "FALLBACK-TOKEN"))
            .unwrap();
        assert!(std::fs::read_to_string(store.login_path())
            .unwrap()
            .contains("FALLBACK-TOKEN"));
        assert_eq!(
            store.load_login().unwrap().unwrap().refresh_token,
            "FALLBACK-TOKEN"
        );

        // The keyring appears: the next load moves the token and scrubs the file.
        backend.set_unavailable(false);
        assert_eq!(
            store.load_login().unwrap().unwrap().refresh_token,
            "FALLBACK-TOKEN"
        );
        assert!(!std::fs::read_to_string(store.login_path())
            .unwrap()
            .contains("FALLBACK-TOKEN"));
        assert_eq!(
            backend.get(TOKEN_KEY).unwrap().as_deref(),
            Some("FALLBACK-TOKEN")
        );
    }

    #[test]
    fn keyring_outage_after_saving_is_reported_not_treated_as_logged_out() {
        let dir = tempfile::tempdir().unwrap();
        let (store, backend) = memory_store(dir.path());
        store
            .save_login(&SavedLogin::new("alice", 1, "tok"))
            .unwrap();
        backend.set_unavailable(true);
        let err = store.load_login().unwrap_err();
        assert!(format!("{err:#}").contains("keyring"), "{err:#}");
        // Nothing was deleted by the failed read.
        backend.set_unavailable(false);
        assert_eq!(store.load_login().unwrap().unwrap().refresh_token, "tok");
    }

    #[test]
    fn a_wiped_keyring_means_logged_out() {
        let dir = tempfile::tempdir().unwrap();
        let (store, backend) = memory_store(dir.path());
        store
            .save_login(&SavedLogin::new("alice", 1, "tok"))
            .unwrap();
        backend.delete(TOKEN_KEY).unwrap();
        assert!(store.load_login().unwrap().is_none());
    }

    #[test]
    fn corrupt_login_file_is_an_error_not_silence() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _b) = memory_store(dir.path());
        std::fs::write(store.login_path(), "{ nope").unwrap();
        assert!(store.load_login().is_err());
        std::fs::write(
            store.login_path(),
            r#"{"version":99,"account":"a","steam_id":1,"refresh_token":"t","expires_at":null,"saved_at":0}"#,
        )
        .unwrap();
        assert!(store
            .load_login()
            .unwrap_err()
            .to_string()
            .contains("unsupported"));
    }

    #[tokio::test]
    async fn guard_data_is_per_account_case_insensitive_and_in_the_keyring() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, backend) = memory_store(dir.path());
        assert_eq!(store.load("Alice").await.unwrap(), None);
        store.store("Alice", "machine-a".into()).await.unwrap();
        store.store("bob", "machine-b".into()).await.unwrap();
        assert_eq!(
            store.load("alice").await.unwrap().as_deref(),
            Some("machine-a")
        );
        assert_eq!(
            store.load("BOB").await.unwrap().as_deref(),
            Some("machine-b")
        );
        assert!(
            !store.guard_path().exists(),
            "guard data must not hit the disk when a keyring works"
        );

        store.clear_guard(Some("alice")).unwrap();
        assert_eq!(store.load("alice").await.unwrap(), None);
        assert_eq!(
            backend.get("steam_guard:bob").unwrap().as_deref(),
            Some("machine-b")
        );
    }

    #[tokio::test]
    async fn guard_data_falls_back_to_a_private_file_without_a_keyring() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, backend) = memory_store(dir.path());
        backend.set_unavailable(true);
        store.store("Alice", "machine-a".into()).await.unwrap();
        assert_eq!(
            store.load("alice").await.unwrap().as_deref(),
            Some("machine-a")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(store.guard_path())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        store.clear_guard(Some("alice")).unwrap();
        assert_eq!(store.load("alice").await.unwrap(), None);
    }
}
