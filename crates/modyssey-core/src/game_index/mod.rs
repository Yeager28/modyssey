//! Index of known game files: which hash is which build of which file.
//!
//! A Wabbajack modlist pins each game file by xxHash64 and records the game
//! version it was authored against. Every modlist therefore tells us one fact
//! of the form `(game, path, hash) -> version`. This module collects those
//! facts into a small index that ships inside the binary, so that preflight
//! can say *which build* a mismatching file is, and which build the list
//! wanted, without any network access.
//!
//! The index lives in `data/game_index.json`. It is merge-only: adding a
//! modlist can add entries and versions, never remove them, and the file is
//! written sorted so the diff of a contribution is just the new lines.

// The binary compiles its own copy of this tree and only uses the lookup half.
#![allow(dead_code)]

pub mod cli;
pub mod gallery;
pub mod remote;

use crate::modlist::{DownloadState, GameFileSourceState, Modlist};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::OnceLock;

/// Index file format version. Bump on incompatible schema changes.
pub const FORMAT_VERSION: u32 = 1;

/// Longest game-file path we accept from a modlist. Real paths are well under
/// this; the cap only keeps a hostile modlist from bloating the index.
const MAX_PATH_LEN: usize = 260;

static EMBEDDED: OnceLock<GameIndex> = OnceLock::new();

/// One file of one game, identified by content hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedFile {
    /// Wabbajack game type, e.g. `SkyrimSpecialEdition`.
    pub game: String,
    /// Path relative to the game folder, with backslashes, as modlists record it.
    pub path: String,
    /// xxHash64 in Wabbajack's base64 form: what modlists pin a file by.
    pub hash: String,
    /// File size in bytes.
    pub size: u64,
    /// Game versions this exact file has been seen in, sorted.
    #[serde(default)]
    pub versions: Vec<String>,
    /// Valve's SHA-1 for the file, when it came from a depot manifest. Unset
    /// for entries learned from modlists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct IndexFile {
    format: u32,
    files: Vec<IndexedFile>,
}

/// Outcome of folding a modlist into the index.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AddStats {
    /// `GameFileSource` archives the modlist contained.
    pub seen: usize,
    /// Entries that did not exist before.
    pub new_files: usize,
    /// Existing entries that gained a version.
    pub new_versions: usize,
    /// Entries refused (bad hash, empty or oversized path).
    pub rejected: usize,
}

impl AddStats {
    pub fn changed(&self) -> bool {
        self.new_files > 0 || self.new_versions > 0
    }

    pub(crate) fn merge(&mut self, other: AddStats) {
        self.seen += other.seen;
        self.new_files += other.new_files;
        self.new_versions += other.new_versions;
        self.rejected += other.rejected;
    }
}

#[derive(Debug, Default, Clone)]
pub struct GameIndex {
    files: Vec<IndexedFile>,
    by_hash: HashMap<String, Vec<usize>>,
    by_key: HashMap<String, usize>,
}

fn normalize_path(path: &str) -> String {
    path.trim().replace('/', "\\")
}

fn key_of(game: &str, path: &str, hash: &str) -> String {
    format!(
        "{}\u{0}{}\u{0}{}",
        game.to_ascii_lowercase(),
        normalize_path(path).to_ascii_lowercase(),
        hash
    )
}

/// A Wabbajack hash is the base64 of an 8-byte little-endian xxHash64.
fn is_valid_hash(hash: &str) -> bool {
    STANDARD
        .decode(hash)
        .map(|bytes| bytes.len() == 8)
        .unwrap_or(false)
}

impl GameIndex {
    pub fn empty() -> Self {
        Self::default()
    }

    /// The index compiled into this binary. A corrupt embedded file degrades to
    /// an empty index rather than breaking preflight.
    pub fn embedded() -> &'static GameIndex {
        EMBEDDED.get_or_init(|| {
            match Self::parse(include_str!("../../../../data/game_index.json")) {
                Ok(index) => index,
                Err(err) => {
                    tracing::warn!("Embedded game index is unreadable, ignoring it: {err:#}");
                    Self::empty()
                }
            }
        })
    }

    pub fn parse(json: &str) -> Result<Self> {
        let file: IndexFile = serde_json::from_str(json).context("Invalid game index JSON")?;
        if file.format != FORMAT_VERSION {
            bail!(
                "Unsupported game index format {} (expected {})",
                file.format,
                FORMAT_VERSION
            );
        }
        let mut index = Self::empty();
        for entry in file.files {
            index.insert_entry(entry);
        }
        Ok(index)
    }

    /// Load an index file; a missing file is an empty index so a first
    /// contribution can create it.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).with_context(|| format!("Reading {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::empty()),
            Err(e) => Err(e).with_context(|| format!("Reading {}", path.display())),
        }
    }

    /// Write the index sorted, via a temp file so an interrupted run cannot
    /// leave a truncated index behind.
    pub fn save(&self, path: &Path) -> Result<()> {
        let mut files = self.files.clone();
        files.sort_by(|a, b| {
            (
                a.game.to_ascii_lowercase(),
                a.path.to_ascii_lowercase(),
                &a.hash,
            )
                .cmp(&(
                    b.game.to_ascii_lowercase(),
                    b.path.to_ascii_lowercase(),
                    &b.hash,
                ))
        });
        let mut text = serde_json::to_string_pretty(&IndexFile {
            format: FORMAT_VERSION,
            files,
        })?;
        text.push('\n');

        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).with_context(|| format!("Writing {}", tmp.display()))?;
        std::fs::rename(&tmp, path).with_context(|| format!("Replacing {}", path.display()))?;
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn files(&self) -> &[IndexedFile] {
        &self.files
    }

    /// Insert or merge one entry. Returns `(is_new_file, versions_added)`.
    fn insert_entry(&mut self, mut entry: IndexedFile) -> (bool, usize) {
        entry.path = normalize_path(&entry.path);
        entry.versions.retain(|v| !v.trim().is_empty());
        let key = key_of(&entry.game, &entry.path, &entry.hash);

        if let Some(&i) = self.by_key.get(&key) {
            let existing = &mut self.files[i];
            let before = existing.versions.len();
            let merged: BTreeSet<String> =
                existing.versions.drain(..).chain(entry.versions).collect();
            existing.versions = merged.into_iter().collect();
            if existing.sha1.is_none() {
                existing.sha1 = entry.sha1;
            }
            return (false, existing.versions.len() - before);
        }

        let versions = entry.versions.iter().collect::<BTreeSet<_>>().len();
        entry.versions = entry
            .versions
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let i = self.files.len();
        self.by_hash.entry(entry.hash.clone()).or_default().push(i);
        self.by_key.insert(key, i);
        self.files.push(entry);
        (true, versions)
    }

    /// Record that `hash` is `path` of `game` at `version`. Untrusted input:
    /// rejects hashes that are not an 8-byte xxHash64 and implausible paths.
    pub fn insert(
        &mut self,
        game: &str,
        path: &str,
        hash: &str,
        size: u64,
        version: &str,
    ) -> AddStats {
        let path = normalize_path(path);
        let ok = !game.trim().is_empty()
            && !path.is_empty()
            && path.len() <= MAX_PATH_LEN
            && is_valid_hash(hash);
        let mut stats = AddStats {
            seen: 1,
            ..AddStats::default()
        };
        if !ok {
            stats.rejected = 1;
            return stats;
        }
        let (is_new, versions_added) = self.insert_entry(IndexedFile {
            game: game.trim().to_string(),
            path,
            hash: hash.to_string(),
            size,
            versions: vec![version.trim().to_string()],
            sha1: None,
        });
        if is_new {
            stats.new_files = 1;
        } else {
            stats.new_versions = versions_added;
        }
        stats
    }

    /// Fold one pinned game file into the index. `fallback_game` is the
    /// modlist's game type, used when the entry does not name its own game.
    pub fn add_game_file(
        &mut self,
        fallback_game: &str,
        gf: &GameFileSourceState,
        size: u64,
    ) -> AddStats {
        // `gf.game` is the authoritative game for this file; a list's
        // top-level game_type can differ (e.g. TTW lists).
        let game = if gf.game.trim().is_empty() {
            fallback_game
        } else {
            gf.game.as_str()
        };
        self.insert(game, &gf.game_file, &gf.hash, size, &gf.game_version)
    }

    /// Fold a modlist's `GameFileSource` archives into the index.
    pub fn add_modlist(&mut self, modlist: &Modlist) -> AddStats {
        let mut total = AddStats::default();
        for archive in &modlist.archives {
            if let DownloadState::GameFileSource(gf) = &archive.state {
                total.merge(self.add_game_file(&modlist.game_type, gf, archive.size));
            }
        }
        total
    }

    /// Every entry with this hash for `game`.
    pub fn lookup_hash(&self, game: &str, hash: &str) -> Vec<&IndexedFile> {
        self.by_hash
            .get(hash)
            .into_iter()
            .flatten()
            .map(|&i| &self.files[i])
            .filter(|f| f.game.eq_ignore_ascii_case(game))
            .collect()
    }

    /// Versions a hash is known to belong to, sorted and de-duplicated.
    pub fn versions_for_hash(&self, game: &str, hash: &str) -> Vec<String> {
        self.lookup_hash(game, hash)
            .into_iter()
            .flat_map(|f| f.versions.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// Explain a hash mismatch in terms of game builds, when the index knows
    /// either side. `None` means the index has nothing useful to add.
    pub fn explain_mismatch(
        &self,
        game: &str,
        expected_hash: &str,
        actual_hash: &str,
    ) -> Option<String> {
        let have = self.versions_for_hash(game, actual_hash);
        let want = self.versions_for_hash(game, expected_hash);
        match (have.is_empty(), want.is_empty()) {
            (false, false) => Some(format!(
                "your file is from game version {}, this modlist was built for {}",
                have.join(" / "),
                want.join(" / ")
            )),
            (false, true) => Some(format!(
                "your file is from game version {}; the version this modlist wants is not in the index",
                have.join(" / ")
            )),
            (true, false) => Some(format!(
                "this modlist was built for game version {}; your file is not a build the index knows",
                want.join(" / ")
            )),
            (true, true) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Valid Wabbajack-style hashes (base64 of 8 bytes).
    const H1: &str = "AQIDBAUGBwg=";
    const H2: &str = "CAcGBQQDAgE=";

    #[test]
    fn rejects_malformed_hashes_and_paths() {
        let mut idx = GameIndex::empty();
        for hash in ["", "not base64!", "AQID", "AQIDBAUGBwgJCg=="] {
            let s = idx.insert("Skyrim", "a.esm", hash, 1, "1");
            assert_eq!(s.rejected, 1, "hash {hash:?}");
        }
        assert_eq!(idx.insert("Skyrim", "  ", H1, 1, "1").rejected, 1);
        let long = "x".repeat(MAX_PATH_LEN + 1);
        assert_eq!(idx.insert("Skyrim", &long, H1, 1, "1").rejected, 1);
        assert_eq!(idx.insert("", "a.esm", H1, 1, "1").rejected, 1);
        assert!(idx.is_empty());
    }

    #[test]
    fn merges_versions_without_duplicates() {
        let mut idx = GameIndex::empty();
        let a = idx.insert("Skyrim", "Data/Skyrim.esm", H1, 10, "1.6.640");
        assert_eq!((a.new_files, a.new_versions), (1, 0));
        // Same file, case and slash style differ: same entry, new version.
        let b = idx.insert("skyrim", "data\\skyrim.ESM", H1, 10, "1.6.1170");
        assert_eq!((b.new_files, b.new_versions), (0, 1));
        // Exact repeat changes nothing.
        let c = idx.insert("Skyrim", "Data\\Skyrim.esm", H1, 10, "1.6.640");
        assert!(!c.changed());
        assert_eq!(idx.len(), 1);
        assert_eq!(idx.files()[0].path, "Data\\Skyrim.esm");
        assert_eq!(idx.files()[0].versions, ["1.6.1170", "1.6.640"]);
    }

    #[test]
    fn empty_version_still_records_the_hash() {
        let mut idx = GameIndex::empty();
        idx.insert("Skyrim", "a.esm", H1, 1, "");
        assert_eq!(idx.len(), 1);
        assert!(idx.files()[0].versions.is_empty());
    }

    #[test]
    fn explain_mismatch_covers_each_case() {
        let mut idx = GameIndex::empty();
        idx.insert("Skyrim", "SkyrimSE.exe", H1, 1, "1.5.97");
        idx.insert("Skyrim", "SkyrimSE.exe", H2, 1, "1.6.1170");
        idx.insert("Fallout4", "Fallout4.exe", "CQkJCQkJCQk=", 1, "9.9");

        let both = idx.explain_mismatch("Skyrim", H1, H2).unwrap();
        assert!(
            both.contains("1.6.1170") && both.contains("1.5.97"),
            "{both}"
        );

        assert!(idx
            .explain_mismatch("Skyrim", "CgoKCgoKCgo=", H2)
            .unwrap()
            .contains("not in the index"));
        assert!(idx
            .explain_mismatch("Skyrim", H1, "CgoKCgoKCgo=")
            .unwrap()
            .contains("1.5.97"));
        assert!(idx
            .explain_mismatch("Skyrim", "CgoKCgoKCgo=", "CwsLCwsLCws=")
            .is_none());
        // Another game's hash must not leak across games.
        assert!(idx.explain_mismatch("Fallout4", H1, H2).is_none());
    }

    #[test]
    fn save_is_sorted_and_round_trips() {
        let mut idx = GameIndex::empty();
        idx.insert("Skyrim", "b.esm", H2, 2, "2");
        idx.insert("Skyrim", "a.esm", H1, 1, "1");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/index.json");
        idx.save(&path).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.find("a.esm").unwrap() < text.find("b.esm").unwrap());
        assert!(!path.with_extension("json.tmp").exists());

        let loaded = GameIndex::load(&path).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.versions_for_hash("Skyrim", H1), ["1"]);
    }

    #[test]
    fn load_missing_file_is_empty_and_bad_format_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert!(GameIndex::load(&dir.path().join("nope.json"))
            .unwrap()
            .is_empty());
        assert!(GameIndex::parse(r#"{"format":99,"files":[]}"#).is_err());
        assert!(GameIndex::parse("garbage").is_err());
    }

    #[test]
    fn embedded_index_explains_a_real_version_mismatch() {
        // Same game file, two different builds: the shipped data must be able
        // to name both sides, the way preflight uses it.
        let index = GameIndex::embedded();
        let mut by_path: HashMap<(String, String), Vec<&IndexedFile>> = HashMap::new();
        for f in index.files() {
            by_path
                .entry((f.game.clone(), f.path.to_ascii_lowercase()))
                .or_default()
                .push(f);
        }
        let (a, b) = by_path
            .values()
            .find_map(|fs| {
                let a = fs.first()?;
                let b = fs
                    .iter()
                    .find(|f| f.hash != a.hash && f.versions != a.versions)?;
                Some((*a, *b))
            })
            .expect("shipped index should hold a file seen in two builds");

        let msg = index
            .explain_mismatch(&a.game, &a.hash, &b.hash)
            .expect("both hashes are indexed");
        assert!(
            msg.contains(&b.versions[0]) && msg.contains(&a.versions[0]),
            "{msg}"
        );
        println!("{} {}: {msg}", a.game, a.path);
    }

    #[test]
    fn embedded_index_parses() {
        // Guards against committing a malformed data/game_index.json.
        GameIndex::parse(include_str!("../../../../data/game_index.json")).unwrap();
    }
}
