//! Fetching list metadata from modding-openmw.com.

use super::schema::{CfgBundle, ModEntry};
use anyhow::{bail, Context, Result};
use std::path::Path;
use std::time::Duration;

pub const DEFAULT_HOST: &str = "https://modding-openmw.com";
pub const BETA_HOST: &str = "https://beta.modding-openmw.com";

/// Host selection mirrors the knobs the official tools expose
/// (`MODDING_OPENMW_HOST`, `--beta`, `--momw-url`).
#[derive(Debug, Clone)]
pub struct SourceOptions {
    pub host: String,
    /// Ask the site for OpenMW dev-build configuration (`?dev=on`).
    pub dev: bool,
}

impl SourceOptions {
    pub fn new(beta: bool, host_override: Option<String>, dev: bool) -> Self {
        let host = host_override
            .or_else(|| std::env::var("MODDING_OPENMW_HOST").ok())
            .unwrap_or_else(|| if beta { BETA_HOST } else { DEFAULT_HOST }.to_owned());
        Self {
            host: host.trim_end_matches('/').to_owned(),
            dev,
        }
    }
}

impl Default for SourceOptions {
    fn default() -> Self {
        Self::new(false, None, false)
    }
}

/// A list's mods and generated configuration, as published by the site.
#[derive(Debug, Clone)]
pub struct ListData {
    pub slug: String,
    pub mods: Vec<ModEntry>,
    pub cfg: CfgBundle,
    /// `MOMW-Version` response header of the cfg generator, if any.
    pub momw_version: Option<String>,
}

pub struct MomwClient {
    http: reqwest::Client,
    options: SourceOptions,
}

/// Lists the API serves that the site does not advertise on its lists page:
/// a retired predecessor (its page redirects to Morrowind Starter Pack) and an
/// experimental list. The API carries no flag for this, so it is spelled out.
const UNADVERTISED: &[&str] = &["one-day-morrowind-modernization", "morrowind-pbr"];

/// Whether modding-openmw.com lists this slug on its public lists page.
/// Work-in-progress lists (`*-wip`) and the few retired or experimental ones
/// are still installable, but are only offered when asked for.
pub fn is_advertised(slug: &str) -> bool {
    !slug.ends_with("-wip") && !UNADVERTISED.contains(&slug)
}

impl MomwClient {
    pub fn new(options: SourceOptions) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(format!(
                "modyssey/{} (+https://github.com/Yeager28/modyssey)",
                env!("CARGO_PKG_VERSION")
            ))
            .timeout(Duration::from_secs(60))
            .build()
            .context("building HTTP client")?;
        Ok(Self { http, options })
    }

    pub fn host(&self) -> &str {
        &self.options.host
    }

    async fn get(&self, path: &str) -> Result<reqwest::Response> {
        let url = format!("{}{}", self.options.host, path);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("requesting {url}"))?;
        if !resp.status().is_success() {
            bail!("{url} returned HTTP {}", resp.status());
        }
        Ok(resp)
    }

    /// Slugs of every list the API serves, including work-in-progress ones.
    pub async fn list_slugs(&self) -> Result<Vec<String>> {
        self.get("/api/lists")
            .await?
            .json()
            .await
            .context("parsing /api/lists")
    }

    pub async fn fetch_list(&self, slug: &str) -> Result<ListData> {
        let slugs = self.list_slugs().await?;
        if !slugs.iter().any(|s| s == slug) {
            bail!(
                "unknown list '{slug}'; available lists: {}",
                slugs.join(", ")
            );
        }
        let mods: Vec<ModEntry> = self
            .get(&format!("/api/lists/{slug}"))
            .await?
            .json()
            .await
            .with_context(|| format!("parsing mods of list {slug}"))?;

        let cfg_path = if self.options.dev {
            format!("/api/cfg-generator/{slug}?dev=on")
        } else {
            format!("/api/cfg-generator/{slug}")
        };
        let cfg_resp = self.get(&cfg_path).await?;
        let momw_version = cfg_resp
            .headers()
            .get("momw-version")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let cfg: CfgBundle = cfg_resp
            .json()
            .await
            .with_context(|| format!("parsing cfg of list {slug}"))?;

        Ok(ListData {
            slug: slug.to_owned(),
            mods,
            cfg,
            momw_version,
        })
    }
}

/// Load list data from local JSON files (offline use, tests and fixtures).
pub fn load_list_files(slug: &str, mods_json: &Path, cfg_json: &Path) -> Result<ListData> {
    let mods = serde_json::from_slice(
        &std::fs::read(mods_json).with_context(|| format!("reading {}", mods_json.display()))?,
    )
    .with_context(|| format!("parsing {}", mods_json.display()))?;
    let cfg = serde_json::from_slice(
        &std::fs::read(cfg_json).with_context(|| format!("reading {}", cfg_json.display()))?,
    )
    .with_context(|| format!("parsing {}", cfg_json.display()))?;
    Ok(ListData {
        slug: slug.to_owned(),
        mods,
        cfg,
        momw_version: None,
    })
}

#[cfg(test)]
mod tests {
    use super::is_advertised;

    #[test]
    fn hides_wip_and_unadvertised_lists() {
        for slug in [
            "total-overhaul",
            "i-heart-vanilla-directors-cut",
            "starwind-modded",
        ] {
            assert!(is_advertised(slug), "{slug}");
        }
        for slug in [
            "total-overhaul-wip",
            "arktwend-enhanced-wip",
            "morrowind-pbr",
            "one-day-morrowind-modernization",
        ] {
            assert!(!is_advertised(slug), "{slug}");
        }
    }
}
