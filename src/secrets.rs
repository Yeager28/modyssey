//! Secret storage in the operating system's keyring.
//!
//! On Linux this is the freedesktop Secret Service (KWallet and GNOME Keyring
//! both provide it). Secrets are only ever *moved* out of a plaintext file
//! after they have been written to the keyring and read back identically, so
//! an unavailable or broken keyring can never lose a credential.
//!
//! Every keyring call runs on a helper thread with a timeout. That keeps a
//! locked wallet, a hung D-Bus service or a missing session bus from freezing
//! the app, and keeps the blocking keyring client out of the async runtime.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Duration;

/// Keyring "service" CLF3's entries are filed under. Override with
/// `DS2MODPACK_KEYRING_SERVICE` (used by tests so they never touch real entries).
pub const DEFAULT_SERVICE: &str = "ds2modpack";
const SERVICE_ENV: &str = "DS2MODPACK_KEYRING_SERVICE";

/// Long enough for a user to answer an "unlock your wallet" prompt.
const OPERATION_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    /// No usable keyring: no Secret Service on the bus, it is locked and the
    /// prompt was dismissed, or the call timed out.
    #[error("the system keyring is unavailable: {0}")]
    Unavailable(String),
    /// The keyring answered but the operation failed.
    #[error("keyring error: {0}")]
    Failed(String),
    /// There is no entry under that key. Backends turn this into `Ok(None)` /
    /// `Ok(false)`; it never reaches callers.
    #[error("no such keyring entry")]
    NotFound,
}

impl SecretError {
    pub fn is_unavailable(&self) -> bool {
        matches!(self, SecretError::Unavailable(_))
    }
}

pub trait SecretBackend: Send + Sync {
    /// `Ok(None)` when there is no such entry.
    fn get(&self, key: &str) -> Result<Option<String>, SecretError>;
    fn set(&self, key: &str, value: &str) -> Result<(), SecretError>;
    /// Whether an entry was removed.
    fn delete(&self, key: &str) -> Result<bool, SecretError>;
}

/// The real keyring.
pub struct KeyringBackend {
    service: String,
}

type SharedStore = Arc<zbus_secret_service_keyring_store::Store>;

impl KeyringBackend {
    pub fn new(service: &str) -> Self {
        Self {
            service: service.to_string(),
        }
    }

    /// One store per process; only a successful connection is remembered, so a
    /// keyring that starts later can still be picked up.
    fn store() -> Result<SharedStore, SecretError> {
        static STORE: OnceLock<SharedStore> = OnceLock::new();
        if let Some(store) = STORE.get() {
            return Ok(store.clone());
        }
        let store = zbus_secret_service_keyring_store::Store::new().map_err(map_error)?;
        Ok(STORE.get_or_init(|| store).clone())
    }

    fn run<T: Send + 'static>(
        &self,
        key: &str,
        op: impl FnOnce(keyring_core::Entry) -> keyring_core::Result<T> + Send + 'static,
    ) -> Result<T, SecretError> {
        use keyring_core::api::CredentialStoreApi;
        let service = self.service.clone();
        let key = key.to_string();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| {
                let store = KeyringBackend::store()?;
                let entry = store.build(&service, &key, None).map_err(map_error)?;
                op(entry).map_err(map_error)
            })();
            let _ = tx.send(result);
        });
        match rx.recv_timeout(OPERATION_TIMEOUT) {
            Ok(result) => result,
            Err(_) => Err(SecretError::Unavailable(
                "the keyring did not answer in time".into(),
            )),
        }
    }
}

fn map_error(error: keyring_core::Error) -> SecretError {
    use keyring_core::Error as E;
    match error {
        E::NoStorageAccess(e) | E::PlatformFailure(e) => SecretError::Unavailable(e.to_string()),
        E::NoDefaultStore => SecretError::Unavailable("no keyring store".into()),
        E::NoEntry => SecretError::NotFound,
        other => SecretError::Failed(other.to_string()),
    }
}

impl SecretBackend for KeyringBackend {
    fn get(&self, key: &str) -> Result<Option<String>, SecretError> {
        match self.run(key, |entry| entry.get_password()) {
            Ok(value) => Ok(Some(value)),
            Err(SecretError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<(), SecretError> {
        let value = value.to_string();
        self.run(key, move |entry| entry.set_password(&value))
    }

    fn delete(&self, key: &str) -> Result<bool, SecretError> {
        match self.run(key, |entry| entry.delete_credential()) {
            Ok(()) => Ok(true),
            Err(SecretError::NotFound) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

/// In-memory backend for tests; can be switched to "unavailable".
#[derive(Default)]
pub struct MemoryBackend {
    map: Mutex<HashMap<String, String>>,
    unavailable: AtomicBool,
}

impl MemoryBackend {
    pub fn set_unavailable(&self, unavailable: bool) {
        self.unavailable.store(unavailable, Ordering::SeqCst);
    }

    fn check(&self) -> Result<(), SecretError> {
        if self.unavailable.load(Ordering::SeqCst) {
            Err(SecretError::Unavailable("test keyring is offline".into()))
        } else {
            Ok(())
        }
    }
}

impl SecretBackend for MemoryBackend {
    fn get(&self, key: &str) -> Result<Option<String>, SecretError> {
        self.check()?;
        Ok(self.map.lock().unwrap().get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> Result<(), SecretError> {
        self.check()?;
        self.map.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<bool, SecretError> {
        self.check()?;
        Ok(self.map.lock().unwrap().remove(key).is_some())
    }
}

/// A backend plus a record of which values are already known to be stored, so
/// repeated saves of unchanged settings do not hit the keyring every time.
#[derive(Clone)]
pub struct Vault {
    backend: Arc<dyn SecretBackend>,
    synced: Arc<Mutex<HashMap<String, u64>>>,
}

impl std::fmt::Debug for Vault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Vault")
    }
}

fn digest(value: &str) -> u64 {
    let mut h = DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}

impl Vault {
    pub fn new(backend: Arc<dyn SecretBackend>) -> Self {
        Self {
            backend,
            synced: Arc::default(),
        }
    }

    /// The process-wide system keyring vault.
    pub fn system() -> Self {
        static SYSTEM: OnceLock<Vault> = OnceLock::new();
        SYSTEM
            .get_or_init(|| {
                let service = std::env::var(SERVICE_ENV)
                    .ok()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| DEFAULT_SERVICE.to_string());
                Vault::new(Arc::new(KeyringBackend::new(&service)))
            })
            .clone()
    }

    pub fn memory() -> (Self, Arc<MemoryBackend>) {
        let backend = Arc::new(MemoryBackend::default());
        (Self::new(backend.clone()), backend)
    }

    pub fn get(&self, key: &str) -> Result<Option<String>, SecretError> {
        self.backend.get(key)
    }

    pub fn delete(&self, key: &str) -> Result<bool, SecretError> {
        self.synced.lock().unwrap().remove(key);
        self.backend.delete(key)
    }

    /// Store `value`, then read it back and require it to match. Only then is
    /// it safe to remove any other copy. Skips the round trip when this vault
    /// already verified exactly this value for `key`.
    pub fn set_verified(&self, key: &str, value: &str) -> Result<(), SecretError> {
        let d = digest(value);
        if self.synced.lock().unwrap().get(key) == Some(&d) {
            return Ok(());
        }
        self.backend.set(key, value)?;
        match self.backend.get(key)? {
            Some(stored) if stored == value => {
                self.synced.lock().unwrap().insert(key.to_string(), d);
                Ok(())
            }
            _ => Err(SecretError::Failed(
                "the keyring did not return the value just stored".into(),
            )),
        }
    }
}

/// Log a keyring problem once per process, however many saves hit it.
pub fn warn_unavailable_once(error: &SecretError) {
    static WARNED: AtomicBool = AtomicBool::new(false);
    if !WARNED.swap(true, Ordering::SeqCst) {
        tracing::warn!(
            "{error}. Credentials stay in the owner-only settings file instead; \
             they will move to the keyring automatically once it is available."
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_verified_round_trips_and_skips_unchanged_writes() {
        let (vault, backend) = Vault::memory();
        vault.set_verified("k", "v1").unwrap();
        assert_eq!(vault.get("k").unwrap().as_deref(), Some("v1"));

        // Same value again: no keyring access needed, even if it went away.
        backend.set_unavailable(true);
        vault.set_verified("k", "v1").unwrap();
        // A changed value does need the keyring.
        assert!(vault.set_verified("k", "v2").unwrap_err().is_unavailable());
        backend.set_unavailable(false);
        vault.set_verified("k", "v2").unwrap();
        assert_eq!(vault.get("k").unwrap().as_deref(), Some("v2"));
    }

    #[test]
    fn delete_forgets_the_synced_marker() {
        let (vault, backend) = Vault::memory();
        vault.set_verified("k", "v").unwrap();
        assert!(vault.delete("k").unwrap());
        assert!(!vault.delete("k").unwrap());
        // After a delete the same value must be written again, not skipped.
        vault.set_verified("k", "v").unwrap();
        assert_eq!(backend.get("k").unwrap().as_deref(), Some("v"));
    }

    #[test]
    fn mismatched_read_back_is_a_failure() {
        struct Lossy;
        impl SecretBackend for Lossy {
            fn get(&self, _: &str) -> Result<Option<String>, SecretError> {
                Ok(Some("something else".into()))
            }
            fn set(&self, _: &str, _: &str) -> Result<(), SecretError> {
                Ok(())
            }
            fn delete(&self, _: &str) -> Result<bool, SecretError> {
                Ok(false)
            }
        }
        let vault = Vault::new(Arc::new(Lossy));
        let err = vault.set_verified("k", "v").unwrap_err();
        assert!(!err.is_unavailable());
        // And it must not be remembered as synced.
        assert!(vault.set_verified("k", "v").is_err());
    }

    /// Talks to the real Secret Service under a throwaway service name.
    /// Run with: cargo test --lib secrets::tests::live -- --ignored
    #[test]
    #[ignore = "needs a running Secret Service (KWallet / GNOME Keyring)"]
    fn live_keyring_round_trip() {
        let backend = KeyringBackend::new("ds2modpack-live-test");
        let key = format!("probe-{}", std::process::id());
        assert_eq!(backend.get(&key).unwrap(), None);
        backend.set(&key, "s3cret value").unwrap();
        assert_eq!(backend.get(&key).unwrap().as_deref(), Some("s3cret value"));
        backend.set(&key, "changed").unwrap();
        assert_eq!(backend.get(&key).unwrap().as_deref(), Some("changed"));
        assert!(backend.delete(&key).unwrap());
        assert!(!backend.delete(&key).unwrap());
        assert_eq!(backend.get(&key).unwrap(), None);
    }
}
