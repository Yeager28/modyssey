//! Getting a list entry's archive onto disk.
//!
//! Nexus files go through the host (Fluorine authorizes them, so no
//! credentials reach CLF3) or, on the command line, the user's own Premium API
//! key. Everything else is a plain URL, Mega link or GitLab project.

use super::stage::Wrapper;
use crate::downloaders::{
    download_file_with_callback, mega_native, HttpClient, NexusDownloader, ProgressCallback,
};
use crate::installer::host::{HostedDownloadProvider, HostedNexusRequest};
use anyhow::{bail, Context, Result};
use reqwest::Url;
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// How Nexus downloads are authorized.
#[derive(Clone)]
pub enum NexusAccess {
    None,
    /// Fluorine resolves download locations.
    Hosted(Arc<dyn HostedDownloadProvider>),
    /// The user's API key; the account must be Premium for direct links.
    Api(Arc<NexusDownloader>),
}

/// One concrete archive to try for a list entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub url: String,
    pub wrapper: Wrapper,
    /// File name to save it under.
    pub file_name: String,
}

pub struct Fetcher {
    http: HttpClient,
    api: reqwest::Client,
    pub nexus: NexusAccess,
    pub cancel: CancellationToken,
}

impl Fetcher {
    pub fn new(nexus: NexusAccess, cancel: CancellationToken) -> Result<Self> {
        Ok(Self {
            http: HttpClient::new()?,
            api: reqwest::Client::builder()
                .user_agent(concat!(
                    "ds2modpack/",
                    env!("CARGO_PKG_VERSION"),
                    " (+https://github.com/himanshudbg/ds2-modpack-app)"
                ))
                .timeout(std::time::Duration::from_secs(60))
                .build()?,
            nexus,
            cancel,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn fetch_nexus(
        &self,
        domain: &str,
        mod_id: u64,
        file_id: u64,
        archive_name: &str,
        size: Option<u64>,
        dest: &Path,
        progress: Option<&ProgressCallback>,
    ) -> Result<()> {
        let urls = match &self.nexus {
            NexusAccess::None => bail!("Nexus downloads need Fluorine or a Nexus API key"),
            NexusAccess::Hosted(host) => {
                host.resolve_nexus(HostedNexusRequest {
                    archive_name: archive_name.to_owned(),
                    expected_size: size.unwrap_or(0),
                    domain: domain.to_owned(),
                    mod_id,
                    file_id,
                })
                .await?
            }
            NexusAccess::Api(nexus) => vec![nexus
                .get_download_link(domain, mod_id, file_id)
                .await
                .with_context(|| format!("Nexus {domain}/mods/{mod_id}/files/{file_id}"))?],
        };
        let mut failures = Vec::new();
        for url in urls {
            if self.cancel.is_cancelled() {
                bail!("cancelled");
            }
            match download_file_with_callback(&self.http, &url, dest, size, progress).await {
                Ok(_) => return Ok(()),
                Err(e) => failures.push(format!("{e:#}")),
            }
        }
        bail!("every Nexus mirror failed: {}", failures.join("; "))
    }

    /// Download a URL that needs no authorization.
    pub async fn fetch_url(
        &self,
        url: &str,
        dest: &Path,
        progress: Option<&ProgressCallback>,
    ) -> Result<()> {
        let parsed = Url::parse(url).with_context(|| format!("invalid URL '{url}'"))?;
        let host = parsed.host_str().unwrap_or_default();
        if host == "mega.nz" || host.ends_with(".mega.nz") || host == "mega.co.nz" {
            return mega_native::download_mega_file_with_callback(url, dest, progress).await;
        }
        download_file_with_callback(&self.http, &normalize_url(&parsed), dest, None, progress)
            .await
            .map(|_| ())
    }

    /// A download listed in one section of a web page (`page#section`): the
    /// first archive link after the section's anchor.
    pub async fn page_section_candidate(&self, page_url: &str) -> Result<Candidate> {
        let parsed = Url::parse(page_url)?;
        let fragment = parsed
            .fragment()
            .filter(|f| !f.is_empty())
            .with_context(|| format!("{page_url} names no page section"))?
            .to_owned();
        let mut page = parsed.clone();
        page.set_fragment(None);
        let html = self
            .api
            .get(page.clone())
            .send()
            .await?
            .error_for_status()
            .with_context(|| format!("fetching {page}"))?
            .text()
            .await?;
        let link = section_archive_link(&html, &fragment)
            .with_context(|| format!("no archive link in section '{fragment}' of {page}"))?;
        let url = page.join(&link)?;
        let file_name = url
            .path_segments()
            .and_then(|mut s| s.next_back())
            .unwrap_or("download.zip")
            .to_owned();
        Ok(Candidate {
            url: url.into(),
            wrapper: Wrapper::Keep,
            file_name,
        })
    }

    /// The archives to try for a GitLab-hosted entry, best first.
    ///
    /// An explicit archive link is used as it is. A project page resolves to
    /// the project's published build artifact when there is one, else to a
    /// repository archive of its latest release (or default branch).
    pub async fn gitlab_candidates(&self, page_url: &str) -> Result<Vec<Candidate>> {
        let parsed = Url::parse(page_url)?;
        if let Some(explicit) = explicit_archive(&parsed) {
            return Ok(vec![explicit]);
        }
        let project = gitlab_project(&parsed)
            .with_context(|| format!("not a GitLab project address: {page_url}"))?;
        let encoded = project.replace('/', "%2F");
        let name = project.rsplit('/').next().unwrap_or("project").to_owned();

        #[derive(Deserialize)]
        struct Info {
            default_branch: Option<String>,
        }
        #[derive(Deserialize)]
        struct Release {
            tag_name: String,
        }
        let info: Info = self
            .api
            .get(format!("https://gitlab.com/api/v4/projects/{encoded}"))
            .send()
            .await?
            .error_for_status()
            .with_context(|| format!("GitLab project {project}"))?
            .json()
            .await?;
        let branch = info.default_branch.unwrap_or_else(|| "master".into());
        let latest_tag = match self
            .api
            .get(format!(
                "https://gitlab.com/api/v4/projects/{encoded}/releases?per_page=1"
            ))
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => r
                .json::<Vec<Release>>()
                .await
                .ok()
                .and_then(|v| v.into_iter().next())
                .map(|r| r.tag_name),
            _ => None,
        };

        let mut candidates = Vec::new();
        let artifact = format!(
            "https://gitlab.com/api/v4/projects/{encoded}/jobs/artifacts/{branch}/raw/{name}.zip?job=make"
        );
        if self
            .api
            .head(&artifact)
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
        {
            candidates.push(Candidate {
                url: artifact,
                wrapper: Wrapper::Keep,
                file_name: format!("{name}-build.zip"),
            });
        }
        let reference = latest_tag.unwrap_or(branch);
        candidates.push(Candidate {
            url: format!(
                "https://gitlab.com/api/v4/projects/{encoded}/repository/archive.zip?sha={reference}"
            ),
            wrapper: Wrapper::Strip,
            file_name: format!("{name}-{reference}.zip"),
        });
        Ok(candidates)
    }
}

const ARCHIVE_EXTENSIONS: &[&str] = &[".zip", ".7z", ".rar"];

/// The first archive `href` after the element with `id="<fragment>"`.
fn section_archive_link(html: &str, fragment: &str) -> Option<String> {
    let start = ["id=\"", "id='"]
        .iter()
        .filter_map(|prefix| {
            let quote = prefix.chars().last().unwrap();
            html.find(&format!("{prefix}{fragment}{quote}"))
        })
        .min()?;
    let rest = &html[start..];
    let mut cursor = 0;
    while let Some(at) = rest[cursor..].find("href=") {
        let value_start = cursor + at + "href=".len();
        let quote = rest[value_start..].chars().next()?;
        if quote == '"' || quote == '\'' {
            let value = &rest[value_start + 1..];
            let end = value.find(quote)?;
            let href = &value[..end];
            let lower = href.to_lowercase();
            let path = lower.split(['?', '#']).next().unwrap_or("");
            if ARCHIVE_EXTENSIONS.iter().any(|e| path.ends_with(e)) {
                return Some(href.to_owned());
            }
        }
        cursor = value_start + 1;
    }
    None
}

/// Dropbox shows a web page unless asked for the file.
fn normalize_url(url: &Url) -> String {
    if url.host_str().is_some_and(|h| h.ends_with("dropbox.com")) {
        let mut out = url.clone();
        let query: Vec<(String, String)> = url
            .query_pairs()
            .filter(|(k, _)| k != "dl")
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        out.set_query(None);
        out.query_pairs_mut()
            .extend_pairs(query)
            .append_pair("dl", "1");
        return out.into();
    }
    url.as_str().to_owned()
}

/// `https://gitlab.com/ns/project/-/archive/<ref>/project-<ref>.zip`.
fn explicit_archive(url: &Url) -> Option<Candidate> {
    let path = url.path();
    if !path.contains("/-/archive/") && !path.contains("/-/releases/") {
        return None;
    }
    let file_name = path.rsplit('/').next().filter(|n| n.contains('.'))?;
    Some(Candidate {
        url: url.as_str().to_owned(),
        wrapper: Wrapper::Keep,
        file_name: file_name.to_owned(),
    })
}

/// `namespace/project` of a gitlab.com project page or a GitLab Pages site.
pub fn gitlab_project(url: &Url) -> Option<String> {
    let host = url.host_str()?;
    let segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    if let Some(namespace) = host.strip_suffix(".gitlab.io") {
        return Some(format!("{namespace}/{}", segments.first()?));
    }
    if host == "gitlab.com" {
        // Project pages never contain `/-/`; anything after it is a sub-page.
        let end = segments
            .iter()
            .position(|s| *s == "-")
            .unwrap_or(segments.len());
        if end >= 2 {
            return Some(segments[..end].join("/"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(url: &str) -> Option<String> {
        gitlab_project(&Url::parse(url).unwrap())
    }

    #[test]
    fn finds_the_project_behind_gitlab_addresses() {
        assert_eq!(
            project("https://modding-openmw.gitlab.io/sophies-skooma-sweetshoppe/"),
            Some("modding-openmw/sophies-skooma-sweetshoppe".into())
        );
        assert_eq!(
            project("https://gitlab.com/fallchildren/openmw-third-person-alt-attacks"),
            Some("fallchildren/openmw-third-person-alt-attacks".into())
        );
        assert_eq!(
            project("https://gitlab.com/ns/sub/proj/-/issues"),
            Some("ns/sub/proj".into())
        );
        assert_eq!(project("https://example.com/a/b"), None);
    }

    #[test]
    fn explicit_archive_links_are_kept_as_they_are() {
        let url = Url::parse(
            "https://gitlab.com/andrii.gook/inventoryextender/-/archive/public-tweaks/inventoryextender-public-tweaks.zip",
        )
        .unwrap();
        let candidate = explicit_archive(&url).unwrap();
        assert_eq!(candidate.wrapper, Wrapper::Keep);
        assert_eq!(candidate.file_name, "inventoryextender-public-tweaks.zip");
        assert!(explicit_archive(&Url::parse("https://gitlab.com/ns/p").unwrap()).is_none());
    }

    #[test]
    fn page_sections_yield_their_own_archive() {
        let html = r##"<h2 id="imperial-skirt">Imperial Skirt</h2><p>Text <a href="#persistent">x</a>
            <a href="imperial_skirt.zip">download</a></p>
            <h2 id="gondoliers-license">Gondolier</h2>
            <p><a href='gondoliers_license.zip?v=2'>zip</a></p>"##;
        assert_eq!(
            section_archive_link(html, "imperial-skirt").as_deref(),
            Some("imperial_skirt.zip")
        );
        assert_eq!(
            section_archive_link(html, "gondoliers-license").as_deref(),
            Some("gondoliers_license.zip?v=2")
        );
        assert_eq!(section_archive_link(html, "nope"), None);
    }

    #[test]
    fn dropbox_links_ask_for_the_file() {
        let url = Url::parse("https://www.dropbox.com/scl/fi/abc/x.7z?rlkey=k&dl=0").unwrap();
        let out = normalize_url(&url);
        assert!(out.contains("rlkey=k") && out.ends_with("dl=1") && !out.contains("dl=0"));
        let plain = Url::parse("https://modding-openmw.com/files/Mesh Fix v1.2.7z").unwrap();
        assert!(normalize_url(&plain).starts_with("https://modding-openmw.com/files/"));
    }
}
