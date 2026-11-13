//! Turning downloaded archives into the folders a list describes.
//!
//! Everything is first unpacked into a staging tree that mirrors modding-
//! openmw.com's own layout, `<Category>/<extract_to>/...`, because that is
//! what the lists' paths and actions refer to. The tree is then split into one
//! folder per `data=` line, which become the instance's mods.
//!
//! The lists' paths come from a case-insensitive file system, so every lookup
//! here ignores case.

use super::plan::{DataEntry, DataEntryKind, MomwPlan};
use super::schema::Action;
use anyhow::{bail, Context, Result};
use std::path::{Component, Path, PathBuf};

/// Find `relative` below `root`, ignoring case. `None` when a part is missing
/// or the path tries to leave `root`.
pub fn resolve_ci(root: &Path, relative: &str) -> Option<PathBuf> {
    let mut current = root.to_path_buf();
    for part in relative.split(['/', '\\']) {
        match part {
            "" | "." => continue,
            ".." => return None,
            _ => {}
        }
        let exact = current.join(part);
        if exact.symlink_metadata().is_ok() {
            current = exact;
            continue;
        }
        let wanted = part.to_lowercase();
        let found = std::fs::read_dir(&current)
            .ok()?
            .filter_map(Result::ok)
            .find(|e| e.file_name().to_string_lossy().to_lowercase() == wanted)?;
        current = found.path();
    }
    Some(current)
}

/// Like [`resolve_ci`] for a path that may not exist yet: the existing part is
/// matched ignoring case and the rest is appended as written.
fn resolve_ci_parent(root: &Path, relative: &str) -> Result<PathBuf> {
    let parts: Vec<&str> = relative
        .split(['/', '\\'])
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    if parts.contains(&"..") || parts.is_empty() {
        bail!("unsafe path '{relative}'");
    }
    let (last, parents) = parts.split_last().unwrap();
    let parent = resolve_ci(root, &parents.join("/"))
        .with_context(|| format!("no folder for '{relative}'"))?;
    // Reuse an existing entry of a different case instead of making a twin.
    let wanted = last.to_lowercase();
    if let Ok(entries) = std::fs::read_dir(&parent) {
        for e in entries.filter_map(Result::ok) {
            if e.file_name().to_string_lossy().to_lowercase() == wanted {
                return Ok(e.path());
            }
        }
    }
    Ok(parent.join(last))
}

/// The folder `relative` below `root`, matched ignoring case and created when
/// missing, so archives that spell a folder differently land in one place.
pub fn dir_ci_create(root: &Path, relative: &str) -> Result<PathBuf> {
    let mut current = root.to_path_buf();
    for part in relative.split(['/', '\\']) {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            bail!("unsafe path '{relative}'");
        }
        let wanted = part.to_lowercase();
        let existing = std::fs::read_dir(&current).ok().and_then(|rd| {
            rd.filter_map(Result::ok)
                .find(|e| e.file_name().to_string_lossy().to_lowercase() == wanted)
        });
        current = match existing {
            Some(e) => e.path(),
            None => current.join(part),
        };
    }
    std::fs::create_dir_all(&current)?;
    Ok(current)
}

fn remove_any(path: &Path) -> Result<()> {
    if path.is_dir() && !path.is_symlink() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_tree(&entry.path(), &dst.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(src, dst)?;
    }
    Ok(())
}

/// A `tes3cmd` run requested by a list; it needs the game's masters, so the
/// caller runs it once the staging tree is complete.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingClean {
    pub plugin: PathBuf,
    pub arguments: Vec<String>,
}

/// Apply one list action below `category_root`. Returns warnings for things
/// that were already in the wanted state.
pub fn apply_action(
    category_root: &Path,
    action: &Action,
    cleans: &mut Vec<PendingClean>,
) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    match action {
        Action::Remove { path, paths } => {
            for p in path.iter().chain(paths.iter().flatten()) {
                match resolve_ci(category_root, p) {
                    Some(target) if target != category_root => {
                        remove_any(&target).with_context(|| format!("removing '{p}'"))?
                    }
                    _ => warnings.push(format!("remove: '{p}' does not exist")),
                }
            }
        }
        Action::Rename { src, dst } => {
            let from = resolve_ci(category_root, src)
                .with_context(|| format!("rename: '{src}' does not exist"))?;
            let to = resolve_ci_parent(category_root, dst)?;
            if from != to {
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                // Renaming over an existing folder would nest instead.
                if to.exists() && !(to.is_dir() && from.is_dir()) {
                    remove_any(&to)?;
                }
                if to.is_dir() && from.is_dir() {
                    copy_tree(&from, &to)?;
                    remove_any(&from)?;
                } else {
                    std::fs::rename(&from, &to)
                        .with_context(|| format!("renaming '{src}' to '{dst}'"))?;
                }
            }
        }
        Action::Copy { src, dst, force } => {
            let from = resolve_ci(category_root, src)
                .with_context(|| format!("copy: '{src}' does not exist"))?;
            let to = resolve_ci_parent(category_root, dst)?;
            if to.exists() && !force.unwrap_or(false) {
                warnings.push(format!("copy: '{dst}' exists and force is not set"));
            } else {
                copy_tree(&from, &to).with_context(|| format!("copying '{src}' to '{dst}'"))?;
            }
        }
        Action::Clean { path, arguments } => {
            let plugin = resolve_ci(category_root, path)
                .with_context(|| format!("clean: '{path}' does not exist"))?;
            cleans.push(PendingClean {
                plugin,
                arguments: arguments.as_ref().map(|a| a.to_vec()).unwrap_or_default(),
            });
        }
        Action::Unknown => bail!("unsupported list action"),
    }
    Ok(warnings)
}

/// How an archive's own top-level folder is treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wrapper {
    /// Keep the archive layout (every normal download).
    Keep,
    /// A repository archive wraps its files in `<project>-<ref>/`; drop that.
    Strip,
}

/// Unpack `archive` into `dest`.
pub fn extract_into(archive: &Path, dest: &Path, wrapper: Wrapper) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    match wrapper {
        Wrapper::Keep => {
            crate::archive::extract_all(archive, dest)
                .with_context(|| format!("extracting {}", archive.display()))?;
        }
        Wrapper::Strip => {
            let tmp = dest.join(".modyssey-unwrap");
            if tmp.exists() {
                std::fs::remove_dir_all(&tmp)?;
            }
            std::fs::create_dir_all(&tmp)?;
            crate::archive::extract_all(archive, &tmp)
                .with_context(|| format!("extracting {}", archive.display()))?;
            let mut entries: Vec<_> = std::fs::read_dir(&tmp)?.filter_map(Result::ok).collect();
            let source = if entries.len() == 1 && entries[0].path().is_dir() {
                entries.remove(0).path()
            } else {
                tmp.clone()
            };
            merge_into(&source, dest)?;
            std::fs::remove_dir_all(&tmp)?;
        }
    }
    Ok(())
}

/// Move the contents of `from` into `to`, merging folders that already exist.
pub fn merge_into(from: &Path, to: &Path) -> Result<()> {
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.path().is_dir() && target.is_dir() {
            merge_into(&entry.path(), &target)?;
        } else {
            if target.exists() {
                remove_any(&target)?;
            }
            std::fs::rename(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Data directories a mod expects that the staged tree does not contain.
#[derive(Debug, Default)]
pub struct PublishReport {
    pub published: usize,
    pub missing: Vec<String>,
}

fn depth(entry: &DataEntry) -> usize {
    match &entry.kind {
        DataEntryKind::Mod { data_path, .. } => data_path
            .split(['/', '\\'])
            .filter(|p| !p.is_empty())
            .count(),
        _ => 0,
    }
}

/// Move every mod data directory out of `stage_root` into
/// `mods_dir/<instance_name>`.
///
/// A data directory can sit inside another one (a mod root and its
/// sub-folder are both listed). Deeper ones are moved first, so the outer mod
/// ends up without the folder that is its own mod.
pub fn publish_entries(
    plan: &MomwPlan,
    stage_root: &Path,
    mods_dir: &Path,
) -> Result<PublishReport> {
    std::fs::create_dir_all(mods_dir)?;
    let mut entries: Vec<&DataEntry> = plan
        .data_entries
        .iter()
        .filter(|e| matches!(e.kind, DataEntryKind::Mod { .. }))
        .collect();
    entries.sort_by_key(|e| (std::cmp::Reverse(depth(e)), e.order));

    let mut report = PublishReport::default();
    for entry in entries {
        let DataEntryKind::Mod {
            category,
            data_path,
            ..
        } = &entry.kind
        else {
            continue;
        };
        let relative = format!("{category}/{data_path}");
        let Some(source) = resolve_ci(stage_root, &relative).filter(|p| p.is_dir()) else {
            report
                .missing
                .push(format!("{} ({relative})", entry.instance_name));
            continue;
        };
        if matches!(entry.instance_name.as_str(), "" | "." | "..")
            || entry.instance_name.contains(['/', '\\'])
        {
            bail!("unsafe instance folder name {:?}", entry.instance_name);
        }
        let target = mods_dir.join(&entry.instance_name);
        if target.exists() {
            std::fs::remove_dir_all(&target)
                .with_context(|| format!("replacing {}", target.display()))?;
        }
        match std::fs::rename(&source, &target) {
            Ok(()) => {}
            // Staging on another file system: copy, then drop the original.
            Err(_) => {
                copy_tree(&source, &target)?;
                std::fs::remove_dir_all(&source)?;
            }
        }
        report.published += 1;
    }
    Ok(report)
}

/// The component names of a path, for tests and diagnostics.
#[allow(dead_code)]
fn names(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::momw::plan::DataEntry;
    use crate::momw::schema::StringOrList;
    use std::io::Write;

    fn write(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    fn zip_of(path: &Path, files: &[(&str, &str)]) {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for (name, body) in files {
            zip.start_file(*name, options).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn lookups_ignore_case_and_refuse_to_escape() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("Cat/Mod/Data Files/Splash/a.tga"), "x");
        let hit = resolve_ci(dir.path(), "cat\\MOD/data files/splash").unwrap();
        assert!(hit.ends_with("Cat/Mod/Data Files/Splash"));
        assert!(resolve_ci(dir.path(), "Cat/missing").is_none());
        assert!(resolve_ci(dir.path(), "Cat/../etc").is_none());
    }

    #[test]
    fn remove_rename_and_copy_actions() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("M/textures/tr/Ind/a.dds"), "a");
        write(&root.join("M/Data Files/Splash/s.tga"), "s");
        write(&root.join("M/mygui/alt/layout.xml"), "L");
        let mut cleans = Vec::new();

        let removed = Action::Remove {
            path: Some("m/data files/splash".into()),
            paths: Some(vec!["M/nothing".into()]),
        };
        let warnings = apply_action(root, &removed, &mut cleans).unwrap();
        assert!(!root.join("M/Data Files/Splash").exists());
        assert_eq!(warnings.len(), 1);

        let renamed = Action::Rename {
            src: "M/textures/tr/Ind".into(),
            dst: "M/textures/tr/lnd".into(),
        };
        apply_action(root, &renamed, &mut cleans).unwrap();
        assert!(root.join("M/textures/tr/lnd/a.dds").is_file());
        assert!(!root.join("M/textures/tr/Ind").exists());

        let copy = Action::Copy {
            src: "M/mygui/alt/layout.xml".into(),
            dst: "M/mygui/layout.xml".into(),
            force: None,
        };
        apply_action(root, &copy, &mut cleans).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("M/mygui/layout.xml")).unwrap(),
            "L"
        );
        write(&root.join("M/mygui/alt/layout.xml"), "NEW");
        let warnings = apply_action(root, &copy, &mut cleans).unwrap();
        assert_eq!(warnings.len(), 1, "an existing target needs force");
        let forced = Action::Copy {
            src: "M/mygui/alt/layout.xml".into(),
            dst: "M/mygui/layout.xml".into(),
            force: Some(true),
        };
        apply_action(root, &forced, &mut cleans).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("M/mygui/layout.xml")).unwrap(),
            "NEW"
        );
    }

    #[test]
    fn clean_actions_are_queued_not_run() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("E/Plugin.ESP"), "p");
        let mut cleans = Vec::new();
        let action = Action::Clean {
            path: "e/plugin.esp".into(),
            arguments: Some(StringOrList::Many(vec!["delete".into(), "--type".into()])),
        };
        apply_action(dir.path(), &action, &mut cleans).unwrap();
        assert_eq!(cleans.len(), 1);
        assert!(cleans[0].plugin.ends_with("E/Plugin.ESP"));
        assert_eq!(cleans[0].arguments, ["delete", "--type"]);
    }

    #[test]
    fn repository_archives_lose_their_wrapper_folder() {
        let dir = tempfile::tempdir().unwrap();
        let zip = dir.path().join("repo.zip");
        zip_of(
            &zip,
            &[
                ("proj-master/meshes/a.nif", "n"),
                ("proj-master/proj.omwaddon", "p"),
            ],
        );
        let kept = dir.path().join("kept");
        extract_into(&zip, &kept, Wrapper::Keep).unwrap();
        assert!(kept.join("proj-master/proj.omwaddon").is_file());

        let stripped = dir.path().join("stripped");
        extract_into(&zip, &stripped, Wrapper::Strip).unwrap();
        assert!(stripped.join("proj.omwaddon").is_file());
        assert!(stripped.join("meshes/a.nif").is_file());
        assert!(!stripped.join(".modyssey-unwrap").exists());
    }

    fn plan_with(entries: Vec<(usize, &str, &str, &str)>) -> MomwPlan {
        let mut plan = crate::momw::build_plan(&crate::momw::plan::tests_fixture_ihv());
        plan.data_entries = entries
            .into_iter()
            .map(|(order, name, category, data_path)| DataEntry {
                order,
                kind: DataEntryKind::Mod {
                    mod_index: 0,
                    category: category.into(),
                    data_path: data_path.into(),
                },
                instance_name: name.into(),
                nested_in: None,
            })
            .collect();
        plan
    }

    #[test]
    fn nested_data_directories_are_split_children_first() {
        let dir = tempfile::tempdir().unwrap();
        let stage = dir.path().join("stage");
        write(&stage.join("Cat/Mod/root.esp"), "r");
        write(&stage.join("Cat/Mod/meshes/m.nif"), "m");
        write(&stage.join("Cat/Mod/Patch/patch.esp"), "p");
        write(&stage.join("Cat/Other/o.esp"), "o");
        // Parent listed before its child, like a list would.
        let plan = plan_with(vec![
            (0, "Mod", "Cat", "Mod"),
            (1, "Mod - Patch", "cat", "mod/PATCH"),
            (2, "Other", "Cat", "Other"),
            (3, "Lost", "Cat", "Gone"),
        ]);
        let mods = dir.path().join("mods");
        let report = publish_entries(&plan, &stage, &mods).unwrap();

        assert_eq!(report.published, 3);
        assert_eq!(report.missing.len(), 1);
        assert!(report.missing[0].contains("Lost"));
        assert!(mods.join("Mod - Patch/patch.esp").is_file());
        assert!(mods.join("Mod/root.esp").is_file());
        assert!(mods.join("Mod/meshes/m.nif").is_file());
        assert!(!mods.join("Mod/Patch").exists(), "the child is its own mod");
        assert!(mods.join("Other/o.esp").is_file());
        assert_eq!(names(Path::new("/a/b")), ["a", "b"]);
    }
}
