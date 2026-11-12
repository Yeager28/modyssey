//! Settings management for Modyssey
//!
//! Stores user preferences in ~/.config/modyssey/settings.json

// Used by lib crate
#![allow(dead_code)]

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::secrets::{warn_unavailable_once, Vault};

/// Keyring entry names for the secrets kept out of settings.json.
const KEY_NEXUS_API_KEY: &str = "nexus_api_key";
const KEY_LOVERSLAB_PASSWORD: &str = "loverslab_password";

/// Browser install path choices saved per Wabbajack list.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BrowserListPaths {
    #[serde(default)]
    pub downloads_dir: String,

    #[serde(default)]
    pub install_dir: String,
}

/// Mirror of `.modyssey-install.json` in settings, keyed by `machine_name`. Lets
/// `modyssey modlist check` find installs that aren't currently mounted (i.e.
/// when the install dir is on removable storage or temporarily unavailable).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstalledModlistRecord {
    /// Modlist title as taken from the .wabbajack header.
    #[serde(default)]
    pub name: String,

    /// Version string the modlist self-reports at install time.
    #[serde(default)]
    pub installed_version: String,

    /// Original URL the .wabbajack was downloaded from (None for local-file).
    #[serde(default)]
    pub wabbajack_url: Option<String>,

    /// RFC3339 timestamp of when the install finished.
    #[serde(default)]
    pub installed_at: String,

    /// Where downloaded archives live — needed to re-run the installer.
    #[serde(default)]
    pub downloads_dir: String,

    /// The install directory itself.
    #[serde(default)]
    pub install_dir: String,
}

/// User settings for Modyssey
///
/// `nexus_api_key` and `loverslab_password` are held in the system keyring,
/// not in settings.json: `load` fills them in and `save` moves them out. If no
/// keyring is available they stay in the (owner-only) file as before.
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    /// Default installation directory
    #[serde(default)]
    pub default_install_dir: String,

    /// Default downloads directory
    #[serde(default)]
    pub default_downloads_dir: String,

    /// Nexus Mods API key
    #[serde(default)]
    pub nexus_api_key: String,

    /// Selected GPU index (None = auto-select)
    #[serde(default)]
    pub gpu_index: Option<usize>,

    /// GPU name for display (informational only)
    #[serde(default)]
    pub gpu_name: String,

    /// Path to TTW (Tale of Two Wastelands) MPI installer binary
    #[serde(default)]
    pub ttw_installer_path: String,

    /// Path to TTW MPI file (the installation package)
    #[serde(default)]
    pub ttw_mpi_path: String,

    /// Path to Fallout 3 installation (required for TTW)
    #[serde(default)]
    pub fallout3_path: String,

    /// Path to existing TTW output directory (if already installed)
    #[serde(default)]
    pub ttw_output_path: String,

    /// Optional directory for persistent patched-file cache
    #[serde(default)]
    pub patch_cache_dir: String,

    /// LoversLab email for automated downloads
    #[serde(default)]
    pub loverslab_email: String,

    /// LoversLab password for automated downloads
    #[serde(default)]
    pub loverslab_password: String,

    /// Last selected browser game filter.
    #[serde(default)]
    pub browser_game_filter: String,

    /// Last state of the browser NSFW filter.
    #[serde(default)]
    pub browser_show_nsfw: bool,

    /// Last state of the browser unavailable-list filter.
    #[serde(default)]
    pub browser_show_unavailable: bool,

    /// Last state of the browser installed-only filter.
    #[serde(default)]
    pub browser_show_installed_only: bool,

    /// Last selected online modlist machine name.
    #[serde(default)]
    pub browser_last_selected_modlist: Option<String>,

    /// Per-modlist install path choices keyed by machine name.
    #[serde(default)]
    pub browser_list_paths: HashMap<String, BrowserListPaths>,

    /// Records of successful installs keyed by machine_name. Mirrors the
    /// per-install `.modyssey-install.json` so we can find installs whose dir
    /// isn't currently accessible (e.g. external drive unplugged).
    #[serde(default)]
    pub installed_modlists: HashMap<String, InstalledModlistRecord>,

    /// Legacy preference retained for settings compatibility; standalone
    /// installations ignore it and never register with a manager automatically.
    #[serde(default)]
    pub add_to_fluorine: bool,

    /// Optional override for the Fluorine install directory. When empty, the
    /// integration auto-detects (PATH + common locations) and falls back to
    /// ~/.local/share/fluorine-manager for auto-downloads.
    #[serde(default)]
    pub fluorine_path: String,

    /// True once `load` has successfully consulted the keyring, which is what
    /// makes an empty secret field mean "the user cleared it" rather than
    /// "we could not read it". Never written to disk.
    #[serde(skip)]
    secrets_readable: bool,
}

impl std::fmt::Debug for Settings {
    // Secrets must not leak through `{:?}` into logs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let redact = |s: &str| if s.is_empty() { "" } else { "<redacted>" };
        f.debug_struct("Settings")
            .field("default_install_dir", &self.default_install_dir)
            .field("default_downloads_dir", &self.default_downloads_dir)
            .field("nexus_api_key", &redact(&self.nexus_api_key))
            .field("loverslab_email", &self.loverslab_email)
            .field("loverslab_password", &redact(&self.loverslab_password))
            .finish_non_exhaustive()
    }
}

impl Settings {
    /// Get the config directory path (~/.config/modyssey)
    pub(crate) fn config_dir() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .context("Could not determine config directory")?
            .join("modyssey");

        Ok(config_dir)
    }

    /// Get the settings file path
    pub fn settings_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("settings.json"))
    }

    /// Load settings from disk, or return defaults if not found
    pub fn load() -> Self {
        match Self::settings_path().and_then(|p| Self::try_load(&p, &Vault::system())) {
            Ok(settings) => settings,
            Err(e) => {
                eprintln!("Could not load settings: {}. Using defaults.", e);
                Self::default()
            }
        }
    }

    /// Try to load settings, returning error on failure
    fn try_load(path: &Path, vault: &Vault) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let content =
            std::fs::read_to_string(path).with_context(|| format!("Failed to read {:?}", path))?;

        let mut settings: Self = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {:?}", path))?;

        if settings.resolve_secrets(vault) {
            // A plaintext secret was moved into the keyring: rewrite the file
            // without it. Failing to do so only means it is moved next time.
            if let Err(e) = settings.save_to(path, vault) {
                tracing::warn!("Could not rewrite settings after moving secrets: {e:#}");
            }
        }

        Ok(settings)
    }

    /// The secret fields with their keyring entry names.
    fn secret_fields(&mut self) -> [(&'static str, &mut String); 2] {
        [
            (KEY_NEXUS_API_KEY, &mut self.nexus_api_key),
            (KEY_LOVERSLAB_PASSWORD, &mut self.loverslab_password),
        ]
    }

    /// Fill secret fields from the keyring, and migrate any that were found in
    /// the file. Returns true if a secret was migrated and the file should be
    /// rewritten without it.
    fn resolve_secrets(&mut self, vault: &Vault) -> bool {
        let mut migrated = false;
        let mut readable = true;
        for (key, field) in self.secret_fields() {
            if !field.is_empty() {
                // Plaintext from an older version, or left by a keyring outage.
                match vault.set_verified(key, field) {
                    Ok(()) => migrated = true,
                    Err(e) => {
                        readable = false;
                        warn_unavailable_once(&e);
                    }
                }
            } else {
                match vault.get(key) {
                    Ok(Some(value)) => *field = value,
                    Ok(None) => {}
                    Err(e) => {
                        readable = false;
                        warn_unavailable_once(&e);
                    }
                }
            }
        }
        self.secrets_readable = readable;
        migrated
    }

    /// Save settings to disk
    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::settings_path()?, &Vault::system())
    }

    fn save_to(&self, path: &Path, vault: &Vault) -> Result<()> {
        let mut on_disk = self.clone();
        let readable = self.secrets_readable;
        for (key, field) in on_disk.secret_fields() {
            if field.is_empty() {
                // The user cleared it. Only delete when we know the keyring was
                // readable at load; otherwise an outage would erase a good key.
                if readable {
                    if let Err(e) = vault.delete(key) {
                        warn_unavailable_once(&e);
                    }
                }
            } else {
                match vault.set_verified(key, field) {
                    Ok(()) => field.clear(),
                    // Keep it in the file rather than lose it.
                    Err(e) => warn_unavailable_once(&e),
                }
            }
        }
        on_disk.write_file(path)
    }

    fn write_file(&self, path: &Path) -> Result<()> {
        let config_dir = path.parent().context("Settings path has no parent")?;
        std::fs::create_dir_all(config_dir)
            .with_context(|| format!("Failed to create {:?}", config_dir))?;

        let content = serde_json::to_string_pretty(self).context("Failed to serialize settings")?;

        let mut options = std::fs::OpenOptions::new();
        options.create(true).write(true).truncate(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options
            .open(path)
            .with_context(|| format!("Failed to open {:?} for writing", path))?;
        file.write_all(content.as_bytes())
            .with_context(|| format!("Failed to write {:?}", path))?;
        #[cfg(unix)]
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("Failed to secure permissions on {:?}", path))?;

        Ok(())
    }

    /// Check if any defaults are set
    pub fn has_defaults(&self) -> bool {
        !self.default_install_dir.is_empty()
            || !self.default_downloads_dir.is_empty()
            || !self.nexus_api_key.is_empty()
    }

    /// Check if TTW settings are configured
    pub fn has_ttw_config(&self) -> bool {
        !self.ttw_output_path.is_empty()
            || (!self.ttw_installer_path.is_empty()
                && !self.ttw_mpi_path.is_empty()
                && !self.fallout3_path.is_empty())
    }

    /// Check if TTW can be installed (has installer, MPI, and FO3 paths)
    pub fn can_install_ttw(&self) -> bool {
        !self.ttw_installer_path.is_empty()
            && !self.ttw_mpi_path.is_empty()
            && !self.fallout3_path.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_default() {
        let settings = Settings::default();
        assert!(settings.default_install_dir.is_empty());
        assert!(settings.nexus_api_key.is_empty());
        assert!(settings.gpu_index.is_none());
    }

    #[test]
    fn test_settings_serialize() {
        let settings = Settings {
            default_install_dir: "/home/user/Games".into(),
            default_downloads_dir: "/home/user/Downloads".into(),
            nexus_api_key: "test_key".into(),
            gpu_index: Some(0),
            gpu_name: "Test GPU".into(),
            ttw_installer_path: String::new(),
            ttw_mpi_path: String::new(),
            fallout3_path: String::new(),
            ttw_output_path: String::new(),
            patch_cache_dir: "/home/user/.cache/modyssey/patches".into(),
            loverslab_email: String::new(),
            loverslab_password: String::new(),
            browser_game_filter: String::new(),
            browser_show_nsfw: false,
            browser_show_unavailable: false,
            browser_show_installed_only: false,
            browser_last_selected_modlist: None,
            browser_list_paths: HashMap::new(),
            installed_modlists: HashMap::new(),
            add_to_fluorine: false,
            fluorine_path: String::new(),
            ..Default::default()
        };

        let json = serde_json::to_string(&settings).unwrap();
        let loaded: Settings = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.default_install_dir, settings.default_install_dir);
        assert_eq!(loaded.gpu_index, Some(0));
    }

    #[test]
    fn test_ttw_config_check() {
        let mut settings = Settings::default();
        assert!(!settings.has_ttw_config());
        assert!(!settings.can_install_ttw());

        settings.ttw_output_path = "/path/to/ttw".into();
        assert!(settings.has_ttw_config());

        let mut settings2 = Settings::default();
        settings2.ttw_installer_path = "/path/to/installer".into();
        settings2.ttw_mpi_path = "/path/to/file.mpi".into();
        settings2.fallout3_path = "/path/to/fo3".into();
        assert!(settings2.has_ttw_config());
        assert!(settings2.can_install_ttw());
    }

    // --- keyring-backed secrets ---------------------------------------

    fn secrets_fixture() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        Vault,
        std::sync::Arc<crate::secrets::MemoryBackend>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let (vault, backend) = Vault::memory();
        (dir, path, vault, backend)
    }

    fn file_text(path: &std::path::Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn secrets_leave_the_file_and_come_back_on_load() {
        let (_d, path, vault, _b) = secrets_fixture();
        let mut s = Settings::default();
        s.default_install_dir = "/games".into();
        s.nexus_api_key = "NEXUS-SECRET".into();
        s.loverslab_email = "me@example.com".into();
        s.loverslab_password = "LL-SECRET".into();
        s.save_to(&path, &vault).unwrap();

        let text = file_text(&path);
        assert!(
            !text.contains("NEXUS-SECRET") && !text.contains("LL-SECRET"),
            "{text}"
        );
        assert!(text.contains("/games") && text.contains("me@example.com"));

        let loaded = Settings::try_load(&path, &vault).unwrap();
        assert_eq!(loaded.nexus_api_key, "NEXUS-SECRET");
        assert_eq!(loaded.loverslab_password, "LL-SECRET");
        assert_eq!(loaded.loverslab_email, "me@example.com");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn legacy_plaintext_file_is_migrated_and_scrubbed() {
        let (_d, path, vault, _b) = secrets_fixture();
        std::fs::write(
            &path,
            r#"{"nexus_api_key":"OLD-PLAINTEXT","loverslab_password":"OLD-LL","loverslab_email":"a@b.c"}"#,
        )
        .unwrap();

        let loaded = Settings::try_load(&path, &vault).unwrap();
        // Callers still see the values...
        assert_eq!(loaded.nexus_api_key, "OLD-PLAINTEXT");
        assert_eq!(loaded.loverslab_password, "OLD-LL");
        // ...the keyring has them...
        assert_eq!(
            vault.get("nexus_api_key").unwrap().as_deref(),
            Some("OLD-PLAINTEXT")
        );
        // ...and the file no longer does.
        let text = file_text(&path);
        assert!(
            !text.contains("OLD-PLAINTEXT") && !text.contains("OLD-LL"),
            "{text}"
        );
        assert!(text.contains("a@b.c"));
    }

    #[test]
    fn keyring_outage_never_loses_a_secret() {
        let (_d, path, vault, backend) = secrets_fixture();
        backend.set_unavailable(true);

        let mut s = Settings::default();
        s.nexus_api_key = "KEEP-ME".into();
        s.save_to(&path, &vault).unwrap();
        // No keyring: the secret stays in the file, exactly as before.
        assert!(file_text(&path).contains("KEEP-ME"));
        assert_eq!(
            Settings::try_load(&path, &vault).unwrap().nexus_api_key,
            "KEEP-ME"
        );

        // Once the keyring is back, the next load moves it.
        backend.set_unavailable(false);
        let loaded = Settings::try_load(&path, &vault).unwrap();
        assert_eq!(loaded.nexus_api_key, "KEEP-ME");
        assert!(!file_text(&path).contains("KEEP-ME"));
        assert_eq!(
            vault.get("nexus_api_key").unwrap().as_deref(),
            Some("KEEP-ME")
        );
    }

    #[test]
    fn clearing_a_secret_deletes_it_from_the_keyring() {
        let (_d, path, vault, _b) = secrets_fixture();
        let mut s = Settings::default();
        s.nexus_api_key = "TEMP".into();
        s.save_to(&path, &vault).unwrap();

        let mut loaded = Settings::try_load(&path, &vault).unwrap();
        loaded.nexus_api_key.clear();
        loaded.save_to(&path, &vault).unwrap();

        assert_eq!(vault.get("nexus_api_key").unwrap(), None);
        assert_eq!(Settings::try_load(&path, &vault).unwrap().nexus_api_key, "");
    }

    #[test]
    fn a_keyring_outage_at_load_does_not_erase_secrets_on_save() {
        let (_d, path, vault, backend) = secrets_fixture();
        let mut s = Settings::default();
        s.nexus_api_key = "PRECIOUS".into();
        s.save_to(&path, &vault).unwrap();

        // Load while the keyring is down: the field is empty, but not "cleared".
        backend.set_unavailable(true);
        let mut during_outage = Settings::try_load(&path, &vault).unwrap();
        assert_eq!(during_outage.nexus_api_key, "");
        during_outage.default_install_dir = "/changed".into();

        // The keyring comes back before the app saves a settings change.
        backend.set_unavailable(false);
        during_outage.save_to(&path, &vault).unwrap();
        assert_eq!(
            vault.get("nexus_api_key").unwrap().as_deref(),
            Some("PRECIOUS")
        );
    }

    #[test]
    fn debug_output_redacts_secrets() {
        let mut s = Settings::default();
        s.nexus_api_key = "NEXUS-SECRET".into();
        s.loverslab_password = "LL-SECRET".into();
        let text = format!("{s:?}");
        assert!(
            !text.contains("NEXUS-SECRET") && !text.contains("LL-SECRET"),
            "{text}"
        );
    }
}
