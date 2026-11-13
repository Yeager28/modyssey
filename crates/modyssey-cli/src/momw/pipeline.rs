//! The whole install: plan, resolve, download, unpack, clean, split into mods,
//! bundle OpenMW, run the generators, write the instance and validate it.

use super::fetch::{Fetcher, NexusAccess};
use super::install::{stage_archives, Event as StageEvent, EventSink as StageSink, StageOptions};
use super::instance::{write_instance, write_profile_cfg, InstanceReport, InstanceSpec};
use super::nexus::{resolve_nexus_archives, NexusFileLister, Outcome};
use super::openmw::{self, Release};
use super::plan::{DataEntryKind, MomwPlan};
use super::stage::{publish_entries, PendingClean};
use super::tools::{self, GeneratorPlan, ToolSet};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct InstallRequest {
    /// Instance folder to create.
    pub root: PathBuf,
    /// The Morrowind installation (contains `Data Files`).
    pub game_dir: PathBuf,
    /// Where the MOMW tools are (downloaded when missing).
    pub tools_dir: PathBuf,
    pub downloads_dir: PathBuf,
    pub concurrency: usize,
    /// The OpenMW this instance runs with, already in the shared store.
    pub openmw: Release,
    /// Merged plugins, groundcover and light fixes.
    pub generators: bool,
    pub validate: bool,
    /// Navigation meshes, built by the bundled OpenMW.
    pub navmesh: bool,
    /// Add an application-menu entry that starts the instance without Fluorine.
    pub shortcut: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Resolving,
    Downloading,
    Cleaning,
    Splitting,
    Bundling,
    Tools,
    Writing,
    Validating,
    Navmesh,
}

impl Phase {
    pub fn label(&self) -> &'static str {
        match self {
            Phase::Resolving => "Resolving Nexus files",
            Phase::Downloading => "Downloading and unpacking mods",
            Phase::Cleaning => "Cleaning plugins",
            Phase::Splitting => "Creating mods",
            Phase::Bundling => "Setting up OpenMW",
            Phase::Tools => "Generating merged plugins",
            Phase::Writing => "Writing the instance",
            Phase::Validating => "Validating",
            Phase::Navmesh => "Building navigation meshes",
        }
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Phase(Phase),
    Stage(StageEvent),
    /// A line of output from a tool.
    Tool(String),
    Note(String),
}

pub type EventSink = Arc<dyn Fn(Event) + Send + Sync>;

#[derive(Debug, Default)]
pub struct InstallReport {
    pub instance: InstanceReport,
    pub warnings: Vec<String>,
    /// The validator found problems (the instance is still written).
    pub validator_problems: Option<String>,
}

fn note(sink: &EventSink, warnings: &mut Vec<String>, message: String) {
    sink(Event::Note(message.clone()));
    warnings.push(message);
}

/// Run every `tes3cmd` action the list asks for.
async fn run_cleans(
    tools: &ToolSet,
    cleans: &[PendingClean],
    stage_dir: &Path,
    game_dir: &Path,
    cancel: &CancellationToken,
    sink: &EventSink,
) -> Result<()> {
    let backup_root = stage_dir.join("tes3cmd-backups");
    let data_files = game_dir.join("Data Files");
    for clean in cleans {
        let command = tools::tes3cmd_command(
            tools,
            &clean.plugin,
            &clean.arguments,
            &backup_root,
            &data_files,
        );
        let on_line = |line: &str| sink(Event::Tool(line.to_owned()));
        tools::run(&command, cancel, &on_line)
            .await
            .with_context(|| format!("cleaning {}", clean.plugin.display()))?;
    }
    Ok(())
}

pub fn check_game_dir(game_dir: &Path) -> Result<()> {
    if !game_dir.join("Data Files").is_dir() {
        bail!(
            "{} is not a Morrowind installation (no Data Files folder)",
            game_dir.display()
        );
    }
    Ok(())
}

pub async fn run_install(
    plan: &MomwPlan,
    request: &InstallRequest,
    access: NexusAccess,
    lister: &dyn NexusFileLister,
    cancel: CancellationToken,
    sink: EventSink,
) -> Result<InstallReport> {
    if !plan.blockers.is_empty() {
        bail!(
            "the list cannot be installed yet:\n  {}",
            plan.blockers.join("\n  ")
        );
    }
    check_game_dir(&request.game_dir)?;
    std::fs::create_dir_all(&request.root)?;

    sink(Event::Phase(Phase::Resolving));
    let resolved = resolve_nexus_archives(plan, lister, 4).await;
    let unresolved: Vec<String> = resolved
        .iter()
        .filter_map(|r| match &r.outcome {
            Outcome::Failed { error } => Some(format!("{}: {error}", r.mod_name)),
            _ => None,
        })
        .collect();
    if !unresolved.is_empty() {
        bail!(
            "{} Nexus file(s) could not be found:\n  {}",
            unresolved.len(),
            unresolved.join("\n  ")
        );
    }

    sink(Event::Phase(Phase::Downloading));
    let fetcher = Fetcher::new(access, cancel.clone())?;
    let stage_dir = request.root.join(".stage");
    let options = StageOptions {
        downloads_dir: request.downloads_dir.clone(),
        stage_dir: stage_dir.clone(),
        concurrency: request.concurrency,
    };
    let stage_sink: StageSink = {
        let sink = sink.clone();
        Arc::new(move |e| sink(Event::Stage(e)))
    };
    let outcome = stage_archives(plan, &resolved, &fetcher, &options, &stage_sink).await;
    if !outcome.failures.is_empty() {
        bail!(
            "{} mod(s) could not be downloaded or unpacked:\n  {}",
            outcome.failures.len(),
            outcome
                .failures
                .iter()
                .map(|(i, why)| format!("{}: {why}", plan.archives[*i].mod_name))
                .collect::<Vec<_>>()
                .join("\n  ")
        );
    }
    let mut report = InstallReport::default();
    report.warnings.extend(outcome.warnings.iter().cloned());

    let needs_tools = request.generators || request.validate || !outcome.cleans.is_empty();
    let tool_set = if needs_tools {
        Some(tools::provision(&request.tools_dir, false).await?)
    } else {
        None
    };

    if !outcome.cleans.is_empty() {
        sink(Event::Phase(Phase::Cleaning));
        run_cleans(
            tool_set.as_ref().expect("tools were provisioned"),
            &outcome.cleans,
            &stage_dir,
            &request.game_dir,
            &cancel,
            &sink,
        )
        .await?;
    }

    sink(Event::Phase(Phase::Splitting));
    let published = publish_entries(plan, &stage_dir, &request.root.join("mods"))?;
    if !published.missing.is_empty() {
        bail!(
            "the downloads do not contain the expected folders:\n  {}",
            published.missing.join("\n  ")
        );
    }
    let _ = std::fs::remove_dir_all(&stage_dir);

    finish_instance(
        plan,
        request,
        tool_set.as_ref(),
        &cancel,
        &sink,
        &mut report,
    )
    .await?;
    Ok(report)
}

/// Everything after the mods exist: bundle OpenMW, write the config, run the
/// generators, record the instance, validate and build navigation meshes.
/// Also used to bring an existing instance up to date.
pub async fn finish_instance(
    plan: &MomwPlan,
    request: &InstallRequest,
    tool_set: Option<&ToolSet>,
    cancel: &CancellationToken,
    sink: &EventSink,
    report: &mut InstallReport,
) -> Result<()> {
    let spec = InstanceSpec {
        root: &request.root,
        game_dir: &request.game_dir,
        plan,
    };
    let on_line = {
        let sink = sink.clone();
        move |line: &str| sink(Event::Tool(line.to_owned()))
    };

    sink(Event::Phase(Phase::Bundling));
    openmw::bundle(&request.root, &request.openmw)?;
    if request.shortcut {
        // A menu entry is a convenience; failing to write one never fails the install.
        let title = request
            .root
            .file_name()
            .map_or_else(|| plan.slug.clone(), |n| n.to_string_lossy().into_owned());
        let written = openmw::default_applications_dir()
            .and_then(|dir| openmw::install_desktop_entry(&request.root, &title, &dir));
        if let Err(e) = written {
            sink(Event::Tool(format!("could not add a menu entry: {e:#}")));
        }
    }
    let tools_dir = request.root.join("momw-tools");
    std::fs::create_dir_all(&tools_dir)?;
    // The vanilla settings come from the player's own Morrowind.ini, read by
    // the bundled importer, so nothing is taken from a system OpenMW config.
    let imported = tools_dir.join("imported.cfg");
    let _ = std::fs::remove_file(&imported);
    let ini = request.game_dir.join("Morrowind.ini");
    let imported_text = if ini.is_file() {
        let command = tools::iniimporter_command(
            &openmw::program(&request.root, "openmw-iniimporter"),
            &ini,
            &imported,
        );
        tools::run(&command, cancel, &on_line).await?;
        std::fs::read_to_string(&imported).unwrap_or_default()
    } else {
        note(
            sink,
            &mut report.warnings,
            "Morrowind.ini was not found; vanilla settings use OpenMW's defaults".into(),
        );
        String::new()
    };
    let cfg_path = write_profile_cfg(&spec, &imported_text)?;

    if request.generators {
        let tool_set = tool_set.context("the MOMW tools were not set up")?;
        let generators = GeneratorPlan::from_tools(&plan.tools);
        if generators.delta_plugin || generators.groundcoverify || generators.light_fixes {
            sink(Event::Phase(Phase::Tools));
            let output = tools_output_dir(plan, &request.root)?;
            tools::run_generators(
                tool_set,
                &generators,
                &cfg_path,
                &output,
                &tools_dir,
                cancel,
                &on_line,
            )
            .await?;
        }
    }

    sink(Event::Phase(Phase::Writing));
    report.instance = write_instance(&spec)?;
    for name in &report.instance.missing_content {
        note(
            sink,
            &mut report.warnings,
            format!("the list loads {name}, but no mod provides it"),
        );
    }
    write_instance_record(plan, request)?;

    if request.validate {
        sink(Event::Phase(Phase::Validating));
        let tool_set = tool_set.context("the MOMW tools were not set up")?;
        let validator = tools::validator_command(tool_set, &cfg_path);
        if let Err(e) = tools::run(&validator, cancel, &on_line).await {
            report.validator_problems = Some(format!("{e:#}"));
        }
    }

    if request.navmesh {
        sink(Event::Phase(Phase::Navmesh));
        let profile = request.root.join("profiles").join("Default");
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get());
        let command = tools::navmeshtool_command(
            &openmw::program(&request.root, "openmw-navmeshtool"),
            &profile,
            &profile,
            threads,
        );
        tools::run(&command, cancel, &on_line).await?;
    }
    Ok(())
}

/// The MOMW version recorded when the instance was installed, if known.
pub fn installed_list_version(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join("momw-instance.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get("momw_version")?.as_str().map(str::to_owned)
}

/// `momw-instance.json`: which list and which OpenMW this instance is, so an
/// update can tell when either has changed.
fn write_instance_record(plan: &MomwPlan, request: &InstallRequest) -> Result<()> {
    let installed_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let record = serde_json::json!({
        "list": plan.slug,
        "momw_version": plan.momw_version,
        "openmw_version": request.openmw.version,
        "updated_at": installed_at,
    });
    std::fs::write(
        request.root.join("momw-instance.json"),
        serde_json::to_vec_pretty(&record)?,
    )?;
    Ok(())
}

fn tools_output_dir(plan: &MomwPlan, root: &Path) -> Result<PathBuf> {
    let order = plan
        .tools_output_entry
        .context("the list has no MOMW Tools Pack folder to hold generated files")?;
    let entry = plan
        .data_entries
        .iter()
        .find(|e| e.order == order && matches!(e.kind, DataEntryKind::Mod { .. }))
        .context("the tools folder is not a mod")?;
    Ok(root.join("mods").join(&entry.instance_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_installed_list_version_comes_from_the_instance_record() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(installed_list_version(dir.path()), None);
        std::fs::write(
            dir.path().join("momw-instance.json"),
            r#"{"list":"x","momw_version":"9.2.2","openmw_version":"0.51.0"}"#,
        )
        .unwrap();
        assert_eq!(installed_list_version(dir.path()).as_deref(), Some("9.2.2"));
        std::fs::write(dir.path().join("momw-instance.json"), "not json").unwrap();
        assert_eq!(installed_list_version(dir.path()), None);
    }
}
