//! Wire types for the Modding-OpenMW (MOMW) public list API.
//!
//! `GET /api/lists/<slug>` returns an ordered array of [`ModEntry`] and
//! `GET /api/cfg-generator/<slug>` returns a [`CfgBundle`]. Both are plain
//! JSON data published by modding-openmw.com. Unknown fields are ignored so the
//! site can add metadata without breaking older Modyssey builds.

use serde::{Deserialize, Serialize};

/// One mod step of a MOMW list.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ModEntry {
    pub name: String,
    #[serde(default)]
    pub category: String,
    /// Directory the mod is extracted into, relative to its category.
    #[serde(default)]
    pub dir: String,
    #[serde(default)]
    pub slug: String,
    /// Project page. For most mods this is the Nexus mod page.
    #[serde(default)]
    pub url: String,
    /// Site-hosted or third-party direct download link.
    #[serde(default)]
    pub dl_url: String,
    /// Data directories, relative to the category (`<dir>/<subpath>`).
    #[serde(default)]
    pub data_paths: Vec<String>,
    /// Content files (plugins, `.omwscripts`) the mod provides.
    #[serde(default)]
    pub plugins: Vec<String>,
    #[serde(default)]
    pub download_info: Vec<DownloadInfo>,
    #[serde(default)]
    pub on_lists: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DownloadInfo {
    #[serde(default)]
    pub direct_download: Option<String>,
    /// Exact Nexus file display name used to pick the archive.
    #[serde(default)]
    pub file_name: Option<String>,
    /// Extraction target relative to the category directory.
    #[serde(default)]
    pub extract_to: Option<String>,
    #[serde(default)]
    pub nexus_file_id: Option<u64>,
    /// When set the Nexus file must not be advanced along its update chain.
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub actions: Vec<Action>,
}

/// Post-extraction file operations. Paths are relative to the category
/// directory (`<dir>/...`).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum Action {
    Remove {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        paths: Option<Vec<String>>,
    },
    Rename {
        src: String,
        dst: String,
    },
    Copy {
        src: String,
        dst: String,
        #[serde(default)]
        force: Option<bool>,
    },
    /// Run `tes3cmd clean` on a plugin.
    Clean {
        path: String,
        #[serde(default)]
        arguments: Option<StringOrList>,
    },
    /// An action this Modyssey build does not understand. Planning reports it as a
    /// blocker rather than silently skipping a step.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum StringOrList {
    One(String),
    Many(Vec<String>),
}

impl StringOrList {
    pub fn to_vec(&self) -> Vec<String> {
        match self {
            Self::One(s) => s.split_whitespace().map(str::to_owned).collect(),
            Self::Many(v) => v.clone(),
        }
    }
}

/// Server-generated `openmw.cfg` fragments and `settings.cfg` for one list.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct CfgBundle {
    #[serde(default)]
    pub openmw_cfg: OpenmwCfg,
    #[serde(default)]
    pub settings_cfg: String,
}

/// Each field is the multi-line text of one `openmw.cfg` section
/// (`data=...`, `content=...`, ...).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct OpenmwCfg {
    #[serde(default, rename = "fallback values")]
    pub fallback_values: String,
    #[serde(default, rename = "data paths")]
    pub data_paths: String,
    #[serde(default, rename = "fallback archives")]
    pub fallback_archives: String,
    #[serde(default, rename = "content files")]
    pub content_files: String,
    #[serde(default, rename = "groundcover files")]
    pub groundcover_files: String,
}

/// Values of every `key=` line in `text`, in order.
pub fn cfg_values<'a>(text: &'a str, key: &str) -> Vec<&'a str> {
    text.lines()
        .filter_map(|line| {
            let (k, v) = line.split_once('=')?;
            (k.trim() == key).then_some(v.trim_end_matches('\r'))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_actions_and_ignores_unknown_fields() {
        let json = r#"{
            "name": "X", "category": "C", "future_field": 1,
            "download_info": [{
                "file_name": "f", "pinned": true, "extra": [],
                "actions": [
                    {"action": "remove", "path": "a"},
                    {"action": "remove", "paths": ["b", "c"]},
                    {"action": "rename", "src": "s", "dst": "d"},
                    {"action": "copy", "src": "s", "dst": "d", "force": true},
                    {"action": "clean", "path": "p.esp", "arguments": "-x -y"},
                    {"action": "clean", "path": "q.esp", "arguments": ["-z"]},
                    {"action": "teleport", "where": "moon"}
                ]
            }]
        }"#;
        let m: ModEntry = serde_json::from_str(json).unwrap();
        let acts = &m.download_info[0].actions;
        assert_eq!(acts.len(), 7);
        assert!(m.download_info[0].pinned);
        assert!(matches!(acts[6], Action::Unknown));
        match &acts[4] {
            Action::Clean { arguments, .. } => {
                assert_eq!(arguments.as_ref().unwrap().to_vec(), vec!["-x", "-y"])
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn cfg_values_extracts_ordered_values() {
        let text = "data=A\ndata=B C\r\ncontent=X\n\n";
        assert_eq!(cfg_values(text, "data"), vec!["A", "B C"]);
        assert_eq!(cfg_values(text, "content"), vec!["X"]);
        assert!(cfg_values(text, "groundcover").is_empty());
    }
}
