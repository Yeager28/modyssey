//! Known games registry
//!
//! The game registry is data-driven: the shipped defaults live in
//! `data/games.json` (embedded via `include_str!`) and users can extend or
//! override them with an overlay file at
//! `%LOCALAPPDATA%/modyssey/games-overlay.json` (Windows;
//          `~/.local/share/modyssey/` on Linux — `dirs::data_local_dir()`).
//! Load order: embedded defaults first, then overlay entries applied
//! additively — an overlay entry replaces the default entry with the same
//! `steam_app_id` or the same `name` (case-insensitive); anything else is
//! appended.
//!
//! Per-game metadata:
//! - Steam App ID
//! - GOG App ID (if available)
//! - My Games folder name (Documents/My Games/*)
//! - AppData/Local + AppData/Roaming folder names
//! - Registry path/value for game detection
//! - steamapps/common folder name
//! - Wabbajack `GameType` string

use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use tracing::warn;

/// Current registry schema version. A file with a different
/// `schema_version` is rejected (the embedded copy panics — a programming
/// error caught by tests; an overlay copy is ignored with a warning).
pub const GAMES_SCHEMA_VERSION: u32 = 1;

/// Configuration for a known game
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnownGame {
    /// Display name
    pub name: String,
    /// Steam App ID
    pub steam_app_id: String,
    /// GOG App ID (if available)
    pub gog_app_id: Option<String>,
    /// Folder name in Documents/My Games (if applicable)
    pub my_games_folder: Option<String>,
    /// Folder name in AppData/Local (if applicable)
    pub appdata_local_folder: Option<String>,
    /// Folder name in AppData/Roaming (if applicable)
    pub appdata_roaming_folder: Option<String>,
    /// Registry path under HKLM\Software\ (for game detection)
    pub registry_path: String,
    /// Registry value name for install path
    pub registry_value: String,
    /// Expected folder name in steamapps/common/
    pub steam_folder: String,
    /// Wabbajack `GameType` string from modlist JSON (e.g. "FalloutNewVegas").
    /// None for store variants that don't have their own Wabbajack enum entry.
    pub wabbajack_type: Option<String>,
}

/// Wrapper shared by the embedded registry and user overlays.
#[derive(Debug, Deserialize)]
struct GamesFile {
    schema_version: u32,
    #[serde(default)]
    games: Vec<KnownGame>,
}

/// The merged game registry, built once on first use: embedded
/// `data/games.json` + user overlay (if present).
static REGISTRY: LazyLock<Vec<KnownGame>> = LazyLock::new(|| {
    let base = parse_registry(include_str!("../../../data/games.json"))
        .expect("embedded data/games.json must be valid (guarded by tests)");
    match load_overlay() {
        OverlayOutcome::None => base,
        OverlayOutcome::Entries(entries) => merge_registry(base, &entries),
    }
});

/// All known games that Modyssey supports (embedded defaults merged with the
/// user overlay).
pub fn games() -> &'static [KnownGame] {
    &REGISTRY
}

/// Where the overlay file lives:
/// `%LOCALAPPDATA%/modyssey/games-overlay.json` on Windows.
pub fn overlay_path() -> Option<std::path::PathBuf> {
    dirs::data_local_dir().map(|d| d.join("modyssey").join("games-overlay.json"))
}

enum OverlayOutcome {
    /// No overlay file, or it is unusable (already warned).
    None,
    Entries(Vec<KnownGame>),
}

fn load_overlay() -> OverlayOutcome {
    let Some(path) = overlay_path() else {
        return OverlayOutcome::None;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return OverlayOutcome::None; // optional by design
    };
    match serde_json::from_str::<GamesFile>(&text) {
        Ok(file) if file.schema_version == GAMES_SCHEMA_VERSION => {
            OverlayOutcome::Entries(file.games)
        }
        Ok(file) => {
            warn!(
                "Ignoring {} with schema_version {} (supported: {})",
                path.display(),
                file.schema_version,
                GAMES_SCHEMA_VERSION
            );
            OverlayOutcome::None
        }
        Err(e) => {
            warn!(
                "Ignoring malformed game registry overlay {}: {}",
                path.display(),
                e
            );
            OverlayOutcome::None
        }
    }
}

/// Parse a registry file. Any schema version other than the current one is
/// an error (the caller decides whether that is fatal).
fn parse_registry(text: &str) -> Result<Vec<KnownGame>, String> {
    let file: GamesFile =
        serde_json::from_str(text).map_err(|e| format!("invalid games registry: {e}"))?;
    if file.schema_version != GAMES_SCHEMA_VERSION {
        return Err(format!(
            "unsupported games schema_version {} (expected {})",
            file.schema_version, GAMES_SCHEMA_VERSION
        ));
    }
    Ok(file.games)
}

/// Apply overlay entries to the embedded defaults: an entry replaces the
/// default with the same `steam_app_id` or the same `name`
/// (case-insensitive); unmatched entries are appended in order.
fn merge_registry(mut base: Vec<KnownGame>, overlay: &[KnownGame]) -> Vec<KnownGame> {
    for entry in overlay {
        let existing = base.iter().position(|g| {
            g.steam_app_id == entry.steam_app_id
                || g.name.to_lowercase() == entry.name.to_lowercase()
        });
        match existing {
            Some(idx) => base[idx] = entry.clone(),
            None => base.push(entry.clone()),
        }
    }
    base
}

/// Find a known game by Steam App ID
pub fn find_by_steam_id(app_id: &str) -> Option<&'static KnownGame> {
    games().iter().find(|g| g.steam_app_id == app_id)
}

/// Find a known game by GOG App ID
pub fn find_by_gog_id(app_id: &str) -> Option<&'static KnownGame> {
    games()
        .iter()
        .find(|g| g.gog_app_id.as_deref() == Some(app_id))
}

/// Find a known game by name (case-insensitive)
pub fn find_by_name(name: &str) -> Option<&'static KnownGame> {
    let name_lower = name.to_lowercase();
    games().iter().find(|g| g.name.to_lowercase() == name_lower)
}

/// Find a known game by its Wabbajack `GameType` string (e.g. "FalloutNewVegas",
/// "SkyrimSpecialEdition"). Accepts common aliases Wabbajack has used across versions.
pub fn find_by_wabbajack_type(wj_type: &str) -> Option<&'static KnownGame> {
    // Direct match against the canonical wabbajack_type field
    if let Some(g) = games()
        .iter()
        .find(|g| g.wabbajack_type.as_deref() == Some(wj_type))
    {
        return Some(g);
    }

    // Alias fallback — older modlists / short-form names
    let aliased = match wj_type {
        "SkyrimSE" => "SkyrimSpecialEdition",
        "FalloutNV" => "FalloutNewVegas",
        "EnderalSE" => "EnderalSpecialEdition",
        "TheWitcher3" => "Witcher3",
        _ => return None,
    };
    games()
        .iter()
        .find(|g| g.wabbajack_type.as_deref() == Some(aliased))
}

/// Convenience: return `(steam_app_id, gog_app_id)` for a Wabbajack game_type.
/// GOG id is `None` for games without a known GOG variant.
///
/// Returns only the *canonical* entry; for installs that come in store
/// variants (Fallout 3 vs. Fallout 3 GOTY both shipped on Steam under
/// different app IDs), use [`variants_for_wabbajack_type`] which returns
/// every install candidate.
pub fn ids_for_wabbajack_type(wj_type: &str) -> Option<(String, Option<String>)> {
    let g = find_by_wabbajack_type(wj_type)?;
    Some((g.steam_app_id.clone(), g.gog_app_id.clone()))
}

/// Return every known `KnownGame` entry that maps to the same Wabbajack
/// `game_type`, ordered with the canonical entry first.
///
/// A store variant is recognised as `wabbajack_type: None` (i.e. Wabbajack
/// doesn't have its own enum for it) plus a shared `registry_path` with the
/// canonical entry — Bethesda variants use the same registry key regardless
/// of the Steam app ID, so this catches Fallout 3 GOTY (22370) when the
/// modlist asks for "Fallout3" (22300) and vice-versa.
pub fn variants_for_wabbajack_type(wj_type: &str) -> Vec<&'static KnownGame> {
    let Some(canonical) = find_by_wabbajack_type(wj_type) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(2);
    out.push(canonical);
    for g in games() {
        if g.wabbajack_type.is_none() && g.registry_path == canonical.registry_path {
            out.push(g);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_registry_parses() {
        let games = parse_registry(include_str!("../../../data/games.json"))
            .expect("embedded registry must parse");
        assert!(!games.is_empty());
    }

    #[test]
    fn registry_has_18_shipped_games() {
        assert_eq!(games().len(), 18, "shipped games.json entry count changed");
    }

    #[test]
    fn dark_souls_ii_entry_is_exact() {
        let g = find_by_steam_id("335300").expect("DS2 SOTFS entry");
        assert_eq!(g.name, "Dark Souls II: Scholar of the First Sin");
        assert_eq!(g.steam_app_id, "335300");
        assert_eq!(g.gog_app_id, None);
        assert_eq!(g.my_games_folder.as_deref(), Some("NBGI\\DarkSoulsII"));
        assert_eq!(
            g.registry_path,
            r"Software\Namco Bandai Games\Dark Souls II"
        );
        assert_eq!(g.registry_value, "InstallPath");
        assert_eq!(g.steam_folder, "Dark Souls II Scholar of the First Sin");
        assert_eq!(g.wabbajack_type.as_deref(), Some("DarkSoulsII"));
        assert_eq!(find_by_wabbajack_type("DarkSoulsII"), Some(g));
    }

    #[test]
    fn fallout3_resolves_to_both_steam_variants() {
        let v = variants_for_wabbajack_type("Fallout3");
        let ids: Vec<&str> = v.iter().map(|g| g.steam_app_id.as_str()).collect();
        assert!(
            ids.contains(&"22300") && ids.contains(&"22370"),
            "expected Fallout3 lookup to include both 22300 and 22370; got {:?}",
            ids
        );
        // Canonical entry first.
        assert_eq!(v[0].steam_app_id, "22300");
    }

    #[test]
    fn variants_handles_aliases() {
        let v = variants_for_wabbajack_type("SkyrimSE");
        assert!(!v.is_empty(), "alias 'SkyrimSE' should resolve");
        assert_eq!(v[0].steam_app_id, "489830");
    }

    #[test]
    fn unknown_type_returns_empty() {
        assert!(variants_for_wabbajack_type("NotAGame").is_empty());
    }

    #[test]
    fn overlay_overrides_by_steam_id_and_name() {
        let base = parse_registry(include_str!("../../../data/games.json")).unwrap();
        let mut overridden = base[0].clone();
        overridden.steam_folder = "Custom Folder".to_string();
        let added = KnownGame {
            name: "My Custom Game".to_string(),
            steam_app_id: "999999".to_string(),
            gog_app_id: None,
            my_games_folder: None,
            appdata_local_folder: None,
            appdata_roaming_folder: None,
            registry_path: r"Software\Custom\Game".to_string(),
            registry_value: "InstallPath".to_string(),
            steam_folder: "CustomGame".to_string(),
            wabbajack_type: Some("MyCustomGame".to_string()),
        };
        let by_name = KnownGame {
            name: base[1].name.clone(), // same name, different steam id
            steam_app_id: "888888".to_string(),
            ..added.clone()
        };
        let merged = merge_registry(base.clone(), &[overridden, added.clone(), by_name]);

        assert_eq!(merged.len(), base.len() + 1, "one entry appended");
        // Override by steam_app_id replaced in place.
        assert_eq!(merged[0].steam_folder, "Custom Folder");
        assert_eq!(merged[0].steam_app_id, base[0].steam_app_id);
        // Override by name replaced the second entry (same position).
        assert_eq!(merged[1].steam_app_id, "888888");
        // The unmatched entry was appended and is findable.
        assert_eq!(find_in(&merged, "999999").name, "My Custom Game");
    }

    fn find_in<'a>(games: &'a [KnownGame], steam_id: &str) -> &'a KnownGame {
        games.iter().find(|g| g.steam_app_id == steam_id).unwrap()
    }
}
