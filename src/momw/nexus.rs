//! Choosing the Nexus file a MOMW list entry refers to.
//!
//! A list entry names its file by the author's display name on Nexus (for
//! example `Cutting Room Floor - Modular`), not by file id. The id is only
//! given for a few entries. Unless an entry is `pinned`, the author's update
//! chain is followed, so a list keeps working after the author replaces the
//! file with a newer upload.

use super::plan::{MomwPlan, SourceKind};
use crate::downloaders::nexus_files::{NexusFile, NexusFileListing};
use crate::downloaders::NexusDownloader;
use crate::installer::host::HostedDownloadProvider;
use anyhow::{bail, Result};
use async_trait::async_trait;
use futures::StreamExt;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    pub file_id: u64,
    /// Archive name of the chosen file, when the listing contains it.
    pub archive_name: Option<String>,
    /// Size announced by Nexus, when the listing contains the file.
    pub size: Option<u64>,
    /// The chosen file replaced the one the entry named.
    pub followed_update: bool,
}

fn norm(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// `Patch for Purists-45096-5-1-2-1672944563.7z` -> `patch for purists`.
fn archive_stem(file_name: &str) -> String {
    let stem = file_name
        .rsplit_once('.')
        .map_or(file_name, |(stem, _)| stem);
    let mut parts: Vec<&str> = stem.split('-').collect();
    // Nexus appends `-<mod id>-<version parts>-<unix timestamp>`.
    let timestamp = parts
        .last()
        .is_some_and(|p| p.len() >= 9 && p.bytes().all(|b| b.is_ascii_digit()));
    if timestamp {
        while parts.len() > 1
            && parts
                .last()
                .is_some_and(|p| p.bytes().all(|b| b.is_ascii_digit()))
        {
            parts.pop();
        }
    }
    norm(&parts.join("-"))
}

fn is_removed(file: &NexusFile) -> bool {
    matches!(
        file.category_name.as_deref(),
        None | Some("ARCHIVED" | "DELETED")
    )
}

fn newest(mut files: Vec<&NexusFile>) -> Option<&NexusFile> {
    files.sort_by_key(|f| (f.uploaded_timestamp, f.file_id));
    files.pop()
}

/// The file a list entry names, before any update chain is applied.
fn named_file<'a>(listing: &'a NexusFileListing, wanted: &str) -> Option<&'a NexusFile> {
    let exact = |f: &&NexusFile| f.name.as_deref().map(str::trim) == Some(wanted.trim());
    let loose = |f: &&NexusFile| f.name.as_deref().map(norm).as_deref() == Some(&norm(wanted));
    let stem =
        |f: &&NexusFile| f.file_name.as_deref().map(archive_stem).as_deref() == Some(&norm(wanted));
    type Rule<'a> = &'a dyn Fn(&&NexusFile) -> bool;
    let rules: [Rule; 3] = [&exact, &loose, &stem];
    for rule in rules {
        let hits: Vec<&NexusFile> = listing.files.iter().filter(rule).collect();
        let live: Vec<&NexusFile> = hits.iter().copied().filter(|f| !is_removed(f)).collect();
        if let Some(file) = newest(if live.is_empty() { hits } else { live }) {
            return Some(file);
        }
    }
    None
}

pub fn select_file(
    listing: &NexusFileListing,
    wanted: &str,
    explicit_id: Option<u64>,
    pinned: bool,
) -> Result<Selection> {
    let start = match explicit_id {
        Some(id) => id,
        None => match named_file(listing, wanted) {
            Some(file) => file.file_id,
            None => {
                let mut names: Vec<&str> = listing
                    .files
                    .iter()
                    .filter(|f| !is_removed(f))
                    .filter_map(|f| f.name.as_deref())
                    .collect();
                names.truncate(8);
                bail!(
                    "no Nexus file named '{wanted}' (files offered: {})",
                    if names.is_empty() {
                        "none".to_owned()
                    } else {
                        names.join("; ")
                    }
                );
            }
        },
    };

    let mut current = start;
    let mut seen = HashSet::from([start]);
    if !pinned {
        // An entry can name a file that was since replaced, possibly more than
        // once. Only move to a replacement that is still downloadable.
        while let Some(next) = listing
            .file_updates
            .iter()
            .filter(|u| u.old_file_id == current)
            .filter_map(|u| listing.files.iter().find(|f| f.file_id == u.new_file_id))
            .filter(|f| !is_removed(f) && !seen.contains(&f.file_id))
            .max_by_key(|f| f.uploaded_timestamp)
        {
            current = next.file_id;
            seen.insert(current);
        }
    }
    let chosen = listing.files.iter().find(|f| f.file_id == current);
    Ok(Selection {
        file_id: current,
        archive_name: chosen.and_then(|f| f.file_name.clone()),
        size: chosen.and_then(|f| f.size_in_bytes),
        followed_update: current != start,
    })
}

/// Where the files of a Nexus mod come from.
#[async_trait]
pub trait NexusFileLister: Send + Sync {
    async fn list(&self, domain: &str, mod_id: u64) -> Result<NexusFileListing>;
}

/// The user's own API key (command-line use).
#[async_trait]
impl NexusFileLister for NexusDownloader {
    async fn list(&self, domain: &str, mod_id: u64) -> Result<NexusFileListing> {
        self.list_files(domain, mod_id).await
    }
}

/// Fluorine does the authenticated request; no credentials reach CLF3.
pub struct HostLister(pub Arc<dyn HostedDownloadProvider>);

#[async_trait]
impl NexusFileLister for HostLister {
    async fn list(&self, domain: &str, mod_id: u64) -> Result<NexusFileListing> {
        self.0.list_nexus_files(domain, mod_id).await
    }
}

/// Keeps listings on disk so repeated reviews do not spend API requests.
pub struct DiskCache<L> {
    pub inner: L,
    pub dir: PathBuf,
    pub ttl: Duration,
}

#[async_trait]
impl<L: NexusFileLister> NexusFileLister for DiskCache<L> {
    async fn list(&self, domain: &str, mod_id: u64) -> Result<NexusFileListing> {
        let path = self.dir.join(format!("{domain}-{mod_id}.json"));
        let fresh = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age < self.ttl);
        if fresh {
            if let Ok(listing) = std::fs::read(&path)
                .map_err(anyhow::Error::from)
                .and_then(|b| serde_json::from_slice(&b).map_err(Into::into))
            {
                return Ok(listing);
            }
        }
        let listing = self.inner.list(domain, mod_id).await?;
        // A cache that cannot be written only costs a later request.
        if std::fs::create_dir_all(&self.dir).is_ok() {
            if let Ok(bytes) = serde_json::to_vec(&listing) {
                let _ = std::fs::write(&path, bytes);
            }
        }
        Ok(listing)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome {
    Resolved {
        file_id: u64,
        archive_name: Option<String>,
        size: Option<u64>,
        followed_update: bool,
    },
    Failed {
        error: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedArchive {
    /// Position in `MomwPlan::archives`.
    pub index: usize,
    pub mod_name: String,
    pub wanted: Option<String>,
    pub domain: String,
    pub mod_id: u64,
    pub outcome: Outcome,
}

/// Resolve every Nexus archive of a plan to a file id. One listing is fetched
/// per mod, however many of its files the list uses.
pub async fn resolve_nexus_archives(
    plan: &MomwPlan,
    lister: &dyn NexusFileLister,
    concurrency: usize,
) -> Vec<ResolvedArchive> {
    let mods: HashSet<(String, u64)> = plan
        .archives
        .iter()
        .filter_map(|a| match &a.source {
            SourceKind::Nexus { domain, mod_id } => Some((domain.clone(), *mod_id)),
            _ => None,
        })
        .collect();
    let listings: HashMap<(String, u64), Result<NexusFileListing, String>> =
        futures::stream::iter(mods)
            .map(|(domain, mod_id)| async move {
                let result = lister
                    .list(&domain, mod_id)
                    .await
                    .map_err(|e| format!("{e:#}"));
                ((domain, mod_id), result)
            })
            .buffer_unordered(concurrency.max(1))
            .collect()
            .await;

    plan.archives
        .iter()
        .enumerate()
        .filter_map(|(index, job)| {
            let SourceKind::Nexus { domain, mod_id } = &job.source else {
                return None;
            };
            let wanted = job.file_name.clone();
            let outcome = match &listings[&(domain.clone(), *mod_id)] {
                Err(error) => Outcome::Failed {
                    error: error.clone(),
                },
                Ok(listing) => {
                    match select_file(
                        listing,
                        wanted.as_deref().unwrap_or_default(),
                        job.nexus_file_id,
                        job.pinned,
                    ) {
                        Ok(s) => Outcome::Resolved {
                            file_id: s.file_id,
                            archive_name: s.archive_name,
                            size: s.size,
                            followed_update: s.followed_update,
                        },
                        Err(e) => Outcome::Failed {
                            error: format!("{e:#}"),
                        },
                    }
                }
            };
            Some(ResolvedArchive {
                index,
                mod_name: job.mod_name.clone(),
                wanted,
                domain: domain.clone(),
                mod_id: *mod_id,
                outcome,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(id: u64, name: &str, archive: &str, category: &str, ts: i64) -> NexusFile {
        NexusFile {
            file_id: id,
            name: Some(name.into()),
            file_name: Some(archive.into()),
            category_name: Some(category.into()),
            uploaded_timestamp: ts,
            version: None,
            size_in_bytes: Some(7),
        }
    }

    fn listing() -> NexusFileListing {
        serde_json::from_value(serde_json::json!({
            "files": [
                {"file_id": 1, "name": "Cutting Room Floor - Modular", "file_name": "CRF Modular-47307-1-0-1.7z",
                 "category_name": "MAIN", "uploaded_timestamp": 100},
                {"file_id": 2, "name": "Cutting Room Floor - Modular", "file_name": "CRF Modular-47307-1-1-1.7z",
                 "category_name": "MAIN", "uploaded_timestamp": 200},
                {"file_id": 3, "name": "High Resolution Textures", "file_name": "CRF HiRes-47307-2-0-1000000000.7z",
                 "category_name": "OPTIONAL", "uploaded_timestamp": 300},
                {"file_id": 4, "name": "Old Thing", "file_name": "Old Thing-47307-1-0-5.7z",
                 "category_name": "ARCHIVED", "uploaded_timestamp": 50},
                {"file_id": 5, "name": "Patch for Purists", "file_name": "Patch for Purists-45096-5-1-2-1672944563.7z",
                 "category_name": "MAIN", "uploaded_timestamp": 1672944563},
                {"file_id": 6, "name": "Patch for Purists v6", "file_name": "Patch for Purists-45096-6-0-1700000000.7z",
                 "category_name": "MAIN", "uploaded_timestamp": 1700000000}
            ],
            "file_updates": [
                {"old_file_id": 1, "new_file_id": 2},
                {"old_file_id": 5, "new_file_id": 6}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn matches_the_display_name_and_prefers_the_newest_upload() {
        let s = select_file(&listing(), "Cutting Room Floor - Modular", None, false).unwrap();
        assert_eq!(s.file_id, 2);
    }

    #[test]
    fn matching_ignores_case_and_spacing() {
        let s = select_file(&listing(), "  high resolution   TEXTURES", None, false).unwrap();
        assert_eq!(s.file_id, 3);
    }

    #[test]
    fn falls_back_to_the_archive_name_without_its_nexus_suffix() {
        let s = select_file(&listing(), "CRF HiRes", None, false).unwrap();
        assert_eq!(s.file_id, 3);
        assert_eq!(archive_stem("My-Mod-123-1-0-1672944563.7z"), "my-mod");
        assert_eq!(archive_stem("Plain Name.zip"), "plain name");
    }

    #[test]
    fn follows_the_update_chain_unless_pinned() {
        let s = select_file(&listing(), "Patch for Purists", None, false).unwrap();
        assert_eq!((s.file_id, s.followed_update), (6, true));
        let s = select_file(&listing(), "Patch for Purists", None, true).unwrap();
        assert_eq!((s.file_id, s.followed_update), (5, false));
    }

    #[test]
    fn an_explicit_id_wins_over_the_name() {
        let s = select_file(&listing(), "anything", Some(3), true).unwrap();
        assert_eq!(s.file_id, 3);
        // An id the listing does not mention (e.g. archived) is still honoured.
        let s = select_file(&listing(), "anything", Some(99), true).unwrap();
        assert_eq!((s.file_id, s.archive_name), (99, None));
    }

    #[test]
    fn removed_files_are_only_used_when_nothing_else_matches() {
        let s = select_file(&listing(), "Old Thing", None, false).unwrap();
        assert_eq!(s.file_id, 4);
    }

    #[test]
    fn update_chain_stops_at_a_removed_replacement_and_cycles() {
        let mut l = listing();
        l.files.push(file(7, "Loop A", "a.7z", "MAIN", 1));
        l.files.push(file(8, "Loop B", "b.7z", "MAIN", 2));
        l.file_updates.push(
            serde_json::from_value(serde_json::json!({"old_file_id": 7, "new_file_id": 8}))
                .unwrap(),
        );
        l.file_updates.push(
            serde_json::from_value(serde_json::json!({"old_file_id": 8, "new_file_id": 7}))
                .unwrap(),
        );
        assert_eq!(select_file(&l, "Loop A", None, false).unwrap().file_id, 8);
        l.file_updates.push(
            serde_json::from_value(serde_json::json!({"old_file_id": 1, "new_file_id": 4}))
                .unwrap(),
        );
        // Replacement 4 is archived, so file 1 resolves to 2 only.
        assert_eq!(
            select_file(&l, "Cutting Room Floor - Modular", None, false)
                .unwrap()
                .file_id,
            2
        );
    }

    #[test]
    fn a_missing_file_lists_what_was_offered() {
        let err = select_file(&listing(), "Nope", None, false)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("Nope") && err.contains("High Resolution Textures"),
            "{err}"
        );
        assert!(!err.contains("Old Thing"), "{err}");
    }

    struct FakeLister {
        names: HashMap<(String, u64), Vec<String>>,
        calls: std::sync::Mutex<Vec<(String, u64)>>,
        broken: (String, u64),
    }

    #[async_trait]
    impl NexusFileLister for FakeLister {
        async fn list(&self, domain: &str, mod_id: u64) -> Result<NexusFileListing> {
            let key = (domain.to_owned(), mod_id);
            self.calls.lock().unwrap().push(key.clone());
            if key == self.broken {
                bail!("rate limited");
            }
            let files: Vec<NexusFile> = self.names[&key]
                .iter()
                .enumerate()
                .map(|(i, name)| file(i as u64 + 1, name, "x.7z", "MAIN", 1))
                .collect();
            Ok(NexusFileListing {
                files,
                file_updates: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn resolves_a_real_plan_with_one_request_per_mod() {
        let plan = crate::momw::build_plan(&crate::momw::plan::tests_fixture_ihv());
        let mut names: HashMap<(String, u64), Vec<String>> = HashMap::new();
        for a in &plan.archives {
            if let SourceKind::Nexus { domain, mod_id } = &a.source {
                names
                    .entry((domain.clone(), *mod_id))
                    .or_default()
                    .push(a.file_name.clone().unwrap());
            }
        }
        let broken = names.keys().next().unwrap().clone();
        let nexus_archives = plan
            .archives
            .iter()
            .filter(|a| matches!(a.source, SourceKind::Nexus { .. }))
            .count();
        let lister = FakeLister {
            names: names.clone(),
            calls: Default::default(),
            broken: broken.clone(),
        };
        let resolved = resolve_nexus_archives(&plan, &lister, 3).await;

        assert_eq!(resolved.len(), nexus_archives);
        assert_eq!(lister.calls.lock().unwrap().len(), names.len());
        for r in &resolved {
            let is_broken = (r.domain.clone(), r.mod_id) == broken;
            match (&r.outcome, is_broken) {
                (Outcome::Failed { error }, true) => assert!(error.contains("rate limited")),
                (Outcome::Resolved { .. }, false) => {}
                other => panic!("{}: unexpected {other:?}", r.mod_name),
            }
        }
    }
}
