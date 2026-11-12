use super::plan::{build_plan_with_host, MomwPlan};
use super::source::{is_advertised, load_list_files, ListData, MomwClient, SourceOptions};
use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Args)]
pub struct MomwArgs {
    #[command(subcommand)]
    pub command: MomwCommand,

    /// Use beta.modding-openmw.com.
    #[arg(long, global = true)]
    pub beta: bool,

    /// Use another MOMW server (also `MODDING_OPENMW_HOST`).
    #[arg(long, global = true)]
    pub momw_url: Option<String>,

    /// Request configuration for OpenMW dev builds.
    #[arg(long, global = true)]
    pub dev: bool,
}

#[derive(Debug, Subcommand)]
pub enum MomwCommand {
    /// List the mod lists published by modding-openmw.com.
    Lists {
        /// Also show work-in-progress and unadvertised lists.
        #[arg(long)]
        all: bool,
    },
    /// Plan a list install: archives, data directories, tools and blockers.
    ///
    /// Reads only public site data; no credentials are needed.
    Plan {
        /// List slug, e.g. `total-overhaul`.
        slug: String,
        /// Print the complete plan as JSON instead of a summary.
        #[arg(long)]
        json: bool,
        /// Read list mods from a local JSON file instead of the network.
        #[arg(long, requires = "cfg_file")]
        mods_file: Option<PathBuf>,
        /// Read generated cfg from a local JSON file instead of the network.
        #[arg(long, requires = "mods_file")]
        cfg_file: Option<PathBuf>,
        /// Exit unsuccessfully when the plan has blockers.
        #[arg(long)]
        strict: bool,
    },
    /// Add an installed instance folder to Fluorine's instance list.
    Register {
        /// The instance folder (it holds `ModOrganizer.ini`).
        dir: PathBuf,
    },
    /// Download the MOMW Tools Pack binaries the post-install steps use.
    Tools {
        /// Where to unpack them (default: CLF3's data directory).
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Download again even if the tools are already present.
        #[arg(long)]
        update: bool,
        /// Also download OpenMW (a ~540 MB portable release, checked against
        /// its published checksum) into the shared store.
        #[arg(long)]
        openmw: bool,
        /// Which OpenMW to download (default: the newest stable release).
        #[arg(long)]
        openmw_version: Option<String>,
    },
    /// Give an installed instance its own OpenMW, vanilla settings, launch
    /// script and navigation meshes, without downloading the mods again.
    Bundle {
        /// List slug the instance was installed from.
        slug: String,
        /// The instance folder.
        #[arg(long)]
        dir: PathBuf,
        /// Your Morrowind folder (found automatically when omitted).
        #[arg(long)]
        game_dir: Option<PathBuf>,
        /// Which OpenMW to bundle (default: the newest stable release).
        #[arg(long)]
        openmw_version: Option<String>,
        /// Skip building navigation meshes.
        #[arg(long)]
        no_navmesh: bool,
        /// Check the result with openmw-validator.
        #[arg(long)]
        validate: bool,
    },
    /// Run the post-install generators on an existing `openmw.cfg`.
    ///
    /// Writes delta-merged, groundcover and light-fix plugins into
    /// `--output-dir`, which must be one of the cfg's `data=` directories.
    RunTools {
        /// The `openmw.cfg` listing every data directory and plugin.
        #[arg(long)]
        cfg: PathBuf,
        /// The data directory that receives the generated files.
        #[arg(long)]
        output_dir: PathBuf,
        /// Tools directory (default: CLF3's data directory).
        #[arg(long)]
        tools_dir: Option<PathBuf>,
        /// Directory for the tools' own settings and derived configs.
        #[arg(long)]
        config_dir: PathBuf,
        /// Merge plugins with delta_plugin.
        #[arg(long)]
        delta_plugin: bool,
        /// Generate groundcover with GoCoverify.
        #[arg(long)]
        groundcover: bool,
        /// Generate light fixes with S3LightFixes.
        #[arg(long)]
        light_fixes: bool,
        /// Finish with openmw-validator.
        #[arg(long)]
        validate: bool,
    },
    /// Download and unpack a list into one folder per data directory.
    ///
    /// For checking sources: it stages and splits the mods but does not write
    /// an instance. Nexus files need your saved API key (Premium).
    Stage {
        /// List slug, e.g. `total-overhaul`.
        slug: String,
        /// Output directory; gets `mods/`, `downloads/` and a temporary stage.
        #[arg(long)]
        out: PathBuf,
        /// Keep archives here instead of `<out>/downloads`.
        #[arg(long)]
        downloads: Option<PathBuf>,
        /// Mods processed in parallel.
        #[arg(long, default_value_t = 4)]
        concurrency: usize,
        /// Skip every mod that comes from Nexus.
        #[arg(long)]
        skip_nexus: bool,
        /// Only mods whose name contains this text (repeatable).
        #[arg(long)]
        only: Vec<String>,
    },
    /// Install a list as a Fluorine instance: download, unpack, run the MOMW
    /// tools and write the profile.
    ///
    /// Nexus files use your saved API key (Premium); Fluorine's installer
    /// window is the way to install without one.
    Install {
        /// List slug, e.g. `total-overhaul`.
        slug: String,
        /// The instance folder to create.
        #[arg(long)]
        dir: PathBuf,
        /// Your Morrowind folder (the one holding `Data Files`); found
        /// automatically when omitted.
        #[arg(long)]
        game_dir: Option<PathBuf>,
        /// Keep archives here (default: one shared folder in CLF3's data
        /// directory, used by every list).
        #[arg(long)]
        downloads: Option<PathBuf>,
        /// Tools directory (default: CLF3's data directory).
        #[arg(long)]
        tools_dir: Option<PathBuf>,
        /// Mods processed in parallel.
        #[arg(long, default_value_t = 4)]
        concurrency: usize,
        /// Do not run the MOMW generators (merged plugins, groundcover, lights).
        #[arg(long)]
        skip_tools: bool,
        /// Check the result with openmw-validator.
        #[arg(long)]
        validate: bool,
        /// Which OpenMW to bundle with the instance (default: the newest
        /// stable release, which is what MOMW's lists require).
        #[arg(long)]
        openmw_version: Option<String>,
        /// Driven by Fluorine: JSON events on stdout, Nexus handled by the host.
        #[arg(long)]
        hosted: bool,
        /// Do not add the instance to Fluorine's instance list.
        #[arg(long)]
        no_register: bool,
        /// Skip building navigation meshes (they are built by default, which
        /// takes a while but avoids long hitches in game).
        #[arg(long)]
        no_navmesh: bool,
        /// For testing: only mods whose name contains this text. The result
        /// is an incomplete instance.
        #[arg(long)]
        only: Vec<String>,
    },
    /// Resolve the Nexus files a list needs, using your saved Nexus API key.
    ///
    /// Reads one `files.json` per Nexus mod (cached for a day) and reports the
    /// file each entry resolves to, so problems show up before any download.
    Resolve {
        /// List slug, e.g. `total-overhaul`.
        slug: String,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
        /// Parallel Nexus requests.
        #[arg(long, default_value_t = 4)]
        concurrency: usize,
        /// Ignore cached file lists.
        #[arg(long)]
        no_cache: bool,
    },
}

pub async fn run(args: &MomwArgs) -> Result<()> {
    let options = SourceOptions::new(args.beta, args.momw_url.clone(), args.dev);
    match &args.command {
        MomwCommand::Lists { all } => {
            let client = MomwClient::new(options)?;
            for slug in client.list_slugs().await? {
                if *all || is_advertised(&slug) {
                    println!("{slug}");
                }
            }
        }
        MomwCommand::Plan {
            slug,
            json,
            mods_file,
            cfg_file,
            strict,
        } => {
            let host = options.host.clone();
            let data: ListData = match (mods_file, cfg_file) {
                (Some(m), Some(c)) => load_list_files(slug, m, c)?,
                _ => MomwClient::new(options)?.fetch_list(slug).await?,
            };
            let plan = build_plan_with_host(&data, &host);
            if *json {
                let mut out = std::io::stdout().lock();
                serde_json::to_writer_pretty(&mut out, &plan)?;
                writeln!(out)?;
            } else {
                print_summary(&plan);
            }
            if *strict && !plan.blockers.is_empty() {
                bail!("plan has {} blocker(s)", plan.blockers.len());
            }
        }
        MomwCommand::Register { dir } => {
            if !dir.join("ModOrganizer.ini").is_file() {
                bail!(
                    "{} is not an instance folder (no ModOrganizer.ini)",
                    dir.display()
                );
            }
            crate::fluorine::register_portable_instance(dir, false)?;
            println!("Added {} to Fluorine's instances", dir.display());
        }
        MomwCommand::Tools {
            dir,
            update,
            openmw,
            openmw_version,
        } => {
            let dir = match dir {
                Some(dir) => dir.clone(),
                None => super::tools::default_tools_dir()?,
            };
            let tools = super::tools::provision(&dir, *update).await?;
            println!("Tools directory: {}", tools.dir.display());
            print!("{}", tools.versions());
            if *openmw {
                let release = super::openmw::ensure_release(
                    &super::openmw::default_store_dir()?,
                    &super::openmw::Channel::from_arg(openmw_version.as_deref()),
                )
                .await?;
                println!("OpenMW {}: {}", release.version, release.dir.display());
            }
        }
        MomwCommand::RunTools {
            cfg,
            output_dir,
            tools_dir,
            config_dir,
            delta_plugin,
            groundcover,
            light_fixes,
            validate,
        } => {
            let dir = match tools_dir {
                Some(dir) => dir.clone(),
                None => super::tools::default_tools_dir()?,
            };
            let tools = super::tools::provision(&dir, false).await?;
            let plan = super::tools::GeneratorPlan {
                delta_plugin: *delta_plugin,
                groundcoverify: *groundcover,
                light_fixes: *light_fixes,
            };
            let cancel = tokio_util::sync::CancellationToken::new();
            let log = |line: &str| eprintln!("  {line}");
            super::tools::run_generators(&tools, &plan, cfg, output_dir, config_dir, &cancel, &log)
                .await?;
            if *validate {
                super::tools::run(&super::tools::validator_command(&tools, cfg), &cancel, &log)
                    .await?;
            }
        }
        MomwCommand::Install {
            slug,
            dir,
            game_dir,
            downloads,
            tools_dir,
            concurrency,
            skip_tools,
            validate,
            openmw_version,
            no_navmesh,
            no_register,
            hosted,
            only,
        } => {
            let args = InstallArgs {
                slug,
                dir,
                game_dir: game_dir.as_deref(),
                downloads: downloads.as_deref(),
                tools_dir: tools_dir.as_deref(),
                concurrency: *concurrency,
                skip_tools: *skip_tools,
                validate: *validate,
                openmw_version: openmw_version.as_deref(),
                no_navmesh: *no_navmesh,
                no_register: *no_register,
                only,
            };
            if *hosted {
                // Nothing but JSON may reach stdout from here on.
                return super::hosted::run_hosted(prepare_install(options, &args)).await;
            }
            let super::hosted::Prepared {
                plan,
                request,
                lock: _lock,
            } = prepare_install(options, &args).await?;

            let key = crate::settings::Settings::load().nexus_api_key;
            if key.trim().is_empty() {
                bail!("no Nexus API key is saved in the DS2 Modpack settings");
            }
            let (access, lister) = api_nexus(key.trim())?;
            let sink: super::pipeline::EventSink = std::sync::Arc::new(|event| match event {
                super::pipeline::Event::Phase(p) => eprintln!("== {}", p.label()),
                super::pipeline::Event::Stage(super::install::Event::Extracting {
                    name, ..
                }) => eprintln!("  unpacking {name}"),
                super::pipeline::Event::Stage(_) => {}
                super::pipeline::Event::Tool(line) => eprintln!("  {line}"),
                super::pipeline::Event::Note(note) => eprintln!("  note: {note}"),
            });
            let cancel = tokio_util::sync::CancellationToken::new();
            let report =
                super::pipeline::run_install(&plan, &request, access, &lister, cancel, sink)
                    .await?;
            println!("Instance:        {}", request.root.display());
            println!("Mods:            {}", report.instance.mods);
            println!(
                "Plugins:         {} enabled, {} left off",
                report.instance.plugins_enabled, report.instance.plugins_left_off
            );
            println!("Archives:        {}", report.instance.archives_enabled);
            println!("Warnings:        {}", report.warnings.len());
            // Every list is its own instance, so installing several side by
            // side only needs each to appear in Fluorine's list.
            if !*no_register {
                if crate::fluorine::detect(None).is_some() || crate::fluorine::settings_exist() {
                    crate::fluorine::register_portable_instance(&request.root, false)?;
                    println!("Registered:      added to Fluorine's instances (not switched to)");
                } else {
                    println!("Registered:      Fluorine not found; open the folder from Fluorine");
                }
            }
            if let Some(problems) = &report.validator_problems {
                println!("Validator:       reported problems\n{problems}");
            }
        }
        MomwCommand::Bundle {
            slug,
            dir,
            game_dir,
            openmw_version,
            no_navmesh,
            validate,
        } => {
            let _lock = super::lock::InstallLock::acquire(&super::lock::default_lock_path()?)?;
            if !dir.join("mods").is_dir() {
                bail!(
                    "{} is not an installed instance (no mods folder)",
                    dir.display()
                );
            }
            let host = options.host.clone();
            let data = MomwClient::new(options)?.fetch_list(slug).await?;
            let plan = build_plan_with_host(&data, &host);
            // The config is built from the list as the site serves it now. If
            // the site has moved on since this instance was installed, the
            // mods on disk no longer match it, so an update must be a full
            // install rather than a re-bundle.
            if let Some(installed) = super::pipeline::installed_list_version(dir) {
                if plan.momw_version.as_deref() != Some(installed.as_str()) {
                    bail!(
                        "this instance was installed from {slug} {installed}, but the site now \
                         serves {}. Run `ds2modpack momw install` to update it.",
                        plan.momw_version.as_deref().unwrap_or("a newer version")
                    );
                }
            }
            let game_dir = match game_dir {
                Some(dir) => dir.clone(),
                None => find_morrowind()?,
            };
            super::pipeline::check_game_dir(&game_dir)?;
            let openmw = super::openmw::ensure_release(
                &super::openmw::default_store_dir()?,
                &super::openmw::Channel::from_arg(openmw_version.as_deref()),
            )
            .await?;
            let tools_dir = super::tools::default_tools_dir()?;
            let tool_set = if *validate {
                Some(super::tools::provision(&tools_dir, false).await?)
            } else {
                None
            };
            let request = super::pipeline::InstallRequest {
                root: dir.clone(),
                game_dir,
                tools_dir,
                downloads_dir: super::install::default_downloads_dir()?,
                concurrency: 1,
                openmw,
                // The merged plugins already exist in the instance.
                generators: false,
                validate: *validate,
                navmesh: !*no_navmesh,
                shortcut: false,
            };
            let sink: super::pipeline::EventSink = std::sync::Arc::new(|event| match event {
                super::pipeline::Event::Phase(p) => eprintln!("== {}", p.label()),
                super::pipeline::Event::Tool(line) => eprintln!("  {line}"),
                super::pipeline::Event::Note(note) => eprintln!("  note: {note}"),
                super::pipeline::Event::Stage(_) => {}
            });
            let mut report = super::pipeline::InstallReport::default();
            let cancel = tokio_util::sync::CancellationToken::new();
            super::pipeline::finish_instance(
                &plan,
                &request,
                tool_set.as_ref(),
                &cancel,
                &sink,
                &mut report,
            )
            .await?;
            println!("Instance:        {}", dir.display());
            println!("OpenMW:          {}", request.openmw.version);
            println!(
                "Plugins:         {} enabled, {} left off",
                report.instance.plugins_enabled, report.instance.plugins_left_off
            );
            println!(
                "Launch with:     {}",
                dir.join("Launch OpenMW.sh").display()
            );
            if let Some(problems) = &report.validator_problems {
                println!("Validator:       reported problems\n{problems}");
            }
        }
        MomwCommand::Stage {
            slug,
            out,
            downloads,
            concurrency,
            skip_nexus,
            only,
        } => {
            let host = options.host.clone();
            let data = MomwClient::new(options)?.fetch_list(slug).await?;
            let mut plan = build_plan_with_host(&data, &host);
            let wanted: Vec<String> = only.iter().map(|o| o.to_lowercase()).collect();
            plan.retain_mods(|a| {
                (!*skip_nexus || !matches!(a.source, super::plan::SourceKind::Nexus { .. }))
                    && (wanted.is_empty()
                        || wanted.iter().any(|w| a.mod_name.to_lowercase().contains(w)))
            });
            eprintln!(
                "Staging {} archives of {} mods",
                plan.archives.len(),
                plan.data_entries.len()
            );

            let key = crate::settings::Settings::load().nexus_api_key;
            let key = key.trim();
            let needs_nexus = plan
                .archives
                .iter()
                .any(|a| matches!(a.source, super::plan::SourceKind::Nexus { .. }));
            let (access, resolved) = if needs_nexus {
                if key.is_empty() {
                    bail!("no Nexus API key is saved; use --skip-nexus to stage the rest");
                }
                let nexus = std::sync::Arc::new(crate::downloaders::NexusDownloader::new(key)?);
                let cache = super::nexus::DiskCache {
                    inner: crate::downloaders::NexusDownloader::new(key)?,
                    dir: dirs::cache_dir()
                        .context("could not find the cache directory")?
                        .join("ds2modpack")
                        .join("nexus-files"),
                    ttl: std::time::Duration::from_secs(24 * 3600),
                };
                let resolved = super::nexus::resolve_nexus_archives(&plan, &cache, 4).await;
                (super::fetch::NexusAccess::Api(nexus), resolved)
            } else {
                (super::fetch::NexusAccess::None, Vec::new())
            };

            let cancel = tokio_util::sync::CancellationToken::new();
            let fetcher = super::fetch::Fetcher::new(access, cancel)?;
            std::fs::create_dir_all(out)?;
            let stage_options = super::install::StageOptions {
                downloads_dir: downloads.clone().unwrap_or_else(|| out.join("downloads")),
                stage_dir: out.join(".stage"),
                concurrency: *concurrency,
            };
            let sink: super::install::EventSink = std::sync::Arc::new(|event| match event {
                super::install::Event::Extracting { name, .. } => eprintln!("  unpacking {name}"),
                super::install::Event::Warning(w) => eprintln!("  warning: {w}"),
                _ => {}
            });
            let outcome =
                super::install::stage_archives(&plan, &resolved, &fetcher, &stage_options, &sink)
                    .await;
            let report =
                super::stage::publish_entries(&plan, &stage_options.stage_dir, &out.join("mods"))?;
            println!("Archives:        {}", plan.archives.len());
            println!("Failed:          {}", outcome.failures.len());
            for (index, reason) in &outcome.failures {
                println!("  FAILED {}: {reason}", plan.archives[*index].mod_name);
            }
            println!("Mod folders:     {}", report.published);
            for missing in &report.missing {
                println!("  MISSING {missing}");
            }
            println!("tes3cmd actions: {} (not run here)", outcome.cleans.len());
            println!("Warnings:        {}", outcome.warnings.len());
            if outcome.failures.is_empty() && report.missing.is_empty() {
                let _ = std::fs::remove_dir_all(&stage_options.stage_dir);
            } else {
                bail!("some mods could not be staged");
            }
        }
        MomwCommand::Resolve {
            slug,
            json,
            concurrency,
            no_cache,
        } => {
            let host = options.host.clone();
            let data = MomwClient::new(options)?.fetch_list(slug).await?;
            let plan = build_plan_with_host(&data, &host);
            let key = crate::settings::Settings::load().nexus_api_key;
            if key.trim().is_empty() {
                bail!("no Nexus API key is saved in the DS2 Modpack settings");
            }
            let nexus = crate::downloaders::NexusDownloader::new(key.trim())?;
            let cache_dir = dirs::cache_dir()
                .context("could not find the cache directory")?
                .join("ds2modpack")
                .join("nexus-files");
            let lister = super::nexus::DiskCache {
                inner: nexus,
                dir: cache_dir,
                ttl: std::time::Duration::from_secs(if *no_cache { 0 } else { 24 * 3600 }),
            };
            let resolved = super::nexus::resolve_nexus_archives(&plan, &lister, *concurrency).await;
            if *json {
                let mut out = std::io::stdout().lock();
                serde_json::to_writer_pretty(&mut out, &resolved)?;
                writeln!(out)?;
            } else {
                print_resolution(&resolved);
            }
            if resolved
                .iter()
                .any(|r| matches!(r.outcome, super::nexus::Outcome::Failed { .. }))
            {
                bail!("some Nexus files could not be resolved");
            }
        }
    }
    Ok(())
}

struct InstallArgs<'a> {
    slug: &'a str,
    dir: &'a Path,
    game_dir: Option<&'a Path>,
    downloads: Option<&'a Path>,
    tools_dir: Option<&'a Path>,
    concurrency: usize,
    skip_tools: bool,
    validate: bool,
    openmw_version: Option<&'a str>,
    no_navmesh: bool,
    no_register: bool,
    only: &'a [String],
}

/// Everything checked before any download starts, so a problem shows up
/// immediately: the lock, the tools, the game folder and the list itself.
async fn prepare_install(
    options: SourceOptions,
    a: &InstallArgs<'_>,
) -> Result<super::hosted::Prepared> {
    let lock = super::lock::InstallLock::acquire(&super::lock::default_lock_path()?)?;
    let host = options.host.clone();
    let data = MomwClient::new(options)?.fetch_list(a.slug).await?;
    let mut plan = build_plan_with_host(&data, &host);
    if !a.only.is_empty() {
        let wanted: Vec<String> = a.only.iter().map(|o| o.to_lowercase()).collect();
        plan.retain_mods(|j| wanted.iter().any(|w| j.mod_name.to_lowercase().contains(w)));
        eprintln!("note: --only builds an incomplete instance");
    }
    // The OpenMW this instance will run with. Fetched now, so a failed
    // download shows up before the mods are downloaded.
    let openmw = super::openmw::ensure_release(
        &super::openmw::default_store_dir()?,
        &super::openmw::Channel::from_arg(a.openmw_version),
    )
    .await?;
    let request = super::pipeline::InstallRequest {
        root: a.dir.to_path_buf(),
        game_dir: match a.game_dir {
            Some(dir) => dir.to_path_buf(),
            None => find_morrowind()?,
        },
        tools_dir: match a.tools_dir {
            Some(dir) => dir.to_path_buf(),
            None => super::tools::default_tools_dir()?,
        },
        // One cache for every list, so archives are fetched once.
        downloads_dir: match a.downloads {
            Some(dir) => dir.to_path_buf(),
            None => super::install::default_downloads_dir()?,
        },
        concurrency: a.concurrency,
        openmw,
        generators: !a.skip_tools,
        validate: a.validate && !a.skip_tools,
        navmesh: !a.no_navmesh && !a.skip_tools,
        shortcut: !a.no_register,
    };
    Ok(super::hosted::Prepared {
        plan,
        request,
        lock,
    })
}

/// The user's Morrowind folder, from the installed-games scan.
fn find_morrowind() -> Result<PathBuf> {
    let scan = crate::game_finder::detect_all_games();
    let game = scan
        .find_by_name("Morrowind")
        .context("Morrowind was not found; pass --game-dir")?;
    if !game.install_path.join("Data Files").is_dir() {
        bail!(
            "{} has no Data Files folder; pass --game-dir",
            game.install_path.display()
        );
    }
    Ok(game.install_path.clone())
}

/// Nexus access through the user's own API key, with cached file lists.
fn api_nexus(
    key: &str,
) -> Result<(
    super::fetch::NexusAccess,
    super::nexus::DiskCache<crate::downloaders::NexusDownloader>,
)> {
    let lister = super::nexus::DiskCache {
        inner: crate::downloaders::NexusDownloader::new(key)?,
        dir: dirs::cache_dir()
            .context("could not find the cache directory")?
            .join("ds2modpack")
            .join("nexus-files"),
        ttl: std::time::Duration::from_secs(24 * 3600),
    };
    Ok((
        super::fetch::NexusAccess::Api(std::sync::Arc::new(
            crate::downloaders::NexusDownloader::new(key)?,
        )),
        lister,
    ))
}

fn print_resolution(resolved: &[super::nexus::ResolvedArchive]) {
    use super::nexus::Outcome;
    let mut updated = 0;
    let mut failed = Vec::new();
    for r in resolved {
        match &r.outcome {
            Outcome::Resolved {
                followed_update, ..
            } => updated += usize::from(*followed_update),
            Outcome::Failed { error } => failed.push((r, error)),
        }
    }
    println!(
        "Nexus files:     {} ({} resolved, {} replaced by a newer upload, {} failed)",
        resolved.len(),
        resolved.len() - failed.len(),
        updated,
        failed.len()
    );
    for (r, error) in failed {
        println!(
            "  FAILED {} [{}/mods/{}] '{}': {}",
            r.mod_name,
            r.domain,
            r.mod_id,
            r.wanted.as_deref().unwrap_or("?"),
            error
        );
    }
}

fn print_summary(plan: &MomwPlan) {
    let s = &plan.summary;
    println!("List:            {}", plan.slug);
    if let Some(v) = &plan.momw_version {
        println!("MOMW version:    {v}");
    }
    println!("Mods:            {}", s.mods);
    println!("Archives:        {}", s.archives);
    for (kind, n) in &s.sources {
        println!("  {kind:<14} {n}");
    }
    println!(
        "Data entries:    {} ({} nested)",
        s.data_entries, s.nested_data_entries
    );
    println!("Content files:   {}", s.content_files);
    println!("Groundcover:     {}", s.groundcover_files);
    if !plan.tools.is_empty() {
        let tools: Vec<String> = plan.tools.iter().map(|t| format!("{t:?}")).collect();
        println!("Tools needed:    {}", tools.join(", "));
    }
    if !plan.generated_content.is_empty() {
        println!("Tool-generated:  {}", plan.generated_content.join(", "));
    }
    for w in &plan.warnings {
        println!("warning: {w}");
    }
    for b in &plan.blockers {
        println!("BLOCKER: {b}");
    }
    if plan.blockers.is_empty() {
        println!("No blockers.");
    }
}
