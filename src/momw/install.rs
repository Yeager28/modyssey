//! Download every archive of a list and unpack it into the staging tree.

use super::fetch::{Candidate, Fetcher};
use super::nexus::{Outcome, ResolvedArchive};
use super::plan::{ArchiveJob, DataEntryKind, MomwPlan, SourceKind};
use super::stage::{
    apply_action, dir_ci_create, extract_into, merge_into, resolve_ci, PendingClean, Wrapper,
};
use crate::downloaders::ProgressCallback;
use anyhow::{bail, Context, Result};
use futures::StreamExt;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct StageOptions {
    /// Where downloaded archives are kept and reused from.
    pub downloads_dir: PathBuf,
    /// Staging tree root; `<Category>/<extract_to>/...` is built below it.
    pub stage_dir: PathBuf,
    /// Mods processed in parallel.
    pub concurrency: usize,
}

#[derive(Debug, Clone)]
pub enum Event {
    Downloading {
        archive: usize,
        name: String,
        done: u64,
        total: u64,
    },
    Extracting {
        archive: usize,
        name: String,
    },
    Staged {
        archive: usize,
    },
    Warning(String),
}

/// Receives progress; cloned into each download's progress callback.
pub type EventSink = std::sync::Arc<dyn Fn(Event) + Send + Sync>;

#[derive(Debug, Default)]
pub struct StageOutcome {
    pub cleans: Vec<PendingClean>,
    /// `(archive index, reason)` for archives that could not be staged.
    pub failures: Vec<(usize, String)>,
    pub warnings: Vec<String>,
}

/// One download cache for every list on this machine.
pub fn default_downloads_dir() -> Result<PathBuf> {
    Ok(dirs::data_local_dir()
        .context("could not find the local data directory")?
        .join("ds2modpack")
        .join("momw-downloads"))
}

fn file_name_of(url: &str, fallback: &str) -> String {
    let name = url
        .split(['?', '#'])
        .next()
        .and_then(|u| u.rsplit('/').next())
        .map(percent_decode)
        .filter(|n| n.contains('.'))
        .unwrap_or_else(|| fallback.to_owned());
    name.replace(['/', '\\', ':'], "_")
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&text[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Stage every archive of `plan`. Failures are collected per archive so one
/// bad download does not stop the rest.
pub async fn stage_archives(
    plan: &MomwPlan,
    resolved: &[ResolvedArchive],
    fetcher: &Fetcher,
    options: &StageOptions,
    on_event: &EventSink,
) -> StageOutcome {
    std::fs::create_dir_all(&options.downloads_dir).ok();
    // List actions rename and remove files, so a tree left by an earlier
    // failed run cannot be replayed. Downloads are cached separately.
    let _ = std::fs::remove_dir_all(&options.stage_dir);
    std::fs::create_dir_all(&options.stage_dir).ok();

    // Archives of one mod share a folder, so they run in order; different
    // mods run side by side.
    let mut by_mod: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, job) in plan.archives.iter().enumerate() {
        by_mod.entry(job.mod_index).or_default().push(index);
    }
    type ModResult = (Vec<PendingClean>, Vec<(usize, String)>, Vec<String>);
    let results: Vec<ModResult> = futures::stream::iter(by_mod)
        .map(|(_, indexes)| async move {
            let mut cleans = Vec::new();
            let mut failures = Vec::new();
            let mut warnings = Vec::new();
            for index in indexes {
                if fetcher.cancel.is_cancelled() {
                    failures.push((index, "cancelled".to_owned()));
                    continue;
                }
                match stage_one(
                    plan,
                    index,
                    resolved,
                    fetcher,
                    options,
                    on_event,
                    &mut cleans,
                    &mut warnings,
                )
                .await
                {
                    Ok(()) => on_event(Event::Staged { archive: index }),
                    Err(e) => failures.push((index, format!("{e:#}"))),
                }
            }
            (cleans, failures, warnings)
        })
        .buffer_unordered(options.concurrency.max(1))
        .collect()
        .await;

    let mut outcome = StageOutcome::default();
    for (cleans, failures, warnings) in results {
        outcome.cleans.extend(cleans);
        outcome.failures.extend(failures);
        outcome.warnings.extend(warnings);
    }
    outcome.failures.sort_by_key(|(i, _)| *i);
    outcome
}

fn progress_for(archive: usize, name: &str, on_event: &EventSink) -> ProgressCallback {
    let name = name.to_owned();
    let on_event = on_event.clone();
    Box::new(move |done, total, _| {
        on_event(Event::Downloading {
            archive,
            name: name.clone(),
            done,
            total,
        })
    })
}

#[allow(clippy::too_many_arguments)]
async fn stage_one(
    plan: &MomwPlan,
    index: usize,
    resolved: &[ResolvedArchive],
    fetcher: &Fetcher,
    options: &StageOptions,
    on_event: &EventSink,
    cleans: &mut Vec<PendingClean>,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let job = &plan.archives[index];
    let category_root = dir_ci_create(&options.stage_dir, &job.category)?;
    let target = dir_ci_create(&category_root, &job.extract_to)?;

    // What a candidate must contain to be accepted: only meaningful when this
    // is the mod's one archive.
    let archives_of_mod = plan
        .archives
        .iter()
        .filter(|a| a.mod_index == job.mod_index)
        .count();
    let required: Vec<String> = if archives_of_mod == 1 {
        plan.data_entries
            .iter()
            .filter_map(|e| match &e.kind {
                DataEntryKind::Mod {
                    mod_index,
                    data_path,
                    ..
                } if *mod_index == job.mod_index => Some(data_path.clone()),
                _ => None,
            })
            .collect()
    } else {
        Vec::new()
    };

    let candidates = candidates_for(job, index, resolved, fetcher).await?;
    let mut errors = Vec::new();
    for candidate in candidates {
        match try_candidate(
            job,
            index,
            &candidate,
            &required,
            &category_root,
            &target,
            fetcher,
            options,
            on_event,
        )
        .await
        {
            Ok(()) => {
                errors.clear();
                break;
            }
            Err(e) => errors.push(format!("{}: {e:#}", candidate.file_name)),
        }
    }
    if !errors.is_empty() {
        bail!("{}", errors.join(" | "));
    }

    for action in &job.actions {
        for warning in apply_action(&category_root, action, cleans)
            .with_context(|| format!("{}: list action", job.mod_name))?
        {
            let message = format!("{}: {warning}", job.mod_name);
            on_event(Event::Warning(message.clone()));
            warnings.push(message);
        }
    }
    Ok(())
}

/// A candidate plus how to obtain it.
enum Plan {
    Nexus {
        domain: String,
        mod_id: u64,
        file_id: u64,
        size: Option<u64>,
    },
    Url,
}

struct Attempt {
    candidate: Candidate,
    plan: Plan,
}

impl std::ops::Deref for Attempt {
    type Target = Candidate;
    fn deref(&self) -> &Candidate {
        &self.candidate
    }
}

async fn candidates_for(
    job: &ArchiveJob,
    index: usize,
    resolved: &[ResolvedArchive],
    fetcher: &Fetcher,
) -> Result<Vec<Attempt>> {
    let url_attempt = |url: &str, wrapper: Wrapper, file_name: String| Attempt {
        candidate: Candidate {
            url: url.to_owned(),
            wrapper,
            file_name,
        },
        plan: Plan::Url,
    };
    Ok(match &job.source {
        SourceKind::Nexus { domain, mod_id } => {
            let entry = resolved
                .iter()
                .find(|r| r.index == index)
                .with_context(|| format!("{}: Nexus file was not resolved", job.mod_name))?;
            match &entry.outcome {
                Outcome::Resolved {
                    file_id,
                    archive_name,
                    size,
                    ..
                } => vec![Attempt {
                    candidate: Candidate {
                        url: String::new(),
                        wrapper: Wrapper::Keep,
                        file_name: archive_name
                            .clone()
                            .unwrap_or_else(|| format!("{domain}-{mod_id}-{file_id}.7z")),
                    },
                    plan: Plan::Nexus {
                        domain: domain.clone(),
                        mod_id: *mod_id,
                        file_id: *file_id,
                        size: *size,
                    },
                }],
                Outcome::Failed { error } => bail!("Nexus file not found: {error}"),
            }
        }
        SourceKind::Direct { url } | SourceKind::GitHub { url } => {
            let name = file_name_of(url, &format!("{}.bin", job.extract_to));
            vec![url_attempt(url, Wrapper::Keep, name)]
        }
        SourceKind::GitLab { url } => fetcher
            .gitlab_candidates(url)
            .await
            .with_context(|| format!("GitLab source {url}"))?
            .into_iter()
            .map(|c| Attempt {
                candidate: c,
                plan: Plan::Url,
            })
            .collect(),
        SourceKind::Other { url } if url.contains('#') => {
            let candidate = fetcher
                .page_section_candidate(url)
                .await
                .with_context(|| format!("page section {url}"))?;
            vec![Attempt {
                candidate,
                plan: Plan::Url,
            }]
        }
        SourceKind::GoogleDrive { url } | SourceKind::Other { url } => {
            bail!("no downloader for {url}")
        }
        SourceKind::Unresolved | SourceKind::None => bail!("the list names no download"),
    })
}

#[allow(clippy::too_many_arguments)]
async fn try_candidate(
    job: &ArchiveJob,
    index: usize,
    attempt: &Attempt,
    required: &[String],
    category_root: &Path,
    target: &Path,
    fetcher: &Fetcher,
    options: &StageOptions,
    on_event: &EventSink,
) -> Result<()> {
    let archive = options.downloads_dir.join(&attempt.file_name);
    let cached = archive.is_file()
        && match &attempt.plan {
            Plan::Nexus { size: Some(s), .. } => archive.metadata().is_ok_and(|m| m.len() == *s),
            _ => archive.metadata().map(|m| m.len() > 0).unwrap_or(false),
        };
    if !cached {
        let partial = archive.with_extension(format!(
            "{}.part",
            archive.extension().and_then(|e| e.to_str()).unwrap_or("")
        ));
        let progress = progress_for(index, &attempt.file_name, on_event);
        match &attempt.plan {
            Plan::Nexus {
                domain,
                mod_id,
                file_id,
                size,
            } => {
                fetcher
                    .fetch_nexus(
                        domain,
                        *mod_id,
                        *file_id,
                        &attempt.file_name,
                        *size,
                        &partial,
                        Some(&progress),
                    )
                    .await?
            }
            Plan::Url => {
                fetcher
                    .fetch_url(&attempt.url, &partial, Some(&progress))
                    .await?
            }
        }
        std::fs::rename(&partial, &archive)?;
    }

    on_event(Event::Extracting {
        archive: index,
        name: attempt.file_name.clone(),
    });
    // Unpack beside the final folder first, so a candidate that turns out to
    // be wrong leaves nothing behind.
    let scratch = options.stage_dir.join(format!(".try-{index}"));
    let _ = std::fs::remove_dir_all(&scratch);
    let result = (|| -> Result<()> {
        let unpacked = scratch.join("data");
        extract_into(&archive, &unpacked, attempt.wrapper)?;
        // The required folders are relative to the category root, which is
        // `target`'s ancestor; place the unpacked files where they will be.
        let probe = scratch.join("category");
        let probe_target = dir_ci_create(&probe, &job.extract_to)?;
        merge_into(&unpacked, &probe_target)?;
        let missing: Vec<&String> = required
            .iter()
            .filter(|p| resolve_ci(&probe, p).is_none_or(|d| !d.is_dir()))
            .collect();
        if !missing.is_empty() {
            bail!(
                "does not contain the expected folder(s): {}",
                missing
                    .iter()
                    .take(3)
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        merge_into(&probe_target, target)?;
        let _ = category_root;
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::momw::fetch::NexusAccess;
    use crate::momw::plan::DataEntry;
    use crate::momw::schema::Action;
    use std::io::Write;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_util::sync::CancellationToken;

    fn zip_bytes(files: &[(&str, &str)]) -> Vec<u8> {
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut cursor);
            let options = zip::write::SimpleFileOptions::default();
            for (name, body) in files {
                zip.start_file(*name, options).unwrap();
                zip.write_all(body.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        cursor.into_inner()
    }

    /// Serves `GET /<name>` from a map until the test ends.
    async fn serve(files: Vec<(&'static str, Vec<u8>)>) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let files = files.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let path = request
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("/")
                        .trim_start_matches('/')
                        .to_owned();
                    let response = match files.iter().find(|(n, _)| *n == path) {
                        Some((_, body)) => {
                            let mut r = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/zip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                body.len()
                            )
                            .into_bytes();
                            r.extend_from_slice(body);
                            r
                        }
                        None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                    };
                    let _ = socket.write_all(&response).await;
                });
            }
        });
        port
    }

    fn job(mod_index: usize, url: String, extract_to: &str, actions: Vec<Action>) -> ArchiveJob {
        ArchiveJob {
            mod_index,
            mod_name: format!("Mod {mod_index}"),
            category: "Cat".into(),
            file_name: Some("x".into()),
            nexus_file_id: None,
            pinned: false,
            extract_to: extract_to.into(),
            source: SourceKind::Direct { url },
            actions,
        }
    }

    fn plan_of(archives: Vec<ArchiveJob>, data: Vec<(usize, &str)>) -> MomwPlan {
        let mut plan = crate::momw::build_plan(&crate::momw::plan::tests_fixture_ihv());
        plan.archives = archives;
        plan.data_entries = data
            .into_iter()
            .enumerate()
            .map(|(order, (mod_index, path))| DataEntry {
                order,
                kind: DataEntryKind::Mod {
                    mod_index,
                    category: "Cat".into(),
                    data_path: path.into(),
                },
                instance_name: path.replace('/', " - "),
                nested_in: None,
            })
            .collect();
        plan
    }

    #[tokio::test]
    async fn downloads_extracts_applies_actions_and_reuses_the_cache() {
        let port = serve(vec![
            (
                "first.zip",
                zip_bytes(&[("Data Files/a.esp", "a"), ("Data Files/Splash/s.tga", "s")]),
            ),
            ("second.zip", zip_bytes(&[("Extra/e.esp", "e")])),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let options = StageOptions {
            downloads_dir: dir.path().join("dl"),
            stage_dir: dir.path().join("stage"),
            concurrency: 2,
        };
        let plan = plan_of(
            vec![
                job(
                    0,
                    format!("http://127.0.0.1:{port}/first.zip"),
                    "ModA",
                    vec![Action::Remove {
                        path: Some("moda/data files/splash".into()),
                        paths: None,
                    }],
                ),
                job(
                    1,
                    format!("http://127.0.0.1:{port}/second.zip"),
                    "ModB",
                    vec![],
                ),
            ],
            vec![(0, "ModA/Data Files"), (1, "ModB/Extra")],
        );
        let fetcher = Fetcher::new(NexusAccess::None, CancellationToken::new()).unwrap();
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink: EventSink = {
            let events = events.clone();
            std::sync::Arc::new(move |e| events.lock().unwrap().push(format!("{e:?}")))
        };
        // Leftovers of an earlier failed run must not be replayed.
        std::fs::create_dir_all(options.stage_dir.join("ModA")).unwrap();
        std::fs::write(options.stage_dir.join("ModA/stale.txt"), "x").unwrap();
        let outcome = stage_archives(&plan, &[], &fetcher, &options, &sink).await;

        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert!(!options.stage_dir.join("ModA/stale.txt").exists());
        assert!(options
            .stage_dir
            .join("Cat/ModA/Data Files/a.esp")
            .is_file());
        assert!(!options
            .stage_dir
            .join("Cat/ModA/Data Files/Splash")
            .exists());
        assert!(options.stage_dir.join("Cat/ModB/Extra/e.esp").is_file());
        assert!(options.downloads_dir.join("first.zip").is_file());
        assert!(!options.stage_dir.join(".try-0").exists());
        assert!(events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.starts_with("Staged")));

        // A second run reuses the downloaded archives (the server is gone from
        // the plan's point of view: point it at a dead port).
        let offline = plan_of(
            vec![job(
                0,
                "http://127.0.0.1:9/first.zip".into(),
                "ModA",
                vec![],
            )],
            vec![(0, "ModA/Data Files")],
        );
        let quiet: EventSink = std::sync::Arc::new(|_| {});
        let again = stage_archives(&offline, &[], &fetcher, &options, &quiet).await;
        assert!(again.failures.is_empty(), "{:?}", again.failures);
    }

    #[tokio::test]
    async fn a_failed_download_is_reported_per_archive() {
        let port = serve(vec![("ok.zip", zip_bytes(&[("D/a.esp", "a")]))]).await;
        let dir = tempfile::tempdir().unwrap();
        let options = StageOptions {
            downloads_dir: dir.path().join("dl"),
            stage_dir: dir.path().join("stage"),
            concurrency: 2,
        };
        let plan = plan_of(
            vec![
                job(
                    0,
                    format!("http://127.0.0.1:{port}/missing.zip"),
                    "Bad",
                    vec![],
                ),
                job(1, format!("http://127.0.0.1:{port}/ok.zip"), "Good", vec![]),
            ],
            vec![(0, "Bad"), (1, "Good/D")],
        );
        let fetcher = Fetcher::new(NexusAccess::None, CancellationToken::new()).unwrap();
        let quiet: EventSink = std::sync::Arc::new(|_| {});
        let outcome = stage_archives(&plan, &[], &fetcher, &options, &quiet).await;
        assert_eq!(outcome.failures.len(), 1);
        assert_eq!(outcome.failures[0].0, 0);
        assert!(options.stage_dir.join("Cat/Good/D/a.esp").is_file());
    }

    #[test]
    fn file_names_come_from_the_url() {
        assert_eq!(
            file_name_of(
                "https://modding-openmw.com/files/Mesh%20Fix%20v1.2.7z",
                "x.bin"
            ),
            "Mesh Fix v1.2.7z"
        );
        assert_eq!(
            file_name_of(
                "https://www.dropbox.com/scl/fi/a/Uviriths_Legacy_3.53.7z?rlkey=k&dl=1",
                "x.bin"
            ),
            "Uviriths_Legacy_3.53.7z"
        );
        assert_eq!(
            file_name_of("https://mega.nz/#!abc!def", "Fallback.bin"),
            "Fallback.bin"
        );
    }
}
