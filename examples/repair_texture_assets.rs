//! Build asset repairs in a separate workspace; never change a profile or save.
//! Usage: cargo run --release --example repair_texture_assets -- PLAN.json
//! The JSON array contains Extract, ExtractBa2, Texture, or Repack actions
//! (tagged by "op"). All output destinations are explicit staging paths.
use anyhow::{Context, Result};
use ba2::fo4::{
    Archive, ArchiveKey, ChunkCompressionOptions, CompressionFormat, CompressionLevel, File,
    FileReadOptions, Format,
};
use ba2::prelude::*;
use ba2::CompressionResult;
use image_dds::ddsfile::Dds;
use modyssey::modlist::{BSAFileState, CreateBSADirective};
use modyssey::textures::{process_texture_with_mips, OutputFormat};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufWriter, Cursor, Write};
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
#[serde(tag = "op")]
enum Action {
    Extract {
        archive: PathBuf,
        files: Vec<String>,
        output: PathBuf,
    },
    ExtractBa2 {
        archive: PathBuf,
        files: BTreeMap<String, PathBuf>,
    },
    Texture {
        source: PathBuf,
        output: PathBuf,
        width: u32,
        height: u32,
        mip_levels: u32,
        format: String,
        #[serde(default)]
        trim_bytes: Option<usize>,
    },
    Repack {
        archive: PathBuf,
        output: PathBuf,
        replacements: BTreeMap<String, PathBuf>,
        directive: CreateBSADirective,
    },
}

fn normalize(path: &str) -> String {
    path.split(['/', '\\'])
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/")
        .to_lowercase()
}

fn write_output(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path.parent().context("Missing output parent")?;
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(data)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}

fn execute(action: Action) -> Result<()> {
    match action {
        Action::Extract {
            archive,
            files,
            output,
        } => {
            fs::create_dir_all(&output)?;
            let extracted = modyssey::archive::sevenzip::extract_files_case_insensitive(
                &archive, &files, &output,
            )?;
            let expected: BTreeSet<_> = files.iter().map(|s| normalize(s)).collect();
            anyhow::ensure!(extracted == expected.len(), "Incomplete source extraction");
        }
        Action::ExtractBa2 { archive, files } => {
            let wanted = files.keys().map(|s| normalize(s)).collect();
            let extracted =
                modyssey::bsa::extract_archive_batch(&archive, &wanted, |name, data| {
                    let output = files
                        .get(&normalize(name))
                        .context("Unexpected archive entry")?;
                    write_output(output, &data)
                })?;
            anyhow::ensure!(extracted == files.len(), "Incomplete BA2 source extraction");
        }
        Action::Texture {
            source,
            output,
            width,
            height,
            mip_levels,
            format,
            trim_bytes,
        } => {
            let input = fs::read(&source).with_context(|| format!("Read {}", source.display()))?;
            let data = if let Some(size) = trim_bytes {
                let mut dds = Dds::read(Cursor::new(&input))?;
                anyhow::ensure!(
                    dds.get_width() == width && dds.get_height() == height,
                    "Trim dimensions differ"
                );
                anyhow::ensure!(
                    dds.data.len() >= size
                        && mip_levels > 0
                        && dds.header.mip_map_count.unwrap_or(1) >= mip_levels,
                    "Cannot recover requested mip data"
                );
                dds.data.truncate(size);
                dds.header.mip_map_count = Some(mip_levels);
                let mut data = Vec::new();
                dds.write(&mut data)?;
                data
            } else {
                process_texture_with_mips(
                    &input,
                    width,
                    height,
                    OutputFormat::parse(&format).context("Unsupported format")?,
                    mip_levels,
                )?
                .data
            };
            let metadata =
                directxtex::TexMetadata::from_dds(&data, directxtex::DDS_FLAGS::default(), None)?;
            anyhow::ensure!(
                metadata.width == width as usize
                    && metadata.height == height as usize
                    && metadata.mip_levels == mip_levels as usize,
                "Repaired DDS differs from requested dimensions/mips"
            );
            write_output(&output, &data)?;
        }
        Action::Repack {
            archive: source,
            output,
            replacements,
            directive,
        } => {
            anyhow::ensure!(
                source != output,
                "Repack must target a separate staging file"
            );
            let (mut archive, options) = Archive::read(source.as_path())?;
            let read_options = FileReadOptions::builder()
                .format(Format::DX10)
                .compression_format(CompressionFormat::Zip)
                .compression_level(CompressionLevel::FO4)
                .compression_result(CompressionResult::Compressed)
                .build();
            for (name, path) in replacements {
                let data = fs::read(path)?;
                let file = File::read(ba2::Copied(&data), &read_options)?;
                archive.insert(ArchiveKey::from(name.as_bytes()), file);
            }
            let compression = ChunkCompressionOptions::builder()
                .compression_format(CompressionFormat::Zip)
                .compression_level(CompressionLevel::FO4)
                .build();
            for state in &directive.file_states {
                if let BSAFileState::BA2(state) = state {
                    let name = normalize(&state.path);
                    let file = archive
                        .get_mut(&ArchiveKey::from(name.as_bytes()))
                        .context("Missing general archive member")?;
                    for chunk in file.iter_mut() {
                        if chunk.is_compressed() != state.compressed {
                            *chunk = if state.compressed {
                                chunk.compress(&compression)?
                            } else {
                                chunk.decompress(&compression)?
                            };
                        }
                    }
                }
            }
            let expected: BTreeSet<_> = directive
                .file_states
                .iter()
                .map(|s| normalize(s.path()))
                .collect();
            anyhow::ensure!(
                archive.len() == expected.len(),
                "Archive member count differs from manifest"
            );
            for name in expected {
                anyhow::ensure!(
                    archive.get(&ArchiveKey::from(name.as_bytes())).is_some(),
                    "Missing {name}"
                );
            }
            fs::create_dir_all(output.parent().context("Missing output parent")?)?;
            let temporary = tempfile::NamedTempFile::new_in(output.parent().unwrap())?;
            {
                let mut writer = BufWriter::new(temporary.as_file());
                archive.write(&mut writer, &options)?;
                writer.flush()?;
            }
            temporary.as_file().sync_all()?;
            temporary.persist(output)?;
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let plan = std::env::args_os()
        .nth(1)
        .context("Pass a JSON repair plan")?;
    let actions: Vec<Action> = serde_json::from_slice(&fs::read(plan)?)?;
    let total = actions.len();
    for (i, action) in actions.into_iter().enumerate() {
        execute(action).with_context(|| format!("Repair action {} of {total}", i + 1))?;
        println!("Completed {} / {total}", i + 1);
    }
    Ok(())
}
