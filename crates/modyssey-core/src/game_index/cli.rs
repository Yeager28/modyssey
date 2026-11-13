//! `modyssey game-index`: grow and query the known-game-file index.

use super::gallery::{scan_gallery, ScanOptions};
use super::{AddStats, GameIndex};
use crate::modlist::parse_wabbajack_file;
use anyhow::{bail, Result};
use clap::{Args, Subcommand};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Where the index is kept in the source tree.
const DEFAULT_INDEX: &str = "data/game_index.json";

#[derive(Debug, Args)]
pub struct GameIndexArgs {
    /// Index file to read and update (run from the repo root for the default).
    #[arg(long, global = true, default_value = DEFAULT_INDEX)]
    pub index: PathBuf,
    #[command(subcommand)]
    pub command: GameIndexCommand,
}

#[derive(Debug, Subcommand)]
pub enum GameIndexCommand {
    /// Add the game files pinned by local .wabbajack modlists to the index.
    Add {
        /// .wabbajack files to read.
        #[arg(required = true)]
        modlists: Vec<PathBuf>,
        /// Report what would change without writing the index.
        #[arg(long)]
        dry_run: bool,
    },
    /// Read the game files every Wabbajack gallery modlist pins, fetching only
    /// each list's `modlist` entry from the CDN, and add them to the index.
    ScanGallery {
        /// Only lists whose title or machine name contains this text.
        #[arg(long)]
        only: Option<String>,
        /// Stop after this many lists.
        #[arg(long)]
        limit: Option<usize>,
        /// Lists read in parallel.
        #[arg(long, default_value_t = 4)]
        concurrency: usize,
        /// Report what would change without writing the index.
        #[arg(long)]
        dry_run: bool,
    },
    /// Show which game versions a file hash is known to belong to.
    Lookup {
        /// Wabbajack game type, e.g. SkyrimSpecialEdition.
        game: String,
        /// File hash in Wabbajack base64 form.
        hash: String,
    },
    /// Summarize the index per game.
    Stats,
}

pub async fn run(args: &GameIndexArgs) -> Result<()> {
    let mut index = GameIndex::load(&args.index)?;

    match &args.command {
        GameIndexCommand::Add { modlists, dry_run } => {
            let mut failed = 0usize;
            let mut changed = false;
            for path in modlists {
                match parse_wabbajack_file(path) {
                    Ok(modlist) => {
                        let stats = index.add_modlist(&modlist);
                        changed |= stats.changed();
                        println!(
                            "{}: {} game files, {} new, {} new versions, {} rejected",
                            path.display(),
                            stats.seen,
                            stats.new_files,
                            stats.new_versions,
                            stats.rejected
                        );
                    }
                    Err(err) => {
                        failed += 1;
                        eprintln!("{}: {err:#}", path.display());
                    }
                }
            }
            if changed && !dry_run {
                index.save(&args.index)?;
                println!("Wrote {} ({} files)", args.index.display(), index.len());
            }
            if failed > 0 {
                bail!("{failed} modlist(s) could not be read");
            }
        }
        GameIndexCommand::ScanGallery {
            only,
            limit,
            concurrency,
            dry_run,
        } => {
            let opts = ScanOptions {
                only: only.clone(),
                limit: *limit,
                concurrency: *concurrency,
            };
            let report = scan_gallery(&opts, |done, total, scan| match &scan.outcome {
                Ok(list) => eprintln!(
                    "[{done}/{total}] {} {}: {} game files, {:.1} of {:.0} MB fetched",
                    scan.title,
                    scan.version,
                    list.files.files.len(),
                    list.fetched_bytes as f64 / 1e6,
                    list.total_bytes as f64 / 1e6
                ),
                Err(err) => eprintln!(
                    "[{done}/{total}] {} {}: FAILED {err:#}",
                    scan.title, scan.version
                ),
            })
            .await?;

            let mut totals = AddStats::default();
            let mut failed = 0usize;
            let mut fetched = 0u64;
            for scan in &report.scans {
                match &scan.outcome {
                    Ok(list) => {
                        fetched += list.fetched_bytes;
                        for pinned in &list.files.files {
                            totals.merge(index.add_game_file(
                                &list.files.game_type,
                                &pinned.state,
                                pinned.size,
                            ));
                        }
                    }
                    Err(_) => failed += 1,
                }
            }
            println!(
                "{} lists read ({:.0} MB fetched), {} skipped (not on the CDN), {} failed: \
                 {} game files, {} new, {} new versions, {} rejected",
                report.scans.len() - failed,
                fetched as f64 / 1e6,
                report.skipped,
                failed,
                totals.seen,
                totals.new_files,
                totals.new_versions,
                totals.rejected
            );
            if totals.changed() && !dry_run {
                index.save(&args.index)?;
                println!("Wrote {} ({} files)", args.index.display(), index.len());
            }
            if failed > 0 {
                bail!("{failed} gallery list(s) could not be read");
            }
        }
        GameIndexCommand::Lookup { game, hash } => {
            let files = index.lookup_hash(game, hash);
            if files.is_empty() {
                bail!("No entry for {game} {hash}");
            }
            for file in files {
                println!(
                    "{}  {} bytes  {}",
                    file.path,
                    file.size,
                    file.versions.join(", ")
                );
            }
        }
        GameIndexCommand::Stats => {
            let mut per_game: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
            for file in index.files() {
                let entry = per_game.entry(file.game.as_str()).or_default();
                entry.0 += 1;
                entry.1 += file.versions.len();
            }
            println!("{} files in {}", index.len(), args.index.display());
            for (game, (files, versions)) in per_game {
                println!("  {game}: {files} files, {versions} version records");
            }
        }
    }
    Ok(())
}
