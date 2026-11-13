//! One MOMW install at a time per machine.
//!
//! Installs share the download cache, the tools folder and the Nexus rate
//! limit, and OpenMW itself works on one active profile at a time, so a second
//! install (from the command line or from Fluorine) is refused while one runs.

#[cfg(unix)]
use anyhow::bail;
use anyhow::{Context, Result};
use std::fs::{File, OpenOptions};
#[cfg(unix)]
use std::io::Read;
use std::io::{Seek, Write};
use std::path::{Path, PathBuf};

/// Held until dropped; the operating system also releases it if Modyssey dies.
#[derive(Debug)]
pub struct InstallLock {
    _file: File,
}

/// `<local data>/modyssey/momw-install.lock`.
pub fn default_lock_path() -> Result<PathBuf> {
    Ok(dirs::data_local_dir()
        .context("could not find the local data directory")?
        .join("modyssey")
        .join("momw-install.lock"))
}

impl InstallLock {
    pub fn acquire(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .with_context(|| format!("opening {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // SAFETY: the descriptor is valid for the lifetime of `file`.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                let mut holder = String::new();
                let _ = file.read_to_string(&mut holder);
                let holder = holder.trim();
                bail!(
                    "another OpenMW list install is already running{}; wait for it to finish",
                    if holder.is_empty() {
                        String::new()
                    } else {
                        format!(" (process {holder})")
                    }
                );
            }
        }
        #[cfg(not(unix))]
        {
            // Windows has no flock. MOMW installs bundle Linux AppImages and
            // are not usable there, so exclusion is best-effort there.
            use std::io::Read;
            let mut holder = String::new();
            let _ = file.read_to_string(&mut holder);
        }
        file.set_len(0)?;
        file.rewind()?;
        write!(file, "{}", std::process::id())?;
        file.flush()?;
        Ok(Self { _file: file })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_second_install_is_refused_until_the_first_ends() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/install.lock");
        let first = InstallLock::acquire(&path).unwrap();
        let err = InstallLock::acquire(&path).unwrap_err().to_string();
        assert!(err.contains("already running"), "{err}");
        assert!(err.contains(&std::process::id().to_string()), "{err}");
        drop(first);
        InstallLock::acquire(&path).unwrap();
    }
}
