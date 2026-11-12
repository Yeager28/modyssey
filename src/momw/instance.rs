//! Writing the Fluorine instance for an installed list.
//!
//! The instance is a portable Mod Organizer folder for the "Morrowind
//! (OpenMW)" game. Mods are the per-data-directory folders made by
//! `stage::publish_entries`; the load order, groundcover and archive choices
//! go into Fluorine's own selection state so its first launch does not enable
//! plugins the list leaves out.

use super::plan::{DataEntryKind, MomwPlan};
use anyhow::{bail, Context, Result};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const GAME_NAME: &str = "Morrowind (OpenMW)";
const PROFILE: &str = "Default";
const STATE_FILE: &str = "fluorine-openmw-selection.json";
const PLUGIN_EXTENSIONS: &[&str] = &[".esp", ".esm", ".omwaddon", ".omwgame", ".omwscripts"];
const VANILLA_BSAS: &[&str] = &["Morrowind.bsa", "Tribunal.bsa", "Bloodmoon.bsa"];
/// Only the base game is forced to the top. Tribunal and Bloodmoon keep the
/// place the list gives them: lists load other masters between them (Distant
/// Seafloor must come before Bloodmoon.esm or Solstheim's land is missing).
const VANILLA_MASTERS: &[&str] = &["Morrowind.esm"];

/// The content list as Fluorine will load it: the list's own order, with the
/// base game first and each name once.
pub fn ordered_content(plan: &MomwPlan) -> Vec<String> {
    let mut seen = HashSet::new();
    VANILLA_MASTERS
        .iter()
        .map(|s| s.to_string())
        .chain(plan.content.iter().cloned())
        .filter(|name| seen.insert(name.to_lowercase()))
        .collect()
}

pub struct InstanceSpec<'a> {
    /// Instance root; must already hold `mods/`.
    pub root: &'a Path,
    /// The Morrowind installation (the folder containing `Data Files`).
    pub game_dir: &'a Path,
    pub plan: &'a MomwPlan,
}

#[derive(Debug, Default)]
pub struct InstanceReport {
    pub mods: usize,
    pub plugins_enabled: usize,
    /// Plugins found in mod folders that the list does not use.
    pub plugins_left_off: usize,
    pub archives_enabled: usize,
    /// Plugins the list loads that no mod folder provides.
    pub missing_content: Vec<String>,
    pub missing_groundcover: Vec<String>,
}

/// Plugins and archives physically present, with the case they are spelled in
/// (the highest-priority provider decides, like the game does).
#[derive(Debug, Default)]
struct Inventory {
    plugins: HashMap<String, String>,
    archives: Vec<String>,
}

fn has_extension(name: &str, extensions: &[&str]) -> bool {
    let lower = name.to_lowercase();
    extensions.iter().any(|e| lower.ends_with(e))
}

fn scan_root(dir: &Path, inventory: &mut Inventory, seen_archives: &mut HashSet<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    files.sort_by_key(|n| n.to_lowercase());
    for name in files {
        if has_extension(&name, PLUGIN_EXTENSIONS) {
            inventory.plugins.insert(name.to_lowercase(), name);
        } else if has_extension(&name, &[".bsa"]) && seen_archives.insert(name.to_lowercase()) {
            inventory.archives.push(name);
        }
    }
}

/// Folders in load order: the game's `Data Files`, then every mod folder.
fn data_dirs(spec: &InstanceSpec) -> Vec<PathBuf> {
    let mods = spec.root.join("mods");
    let mut dirs = vec![spec.game_dir.join("Data Files")];
    let mut entries: Vec<_> = spec
        .plan
        .data_entries
        .iter()
        .filter(|e| matches!(e.kind, DataEntryKind::Mod { .. }))
        .collect();
    entries.sort_by_key(|e| e.order);
    dirs.extend(entries.iter().map(|e| mods.join(&e.instance_name)));
    dirs
}

fn scan(spec: &InstanceSpec) -> Inventory {
    let mut inventory = Inventory::default();
    let mut seen = HashSet::new();
    for dir in data_dirs(spec) {
        scan_root(&dir, &mut inventory, &mut seen);
    }
    inventory
}

/// The version-3 selection state Fluorine's OpenMW plugin reads.
fn selection_state(plan: &MomwPlan, inventory: &Inventory) -> serde_json::Value {
    let spelled = |name: &str| {
        inventory
            .plugins
            .get(&name.to_lowercase())
            .cloned()
            .unwrap_or_else(|| name.to_owned())
    };
    let mut order: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut push = |name: String, order: &mut Vec<String>| {
        if seen.insert(name.to_lowercase()) {
            order.push(name);
        }
    };
    for name in ordered_content(plan).iter().chain(plan.groundcover.iter()) {
        push(spelled(name), &mut order);
    }
    let used: HashSet<String> = order.iter().map(|n| n.to_lowercase()).collect();
    let mut left_off: Vec<&String> = inventory
        .plugins
        .iter()
        .filter(|(key, _)| !used.contains(*key))
        .map(|(_, name)| name)
        .collect();
    left_off.sort_by_key(|n| n.to_lowercase());
    let left_off: Vec<String> = left_off.into_iter().cloned().collect();

    let enabled = order.clone();
    for name in left_off {
        push(name, &mut order);
    }
    let groundcover: Vec<String> = plan.groundcover.iter().map(|n| spelled(n)).collect();

    // Archives the list names are loaded; any other .bsa a mod ships is known
    // but left off, so it is not switched on at the first launch.
    let mut archives: Vec<String> = Vec::new();
    let mut archive_keys: HashSet<String> = HashSet::new();
    for name in VANILLA_BSAS
        .iter()
        .map(|s| s.to_string())
        .chain(plan.fallback_archives.iter().cloned())
    {
        let name = inventory
            .archives
            .iter()
            .find(|a| a.eq_ignore_ascii_case(&name))
            .cloned()
            .unwrap_or(name);
        if archive_keys.insert(name.to_lowercase()) {
            archives.push(name);
        }
    }
    let mut known = archives.clone();
    known.extend(
        inventory
            .archives
            .iter()
            .filter(|a| !archive_keys.contains(&a.to_lowercase()))
            .cloned(),
    );

    json!({
        "version": 3,
        "plugin_order": order,
        "enabled_plugins": enabled,
        "groundcover": groundcover,
        "known_archives": known,
        "archives": archives,
        "profile_config_entries": [],
        "profile_config_entries_known": true,
        "profile_config_terminal": false,
        "content_migration_source": "momw-list",
        "groundcover_migration_source": "momw-list",
        "archive_migration_source": "momw-list",
        "order_migration_source": "momw-list",
        "plugin_state_migrated": true,
    })
}

/// `data="..."` with OpenMW's quoting: `&` and `"` get a leading `&`.
pub fn quote_data_path(path: &str) -> String {
    let mut out = String::from("\"");
    for c in path.chars() {
        if c == '&' || c == '"' {
            out.push('&');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// The lines Fluorine manages in the profile's `openmw.cfg`, in its order:
/// data folders, content, groundcover, archives. Fluorine rewrites exactly
/// these at launch, so writing them here gives every other program (the
/// tools, navmesh generation, the standalone launcher) the same load order.
pub fn managed_lines(spec: &InstanceSpec) -> Vec<String> {
    let mut out = Vec::new();
    for dir in data_dirs(spec) {
        out.push(format!("data={}", quote_data_path(&dir.to_string_lossy())));
    }
    let mut seen = HashSet::new();
    for name in ordered_content(spec.plan) {
        if seen.insert(name.to_lowercase()) {
            out.push(format!("content={name}"));
        }
    }
    for name in &spec.plan.groundcover {
        out.push(format!("groundcover={name}"));
    }
    let mut seen = HashSet::new();
    for archive in VANILLA_BSAS
        .iter()
        .map(|s| s.to_string())
        .chain(spec.plan.fallback_archives.iter().cloned())
    {
        if seen.insert(archive.to_lowercase()) {
            out.push(format!("fallback-archive={archive}"));
        }
    }
    out
}

/// Keys the importer writes that this module owns instead.
fn is_owned_key(line: &str) -> bool {
    let key = line.split('=').next().unwrap_or("").trim().to_lowercase();
    matches!(
        key.as_str(),
        "data"
            | "content"
            | "groundcover"
            | "fallback-archive"
            | "config"
            | "user-data"
            | "replace"
    )
}

/// Write `profiles/Default/openmw.cfg`: the vanilla settings from the ini
/// importer (`imported`), the list's own `fallback=` lines, then the managed
/// block. This is the instance's one OpenMW config folder.
pub fn write_profile_cfg(spec: &InstanceSpec, imported: &str) -> Result<PathBuf> {
    let profile = spec.root.join("profiles").join(PROFILE);
    std::fs::create_dir_all(&profile)?;
    let mut out = String::new();
    for line in imported.lines().filter(|l| !is_owned_key(l)) {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(&format!(
        "\n# MOMW list: {}{}\n",
        spec.plan.slug,
        spec.plan
            .momw_version
            .as_deref()
            .map(|v| format!(" (MOMW {v})"))
            .unwrap_or_default()
    ));
    for line in &spec.plan.fallback_values {
        out.push_str(line);
        out.push('\n');
    }
    out.push('\n');
    for line in managed_lines(spec) {
        out.push_str(&line);
        out.push('\n');
    }
    let path = profile.join("openmw.cfg");
    std::fs::write(&path, out)?;
    Ok(path)
}

fn ini_value(path: &Path) -> Result<String> {
    let text = path.to_str().context("non-UTF-8 instance path")?;
    // QSettings treats backslashes and quotes specially in values.
    if text.contains(['\n', '\r', '\\', '"']) {
        bail!("unsupported character in path {text}");
    }
    Ok(text.to_owned())
}

fn modlist_text(plan: &MomwPlan) -> String {
    let mut names: Vec<(usize, &str)> = plan
        .data_entries
        .iter()
        .filter(|e| matches!(e.kind, DataEntryKind::Mod { .. }))
        .map(|e| (e.order, e.instance_name.as_str()))
        .collect();
    names.sort();
    // Mod Organizer lists the highest priority first.
    let mut out = String::from("# This file was automatically generated by DS2 Modpack by Bonkfyre.\n");
    for (_, name) in names.into_iter().rev() {
        out.push_str(&format!("+{name}\n"));
    }
    out
}

/// Create everything but `mods/`, which the staging step filled.
pub fn write_instance(spec: &InstanceSpec) -> Result<InstanceReport> {
    let root = spec.root;
    let mods = root.join("mods");
    if !mods.is_dir() {
        bail!("{} has no mods folder", root.display());
    }
    if !spec.game_dir.join("Data Files").is_dir() {
        bail!(
            "{} is not a Morrowind installation (no Data Files folder)",
            spec.game_dir.display()
        );
    }
    let profile = root.join("profiles").join(PROFILE);
    std::fs::create_dir_all(&profile)?;
    for dir in ["overwrite", "downloads"] {
        std::fs::create_dir_all(root.join(dir))?;
    }

    let base = ini_value(root)?;
    let game = ini_value(spec.game_dir)?;
    std::fs::write(
        root.join("ModOrganizer.ini"),
        format!(
            "[General]\ngameName={GAME_NAME}\ngamePath={game}\nselected_profile={PROFILE}\n\
             [Settings]\nbase_directory={base}\nmod_directory={base}/mods\n\
             profiles_directory={base}/profiles\ndownload_directory={base}/downloads\n\
             overwrite_directory={base}/overwrite\n"
        ),
    )?;
    std::fs::write(root.join("portable.txt"), "")?;

    std::fs::write(profile.join("modlist.txt"), modlist_text(spec.plan))?;
    std::fs::write(
        profile.join("settings.ini"),
        "[General]\nLocalSaves=true\nLocalSettings=true\n",
    )?;
    if !spec.plan.settings_cfg.trim().is_empty() {
        std::fs::write(profile.join("settings.cfg"), &spec.plan.settings_cfg)?;
    }
    install_shaders_yaml(spec, &profile)?;

    let inventory = scan(spec);
    let state = selection_state(spec.plan, &inventory);
    std::fs::write(profile.join(STATE_FILE), serde_json::to_vec_pretty(&state)?)?;

    let have = |name: &String| inventory.plugins.contains_key(&name.to_lowercase());
    let enabled = state["enabled_plugins"].as_array().map_or(0, Vec::len);
    let total = state["plugin_order"].as_array().map_or(0, Vec::len);
    Ok(InstanceReport {
        mods: spec
            .plan
            .data_entries
            .iter()
            .filter(|e| matches!(e.kind, DataEntryKind::Mod { .. }))
            .count(),
        plugins_enabled: enabled,
        plugins_left_off: total - enabled,
        archives_enabled: state["archives"].as_array().map_or(0, Vec::len),
        missing_content: spec
            .plan
            .content
            .iter()
            .filter(|n| !have(n))
            .cloned()
            .collect(),
        missing_groundcover: spec
            .plan
            .groundcover
            .iter()
            .filter(|n| !have(n))
            .cloned()
            .collect(),
    })
}

/// The post-processing pack ships a recommended `shaders.yaml`; OpenMW reads
/// it from the config folder, which is the profile.
fn install_shaders_yaml(spec: &InstanceSpec, profile: &Path) -> Result<()> {
    let mods = spec.root.join("mods");
    for entry in &spec.plan.data_entries {
        let DataEntryKind::Mod { data_path, .. } = &entry.kind else {
            continue;
        };
        if !data_path.to_lowercase().contains("momwpostprocessingpack") {
            continue;
        }
        let candidate = mods.join(&entry.instance_name).join("shaders.yaml");
        if candidate.is_file() {
            std::fs::copy(candidate, profile.join("shaders.yaml"))?;
            return Ok(());
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::momw::plan::DataEntry;

    fn entry(order: usize, name: &str) -> DataEntry {
        DataEntry {
            order,
            kind: DataEntryKind::Mod {
                mod_index: 0,
                category: "Cat".into(),
                data_path: name.into(),
            },
            instance_name: name.into(),
            nested_in: None,
        }
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "x").unwrap();
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
        game: PathBuf,
        plan: MomwPlan,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("inst");
        let game = dir.path().join("Morrowind");
        touch(&game.join("Data Files/Morrowind.esm"));
        touch(&game.join("Data Files/Morrowind.bsa"));
        touch(&root.join("mods/Base/Base.esm"));
        touch(&root.join("mods/Base/Optional.ESP"));
        touch(&root.join("mods/Patch/patch.esp"));
        touch(&root.join("mods/Patch/Extra.bsa"));
        touch(&root.join("mods/Grass/grass.omwaddon"));
        let mut plan = crate::momw::build_plan(&crate::momw::plan::tests_fixture_ihv());
        plan.data_entries = vec![entry(0, "Base"), entry(1, "Patch"), entry(2, "Grass")];
        plan.content = vec![
            "Morrowind.esm".into(),
            "base.esm".into(),
            "Patch.esp".into(),
        ];
        plan.groundcover = vec!["Grass.omwaddon".into()];
        plan.fallback_archives = vec!["Morrowind.bsa".into()];
        plan.fallback_values = vec!["fallback=Weather_Clear_Cloud_Texture,tx_sky_clear.dds".into()];
        plan.settings_cfg = "[GUI]\nscaling factor = 1.5\n".into();
        Fixture {
            _dir: dir,
            root,
            game,
            plan,
        }
    }

    #[test]
    fn state_enables_only_what_the_list_names() {
        let f = fixture();
        let spec = InstanceSpec {
            root: &f.root,
            game_dir: &f.game,
            plan: &f.plan,
        };
        let report = write_instance(&spec).unwrap();
        let state: serde_json::Value = serde_json::from_slice(
            &std::fs::read(f.root.join("profiles/Default").join(STATE_FILE)).unwrap(),
        )
        .unwrap();

        // Spelling follows the files on disk, not the list.
        assert_eq!(
            state["plugin_order"],
            json!([
                "Morrowind.esm",
                "Base.esm",
                "patch.esp",
                "grass.omwaddon",
                "Optional.ESP"
            ])
        );
        assert_eq!(
            state["enabled_plugins"],
            json!(["Morrowind.esm", "Base.esm", "patch.esp", "grass.omwaddon"])
        );
        assert_eq!(state["groundcover"], json!(["grass.omwaddon"]));
        assert_eq!(
            state["archives"],
            json!(["Morrowind.bsa", "Tribunal.bsa", "Bloodmoon.bsa"])
        );
        // A shipped archive the list does not use is known but not enabled.
        assert!(state["known_archives"]
            .as_array()
            .unwrap()
            .contains(&json!("Extra.bsa")));
        assert_eq!(report.plugins_left_off, 1);
        assert!(report.missing_content.is_empty());
        // Lets the Fluorine side check this exact file with its own validator.
        if let Ok(out) = std::env::var("DSMODPACK_TEST_STATE_OUT") {
            std::fs::copy(f.root.join("profiles/Default").join(STATE_FILE), out).unwrap();
        }
    }

    #[test]
    fn missing_content_is_reported() {
        let mut f = fixture();
        f.plan.content.push("Clean_Missing.ESP".into());
        let spec = InstanceSpec {
            root: &f.root,
            game_dir: &f.game,
            plan: &f.plan,
        };
        let report = write_instance(&spec).unwrap();
        assert_eq!(report.missing_content, ["Clean_Missing.ESP"]);
    }

    #[test]
    fn writes_the_profile_and_instance_files() {
        let f = fixture();
        let spec = InstanceSpec {
            root: &f.root,
            game_dir: &f.game,
            plan: &f.plan,
        };
        write_instance(&spec).unwrap();
        let ini = std::fs::read_to_string(f.root.join("ModOrganizer.ini")).unwrap();
        assert!(ini.contains("gameName=Morrowind (OpenMW)"));
        assert!(ini.contains(&format!("gamePath={}", f.game.display())));
        let profile = f.root.join("profiles/Default");
        // Highest priority first.
        assert_eq!(
            std::fs::read_to_string(profile.join("modlist.txt")).unwrap(),
            "# This file was automatically generated by DS2 Modpack by Bonkfyre.\n+Grass\n+Patch\n+Base\n"
        );
        assert!(std::fs::read_to_string(profile.join("settings.ini"))
            .unwrap()
            .contains("LocalSettings=true"));
        assert!(std::fs::read_to_string(profile.join("settings.cfg"))
            .unwrap()
            .contains("scaling factor"));
    }

    #[test]
    fn the_profile_config_holds_everything_openmw_needs() {
        let f = fixture();
        let spec = InstanceSpec {
            root: &f.root,
            game_dir: &f.game,
            plan: &f.plan,
        };
        let imported = "encoding=win1252\nfallback=Foo,bar\nfallback-archive=Morrowind.bsa\n\
                        content=Morrowind.esm\ndata=\"/stale\"\nno-sound=0\n";
        let path = write_profile_cfg(&spec, imported).unwrap();
        let cfg = std::fs::read_to_string(path).unwrap();
        let lines: Vec<&str> = cfg.lines().collect();

        // Importer settings survive; the keys this module owns are rebuilt.
        assert!(lines.contains(&"encoding=win1252") && lines.contains(&"no-sound=0"));
        assert!(lines.contains(&"fallback=Foo,bar"));
        assert!(!cfg.contains("/stale"));
        assert_eq!(
            lines
                .iter()
                .filter(|l| l.starts_with("fallback-archive="))
                .count(),
            3
        );
        assert!(cfg.contains("fallback=Weather_Clear_Cloud_Texture"));

        let data: Vec<&&str> = lines.iter().filter(|l| l.starts_with("data=")).collect();
        assert_eq!(data.len(), 4);
        assert!(data[0].ends_with("Data Files\""));
        assert!(data[1].contains("/mods/Base\""));
        assert!(data[3].contains("/mods/Grass\""));
        // The list's order, with the base game first.
        assert!(cfg.contains("content=Morrowind.esm\ncontent=base.esm\ncontent=Patch.esp\n"));
        assert!(cfg.contains("groundcover=Grass.omwaddon\n"));
        // The managed block comes last, after the settings it sits beside.
        assert!(cfg.find("fallback=Foo").unwrap() < cfg.find("data=").unwrap());
        assert_eq!(quote_data_path("a&b\"c"), "\"a&&b&\"c\"");
    }

    /// MOMW lists load Distant Seafloor between Tribunal and Bloodmoon. Forcing
    /// the vanilla masters together breaks Solstheim's landscape.
    #[test]
    fn a_plugin_between_the_expansions_keeps_its_place() {
        let mut f = fixture();
        f.plan.content = [
            "Morrowind.esm",
            "Tribunal.esm",
            "distant_seafloor_2.00.esm",
            "Bloodmoon.esm",
            "Patch.esp",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            ordered_content(&f.plan),
            [
                "Morrowind.esm",
                "Tribunal.esm",
                "distant_seafloor_2.00.esm",
                "Bloodmoon.esm",
                "Patch.esp"
            ]
        );
        let spec = InstanceSpec {
            root: &f.root,
            game_dir: &f.game,
            plan: &f.plan,
        };
        let content: Vec<String> = managed_lines(&spec)
            .into_iter()
            .filter(|l| l.starts_with("content="))
            .collect();
        assert_eq!(
            content,
            [
                "content=Morrowind.esm",
                "content=Tribunal.esm",
                "content=distant_seafloor_2.00.esm",
                "content=Bloodmoon.esm",
                "content=Patch.esp"
            ]
        );
        // A list that forgot the base game still gets it first.
        f.plan.content = vec!["Other.esp".into()];
        assert_eq!(ordered_content(&f.plan), ["Morrowind.esm", "Other.esp"]);
    }

    #[test]
    fn refuses_a_game_folder_without_data_files() {
        let f = fixture();
        let empty = f.root.join("not-morrowind");
        std::fs::create_dir_all(&empty).unwrap();
        let spec = InstanceSpec {
            root: &f.root,
            game_dir: &empty,
            plan: &f.plan,
        };
        assert!(write_instance(&spec).is_err());
    }
}
