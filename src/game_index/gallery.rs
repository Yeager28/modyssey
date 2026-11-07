//! Learn game-file hashes from the whole Wabbajack gallery.
//!
//! Every gallery modlist pins the game files it was built against. Reading
//! just the `modlist` entry of each (see [`super::remote`]) turns the gallery
//! into a source of `(game, file, hash, version)` facts for the index.

use super::remote::{scan_cdn_modlist, ModlistGameFiles};
use crate::downloaders::wabbajack_cdn::WabbajackCdnDownloader;
use crate::modlist::browser::ModlistBrowser;
use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use std::collections::HashSet;

/// Hosts the CDN reader understands. Lists elsewhere (GitHub, Drive) are
/// skipped rather than downloaded whole.
const CDN_HOSTS: &[&str] = &[
    "authored-files.wabbajack.org",
    "mirror.wabbajack.org",
    "patches.wabbajack.org",
    "test-files.wabbajack.org",
];

#[derive(Debug, Clone, Default)]
pub struct ScanOptions {
    /// Only lists whose title or machine name contains this (case-insensitive).
    pub only: Option<String>,
    /// Stop after this many lists.
    pub limit: Option<usize>,
    /// Lists read in parallel.
    pub concurrency: usize,
}

#[derive(Debug)]
pub struct ListScan {
    pub title: String,
    pub version: String,
    pub outcome: Result<ScannedList>,
}

#[derive(Debug)]
pub struct ScannedList {
    pub files: ModlistGameFiles,
    pub fetched_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Default)]
pub struct ScanReport {
    pub scans: Vec<ListScan>,
    /// Gallery lists not on the CDN, or without a download link.
    pub skipped: usize,
}

fn is_cdn_url(url: &str) -> bool {
    let url = WabbajackCdnDownloader::remap_url(url);
    reqwest::Url::parse(&url)
        .ok()
        .and_then(|u| u.host_str().map(|h| CDN_HOSTS.contains(&h)))
        .unwrap_or(false)
}

/// Read every matching gallery list's game files. `on_done(done, total, scan)`
/// is called as each list finishes so the caller can report progress.
pub async fn scan_gallery(
    opts: &ScanOptions,
    mut on_done: impl FnMut(usize, usize, &ListScan),
) -> Result<ScanReport> {
    let mut browser = ModlistBrowser::new()?;
    let lists = browser
        .fetch_modlists()
        .await
        .context("Fetching the Wabbajack gallery")?
        .to_vec();

    let needle = opts.only.as_ref().map(|s| s.to_lowercase());
    let mut seen_urls = HashSet::new();
    let mut targets = Vec::new();
    let mut report = ScanReport::default();
    for list in lists {
        if let Some(n) = &needle {
            if !list.title.to_lowercase().contains(n)
                && !list.machine_name.to_lowercase().contains(n)
            {
                continue;
            }
        }
        match list.download_url() {
            Some(url) if is_cdn_url(url) => {
                // The same list can be published from more than one repository.
                if seen_urls.insert(url.to_string()) {
                    targets.push((list.title.clone(), list.version.clone(), url.to_string()));
                }
            }
            _ => report.skipped += 1,
        }
    }
    if let Some(limit) = opts.limit {
        targets.truncate(limit);
    }

    let total = targets.len();
    let cdn = WabbajackCdnDownloader::new()?;
    let cdn = &cdn;
    let mut done = 0;
    let mut results = stream::iter(targets)
        .map(|(title, version, url)| async move {
            let outcome = scan_one(cdn, &url).await;
            ListScan {
                title,
                version,
                outcome,
            }
        })
        .buffer_unordered(opts.concurrency.max(1));
    while let Some(scan) = results.next().await {
        done += 1;
        on_done(done, total, &scan);
        report.scans.push(scan);
    }
    Ok(report)
}

async fn scan_one(cdn: &WabbajackCdnDownloader, url: &str) -> Result<ScannedList> {
    let definition = cdn.get_definition(url).await?;
    let url = url.to_string();
    let scan = tokio::task::spawn_blocking(move || scan_cdn_modlist(&url, &definition)).await??;
    Ok(ScannedList {
        files: scan.files,
        fetched_bytes: scan.fetched_bytes,
        total_bytes: scan.total_bytes,
    })
}
