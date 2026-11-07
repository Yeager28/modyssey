//! Turns MOMW list data into an install plan without touching the network or
//! the filesystem.
//!
//! The plan answers three questions:
//! * which archives must be fetched, and from where ([`ArchiveJob`]),
//! * which directories become Fluorine mods, in `openmw.cfg` order
//!   ([`DataEntry`]), and
//! * what else the configuration needs (content/groundcover/fallback archives,
//!   `settings.cfg`, post-install tools).
//!
//! The server-generated `openmw.cfg` is the authority for ordering. Every
//! `data=` line is resolved back to a `(category, data_path)` pair of a mod.

use super::schema::{cfg_values, Action, CfgBundle, DownloadInfo, ModEntry};
use super::source::{ListData, DEFAULT_HOST};
use regex::Regex;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::OnceLock;

/// Base directory of the Windows layout baked into the generated `data=` lines.
const WINDOWS_BASE: &str = "c:/games/openmwmods/";
/// The generated config points the first `data=` at the game's own Data Files.
const GAME_DATA_MARKER: &str = "/moddingresources/morrowind/data files";
/// Content files that ship with the game and are never provided by a mod.
const VANILLA_CONTENT: &[&str] = &["morrowind.esm", "tribunal.esm", "bloodmoon.esm"];
/// Slug of the pseudo-mod whose directory receives generated plugins.
pub const TOOLS_PACK_SLUG: &str = "momw-tools-pack";

/// Lower-cased names of the `Clean_<plugin>` files that argument-less
/// `tes3cmd clean` actions create. Clean actions with arguments (`delete ...`)
/// edit the plugin in place.
fn tes3cmd_clean_outputs(archives: &[ArchiveJob]) -> HashSet<String> {
    archives
        .iter()
        .flat_map(|a| a.actions.iter())
        .filter_map(|act| match act {
            Action::Clean { path, arguments } if arguments.is_none() => {
                let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
                Some(format!("clean_{base}").to_lowercase())
            }
            _ => None,
        })
        .collect()
}

fn is_tool_output(lower_name: &str) -> bool {
    lower_name.contains("delta-merged")
        || lower_name.contains("deleted_groundcover")
        || lower_name.contains("s3lightfixes")
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceKind {
    /// Nexus Mods file, picked by `file_name` (or `file_id` when pinned).
    Nexus {
        domain: String,
        mod_id: u64,
    },
    /// Plain download URL (site-hosted, Mega, Dropbox, web archive, ...).
    Direct {
        url: String,
    },
    /// Release or package of a GitLab project (MOMW tools and patches).
    GitLab {
        url: String,
    },
    GitHub {
        url: String,
    },
    GoogleDrive {
        url: String,
    },
    /// A host CLF3 has no downloader for yet.
    Other {
        url: String,
    },
    /// Entry needs an archive but the list data does not say which.
    Unresolved,
    /// Informational step; nothing to download.
    None,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArchiveJob {
    pub mod_index: usize,
    pub mod_name: String,
    pub category: String,
    pub file_name: Option<String>,
    pub nexus_file_id: Option<u64>,
    pub pinned: bool,
    /// Extraction target relative to the category directory.
    pub extract_to: String,
    pub source: SourceKind,
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DataEntryKind {
    /// The game's own `Data Files` directory.
    GameData,
    Mod {
        mod_index: usize,
        category: String,
        /// Path relative to the category (`<dir>/<subpath>`).
        data_path: String,
    },
    /// A `data=` line no mod accounts for.
    Unmapped { raw: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct DataEntry {
    /// Position among all `data=` lines (lowest wins first, highest overrides).
    pub order: usize,
    pub kind: DataEntryKind,
    /// Unique Fluorine mod folder name for this data directory.
    pub instance_name: String,
    /// Order of the closest data entry whose directory contains this one.
    pub nested_in: Option<usize>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    Tes3cmd,
    DeltaPlugin,
    Groundcoverify,
    S3LightFixes,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanSummary {
    pub mods: usize,
    pub archives: usize,
    pub data_entries: usize,
    pub nested_data_entries: usize,
    pub content_files: usize,
    pub groundcover_files: usize,
    pub sources: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MomwPlan {
    pub slug: String,
    pub momw_version: Option<String>,
    pub archives: Vec<ArchiveJob>,
    pub data_entries: Vec<DataEntry>,
    pub content: Vec<String>,
    pub groundcover: Vec<String>,
    pub fallback_archives: Vec<String>,
    pub fallback_values: Vec<String>,
    pub settings_cfg: String,
    /// Content files created by post-install tools (known tool outputs, plus
    /// anything no mod provides).
    pub generated_content: Vec<String>,
    /// Order of the data entry (the MOMW Tools Pack directory) the tools write
    /// their generated plugins into.
    pub tools_output_entry: Option<usize>,
    pub tools: BTreeSet<Tool>,
    /// Problems that make an install incorrect if ignored.
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
    pub summary: PlanSummary,
}

fn nexus_url_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^https?://(?:www\.)?nexusmods\.com/([^/]+)/mods/(\d+)").expect("nexus regex")
    })
}

/// `(game domain, mod id)` for a Nexus mod page URL.
pub fn parse_nexus_url(url: &str) -> Option<(String, u64)> {
    let caps = nexus_url_re().captures(url)?;
    Some((caps[1].to_owned(), caps[2].parse().ok()?))
}

fn host_of(url: &str) -> &str {
    url.split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or("")
}

/// Decide where one download of `entry` comes from.
pub fn classify_source(entry: &ModEntry, info: &DownloadInfo, site_host: &str) -> SourceKind {
    let has_file =
        info.file_name.as_deref().is_some_and(|f| !f.is_empty()) || info.nexus_file_id.is_some();

    if let Some(url) = info.direct_download.as_deref().filter(|u| !u.is_empty()) {
        return match parse_nexus_url(url) {
            Some((domain, mod_id)) => SourceKind::Nexus { domain, mod_id },
            None => SourceKind::Direct {
                url: url.to_owned(),
            },
        };
    }
    if has_file {
        if let Some((domain, mod_id)) = parse_nexus_url(&entry.url) {
            return SourceKind::Nexus { domain, mod_id };
        }
    }
    if !entry.dl_url.is_empty() {
        let url = if entry.dl_url.starts_with('/') {
            format!("{site_host}{}", entry.dl_url)
        } else {
            entry.dl_url.clone()
        };
        return SourceKind::Direct { url };
    }

    let url = entry.url.as_str();
    let host = host_of(url);
    match host {
        "gitlab.com" | "modding-openmw.gitlab.io" => SourceKind::GitLab {
            url: url.to_owned(),
        },
        "github.com" => SourceKind::GitHub {
            url: url.to_owned(),
        },
        "drive.google.com" => SourceKind::GoogleDrive {
            url: url.to_owned(),
        },
        "mega.nz" | "www.dropbox.com" | "dropbox.com" => SourceKind::Direct {
            url: url.to_owned(),
        },
        _ if parse_nexus_url(url).is_some() => SourceKind::Unresolved,
        _ if has_file => SourceKind::Other {
            url: url.to_owned(),
        },
        _ if entry.data_paths.is_empty() => SourceKind::None,
        _ => SourceKind::Unresolved,
    }
}

fn source_label(kind: &SourceKind) -> &'static str {
    match kind {
        SourceKind::Nexus { .. } => "nexus",
        SourceKind::Direct { .. } => "direct",
        SourceKind::GitLab { .. } => "gitlab",
        SourceKind::GitHub { .. } => "github",
        SourceKind::GoogleDrive { .. } => "google_drive",
        SourceKind::Other { .. } => "other",
        SourceKind::Unresolved => "unresolved",
        SourceKind::None => "none",
    }
}

/// Lower-cased, forward-slash form used as a lookup key.
fn norm(path: &str) -> String {
    path.replace('\\', "/").trim_end_matches('/').to_lowercase()
}

/// Map a `data=` value to a `(category, data_path)` key or the game data dir.
enum Resolved {
    Game,
    Key(String),
    Unmapped,
}

fn resolve_data_line(raw: &str) -> Resolved {
    let n = norm(raw);
    if n.ends_with(GAME_DATA_MARKER) {
        return Resolved::Game;
    }
    match n.strip_prefix(WINDOWS_BASE) {
        Some(rest) => Resolved::Key(rest.to_owned()),
        None => Resolved::Unmapped,
    }
}

fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' => '-',
            ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let name = cleaned.trim().trim_end_matches('.');
    // Never empty: the name becomes a folder under `mods/`, and "" would
    // resolve to `mods/` itself.
    if name.is_empty() { "_" } else { name }.to_owned()
}

/// Build the install plan for `data`.
impl MomwPlan {
    /// Keep only the mods with an archive for which `keep` is true, with their
    /// archives and data directories. For trying part of a list.
    pub fn retain_mods(&mut self, keep: impl Fn(&ArchiveJob) -> bool) {
        let kept: std::collections::HashSet<usize> = self
            .archives
            .iter()
            .filter(|a| keep(a))
            .map(|a| a.mod_index)
            .collect();
        self.archives.retain(|a| kept.contains(&a.mod_index));
        self.data_entries.retain(|e| match &e.kind {
            DataEntryKind::Mod { mod_index, .. } => kept.contains(mod_index),
            _ => false,
        });
    }
}

pub fn build_plan(data: &ListData) -> MomwPlan {
    build_plan_with_host(data, DEFAULT_HOST)
}

pub fn build_plan_with_host(data: &ListData, site_host: &str) -> MomwPlan {
    let ListData {
        slug,
        mods,
        cfg,
        momw_version,
    } = data;
    let mut blockers = Vec::new();
    let mut warnings = Vec::new();

    let archives = build_archives(mods, site_host, &mut blockers, &mut warnings);
    let data_entries = build_data_entries(mods, cfg, &mut blockers);
    if data_entries.is_empty() && mods.iter().any(|m| !m.data_paths.is_empty()) {
        blockers.push(
            "the site returned no generated openmw.cfg for this list (guide-only or work in progress)"
                .to_owned(),
        );
    }

    let content: Vec<String> = cfg_values(&cfg.openmw_cfg.content_files, "content")
        .into_iter()
        .map(str::to_owned)
        .collect();
    let groundcover: Vec<String> = cfg_values(&cfg.openmw_cfg.groundcover_files, "groundcover")
        .into_iter()
        .map(str::to_owned)
        .collect();
    let fallback_archives: Vec<String> =
        cfg_values(&cfg.openmw_cfg.fallback_archives, "fallback-archive")
            .into_iter()
            .map(str::to_owned)
            .collect();
    let fallback_values: Vec<String> = cfg
        .openmw_cfg
        .fallback_values
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_owned)
        .collect();

    // Tool outputs are not downloaded: the Tools Pack advertises its plugins as
    // its own, `tes3cmd clean` writes `Clean_<name>` next to the original, and
    // anything else no mod provides has to come from somewhere too.
    let tools_pack_plugins: HashSet<String> = mods
        .iter()
        .filter(|m| m.slug == TOOLS_PACK_SLUG)
        .flat_map(|m| m.plugins.iter().map(|p| p.to_lowercase()))
        .collect();
    let clean_outputs = tes3cmd_clean_outputs(&archives);
    let provided: HashSet<String> = mods
        .iter()
        .filter(|m| m.slug != TOOLS_PACK_SLUG)
        .flat_map(|m| m.plugins.iter().map(|p| p.to_lowercase()))
        .collect();
    let generated_content: Vec<String> = content
        .iter()
        .chain(groundcover.iter())
        .filter(|c| {
            let l = c.to_lowercase();
            is_tool_output(&l)
                || tools_pack_plugins.contains(&l)
                || clean_outputs.contains(&l)
                || (!provided.contains(&l) && !VANILLA_CONTENT.contains(&l.as_str()))
        })
        .cloned()
        .collect();
    let tools_output_entry = data_entries.iter().find_map(|d| match &d.kind {
        DataEntryKind::Mod { mod_index, .. } if mods[*mod_index].slug == TOOLS_PACK_SLUG => {
            Some(d.order)
        }
        _ => None,
    });

    let mut tools = BTreeSet::new();
    let content_blob = cfg.openmw_cfg.content_files.as_str();
    if content_blob.contains("delta-merged") {
        tools.insert(Tool::DeltaPlugin);
    }
    if content_blob.contains("deleted_groundcover") {
        tools.insert(Tool::Groundcoverify);
    }
    if content_blob.contains("S3LightFixes") {
        tools.insert(Tool::S3LightFixes);
    }
    if archives
        .iter()
        .any(|a| a.actions.iter().any(|x| matches!(x, Action::Clean { .. })))
    {
        tools.insert(Tool::Tes3cmd);
    }

    for g in &generated_content {
        let l = g.to_lowercase();
        if !(is_tool_output(&l) || tools_pack_plugins.contains(&l) || clean_outputs.contains(&l)) {
            warnings.push(format!(
                "content file '{g}' is not provided by any mod and no known tool generates it"
            ));
        }
    }

    let mut sources: BTreeMap<String, usize> = BTreeMap::new();
    for a in &archives {
        *sources
            .entry(source_label(&a.source).to_owned())
            .or_default() += 1;
    }
    let summary = PlanSummary {
        mods: mods.len(),
        archives: archives.len(),
        data_entries: data_entries.len(),
        nested_data_entries: data_entries
            .iter()
            .filter(|d| d.nested_in.is_some())
            .count(),
        content_files: content.len(),
        groundcover_files: groundcover.len(),
        sources,
    };

    MomwPlan {
        slug: slug.clone(),
        momw_version: momw_version.clone(),
        archives,
        data_entries,
        content,
        groundcover,
        fallback_archives,
        fallback_values,
        settings_cfg: cfg.settings_cfg.clone(),
        generated_content,
        tools_output_entry,
        tools,
        blockers,
        warnings,
        summary,
    }
}

fn build_archives(
    mods: &[ModEntry],
    site_host: &str,
    blockers: &mut Vec<String>,
    warnings: &mut Vec<String>,
) -> Vec<ArchiveJob> {
    let mut jobs = Vec::new();
    for (mod_index, entry) in mods.iter().enumerate() {
        for info in &entry.download_info {
            let source = classify_source(entry, info, site_host);
            match &source {
                SourceKind::None => continue,
                SourceKind::Unresolved => blockers.push(format!(
                    "'{}': list data does not identify the archive to download",
                    entry.name
                )),
                SourceKind::Other { url } => {
                    warnings.push(format!("'{}': no downloader for {url}", entry.name))
                }
                _ => {}
            }
            for action in &info.actions {
                if matches!(action, Action::Unknown) {
                    blockers.push(format!("'{}': unsupported post-extract action", entry.name));
                }
            }
            jobs.push(ArchiveJob {
                mod_index,
                mod_name: entry.name.clone(),
                category: entry.category.clone(),
                file_name: info.file_name.clone(),
                nexus_file_id: info.nexus_file_id,
                pinned: info.pinned,
                extract_to: info
                    .extract_to
                    .clone()
                    .filter(|e| !e.is_empty())
                    .unwrap_or_else(|| entry.dir.clone()),
                source,
                actions: info.actions.clone(),
            });
        }
    }
    jobs
}

fn build_data_entries(
    mods: &[ModEntry],
    cfg: &CfgBundle,
    blockers: &mut Vec<String>,
) -> Vec<DataEntry> {
    // (category/data_path) -> mod index
    let mut index: HashMap<String, (usize, &str)> = HashMap::new();
    for (i, m) in mods.iter().enumerate() {
        for dp in &m.data_paths {
            index.insert(norm(&format!("{}/{}", m.category, dp)), (i, dp.as_str()));
        }
    }

    let lines = cfg_values(&cfg.openmw_cfg.data_paths, "data");

    // How many cfg data lines each mod owns decides the naming scheme.
    let mut per_mod: HashMap<usize, usize> = HashMap::new();
    let mut resolved = Vec::with_capacity(lines.len());
    for raw in &lines {
        let r = match resolve_data_line(raw) {
            Resolved::Game => DataEntryKind::GameData,
            Resolved::Key(key) => match index.get(&key) {
                Some(&(mod_index, data_path)) => {
                    *per_mod.entry(mod_index).or_default() += 1;
                    DataEntryKind::Mod {
                        mod_index,
                        category: mods[mod_index].category.clone(),
                        data_path: data_path.to_owned(),
                    }
                }
                None => DataEntryKind::Unmapped {
                    raw: (*raw).to_owned(),
                },
            },
            Resolved::Unmapped => DataEntryKind::Unmapped {
                raw: (*raw).to_owned(),
            },
        };
        if let DataEntryKind::Unmapped { raw } = &r {
            blockers.push(format!("data path not provided by any mod: {raw}"));
        }
        resolved.push(r);
    }

    let mut used: HashSet<String> = HashSet::new();
    let mut entries: Vec<DataEntry> = Vec::with_capacity(resolved.len());
    for (order, kind) in resolved.into_iter().enumerate() {
        let base = match &kind {
            DataEntryKind::GameData => "Morrowind Data Files".to_owned(),
            DataEntryKind::Unmapped { raw } => {
                sanitize(raw.rsplit(['/', '\\']).next().unwrap_or(raw))
            }
            DataEntryKind::Mod {
                mod_index,
                data_path,
                ..
            } => {
                let m = &mods[*mod_index];
                let sub = data_path
                    .strip_prefix(m.dir.as_str())
                    .unwrap_or(data_path)
                    .trim_matches('/');
                if per_mod.get(mod_index).copied().unwrap_or(0) <= 1 || sub.is_empty() {
                    sanitize(&m.name)
                } else {
                    sanitize(&format!("{} - {}", m.name, sub.replace('/', " - ")))
                }
            }
        };
        let mut name = base.clone();
        let mut n = 2;
        while !used.insert(name.to_lowercase()) {
            name = format!("{base} ({n})");
            n += 1;
        }
        entries.push(DataEntry {
            order,
            kind,
            instance_name: name,
            nested_in: None,
        });
    }

    // Nearest containing data entry (same category/dir ancestry only).
    let keys: Vec<Option<String>> = entries
        .iter()
        .map(|e| match &e.kind {
            DataEntryKind::Mod {
                category,
                data_path,
                ..
            } => Some(norm(&format!("{category}/{data_path}"))),
            _ => None,
        })
        .collect();
    let by_key: HashMap<&str, usize> = keys
        .iter()
        .enumerate()
        .filter_map(|(i, k)| k.as_deref().map(|k| (k, i)))
        .collect();
    for i in 0..entries.len() {
        let Some(key) = keys[i].as_deref() else {
            continue;
        };
        let mut cur = key;
        while let Some((parent, _)) = cur.rsplit_once('/') {
            if let Some(&p) = by_key.get(parent) {
                entries[i].nested_in = Some(entries[p].order);
                break;
            }
            cur = parent;
        }
    }
    entries
}

/// The bundled I Heart Vanilla fixture, for tests in sibling modules.
#[cfg(test)]
pub(crate) fn tests_fixture_ihv() -> ListData {
    tests::ihv()
}

#[cfg(test)]
mod tests {
    use super::*;

    const IHV_MODS: &str = include_str!("testdata/i-heart-vanilla.json");
    const IHV_CFG: &str = include_str!("testdata/cfg-i-heart-vanilla.json");

    pub(super) fn ihv() -> ListData {
        ListData {
            slug: "i-heart-vanilla".into(),
            mods: serde_json::from_str(IHV_MODS).unwrap(),
            cfg: serde_json::from_str(IHV_CFG).unwrap(),
            momw_version: None,
        }
    }

    #[test]
    fn parses_nexus_urls() {
        assert_eq!(
            parse_nexus_url("https://www.nexusmods.com/morrowind/mods/43266?tab=files"),
            Some(("morrowind".to_owned(), 43266))
        );
        assert_eq!(parse_nexus_url("https://example.com/mods/1"), None);
    }

    #[test]
    fn real_list_maps_every_data_line() {
        let plan = build_plan(&ihv());
        assert!(
            plan.blockers.is_empty(),
            "unexpected blockers: {:?}",
            plan.blockers
        );
        assert_eq!(plan.data_entries.len(), 49);
        assert_eq!(plan.data_entries[0].kind, DataEntryKind::GameData);
        assert!(plan
            .data_entries
            .iter()
            .skip(1)
            .all(|d| matches!(d.kind, DataEntryKind::Mod { .. })));
        assert_eq!(
            plan.content.first().map(String::as_str),
            Some("Morrowind.esm")
        );
        assert_eq!(plan.fallback_archives.len(), 3);
    }

    #[test]
    fn instance_names_are_unique_and_safe() {
        let plan = build_plan(&ihv());
        let mut seen = HashSet::new();
        for d in &plan.data_entries {
            assert!(
                seen.insert(d.instance_name.to_lowercase()),
                "{}",
                d.instance_name
            );
            assert!(!d.instance_name.is_empty());
            assert!(!d.instance_name.contains(['/', '\\', ':']));
        }
    }

    #[test]
    fn sanitized_names_are_never_empty() {
        for name in ["", ".", "..", "  ", "...", " . "] {
            assert_eq!(sanitize(name), "_", "{name:?}");
        }
        assert_eq!(sanitize("a/b"), "a-b");
        assert_eq!(sanitize("Mod v1."), "Mod v1");
    }

    #[test]
    fn detects_tools_from_generated_content() {
        let plan = build_plan(&ihv());
        assert!(plan.tools.contains(&Tool::S3LightFixes));
        assert!(plan
            .generated_content
            .iter()
            .any(|c| c.eq_ignore_ascii_case("S3LightFixes.omwaddon")));
        assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
        // The tools write into the Tools Pack data directory.
        let out = plan.tools_output_entry.expect("tools pack entry");
        assert_eq!(plan.data_entries[out].instance_name, "MOMW Tools Pack");
    }

    #[test]
    fn nested_data_paths_are_linked_to_their_parent() {
        let mods: Vec<ModEntry> = serde_json::from_str(
            r#"[{"name":"M","category":"C","dir":"M","data_paths":["M","M/000 Core"],
                 "download_info":[{"file_name":"f","extract_to":"M","actions":[]}],
                 "url":"https://www.nexusmods.com/morrowind/mods/1"}]"#,
        )
        .unwrap();
        let cfg = CfgBundle {
            openmw_cfg: super::super::schema::OpenmwCfg {
                data_paths:
                    "data=C:\\games\\OpenMWMods\\C\\M\ndata=C:\\games\\OpenMWMods\\C\\M\\000 Core\n"
                        .into(),
                ..Default::default()
            },
            settings_cfg: String::new(),
        };
        let plan = build_plan(&ListData {
            slug: "t".into(),
            mods,
            cfg,
            momw_version: None,
        });
        assert!(plan.blockers.is_empty());
        assert_eq!(plan.data_entries[0].nested_in, None);
        assert_eq!(plan.data_entries[1].nested_in, Some(0));
        assert_eq!(plan.data_entries[0].instance_name, "M");
        assert_eq!(plan.data_entries[1].instance_name, "M - 000 Core");
    }

    #[test]
    fn unmapped_data_and_unknown_actions_block() {
        let mods: Vec<ModEntry> = serde_json::from_str(
            r#"[{"name":"M","category":"C","dir":"M","data_paths":["M"],
                 "url":"https://www.nexusmods.com/morrowind/mods/1",
                 "download_info":[{"file_name":"f","actions":[{"action":"bogus"}]}]}]"#,
        )
        .unwrap();
        let cfg = CfgBundle {
            openmw_cfg: super::super::schema::OpenmwCfg {
                data_paths: "data=C:\\games\\OpenMWMods\\C\\Ghost\n".into(),
                ..Default::default()
            },
            settings_cfg: String::new(),
        };
        let plan = build_plan(&ListData {
            slug: "t".into(),
            mods,
            cfg,
            momw_version: None,
        });
        assert_eq!(plan.blockers.len(), 2, "{:?}", plan.blockers);
    }

    #[test]
    fn classifies_sources() {
        let entry = |url: &str, dl: &str| -> ModEntry {
            serde_json::from_str(&format!(
                r#"{{"name":"x","url":"{url}","dl_url":"{dl}","data_paths":["x"]}}"#
            ))
            .unwrap()
        };
        let info = |f: Option<&str>| DownloadInfo {
            direct_download: None,
            file_name: f.map(str::to_owned),
            extract_to: None,
            nexus_file_id: None,
            pinned: false,
            actions: vec![],
        };
        let h = DEFAULT_HOST;
        assert!(matches!(
            classify_source(
                &entry("https://www.nexusmods.com/morrowind/mods/9", ""),
                &info(Some("f")),
                h
            ),
            SourceKind::Nexus { mod_id: 9, .. }
        ));
        assert!(matches!(
            classify_source(
                &entry("https://www.nexusmods.com/morrowind/mods/9", ""),
                &info(None),
                h
            ),
            SourceKind::Unresolved
        ));
        assert_eq!(
            classify_source(&entry("https://x.org", "/files/a.7z"), &info(None), h),
            SourceKind::Direct {
                url: format!("{h}/files/a.7z")
            }
        );
        assert!(matches!(
            classify_source(
                &entry("https://modding-openmw.gitlab.io/foo/", ""),
                &info(None),
                h
            ),
            SourceKind::GitLab { .. }
        ));
    }
}
