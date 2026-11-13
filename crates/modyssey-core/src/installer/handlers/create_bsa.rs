//! CreateBSA directive handler
//!
//! Builds BSA/BA2 archives from files staged in TEMP_BSA_FILES folders.

use crate::bsa::{Ba2Builder, Ba2CompressionFormat, Ba2Version, BsaBuilder};
use crate::installer::processor::ProcessContext;
use crate::installer::sidecar;
use crate::modlist::{BSAFileState, BSAState, CreateBSADirective};
use anyhow::{Context, Result};
use ba2::tes4::{ArchiveFlags, ArchiveTypes, Version};
use rayon::prelude::*;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use walkdir::WalkDir;

/// Archive type to create
enum ArchiveKind {
    /// TES4 BSA (Oblivion, FO3, FNV, Skyrim)
    Bsa {
        version: Version,
        flags: ArchiveFlags,
        types: ArchiveTypes,
    },
    /// FO4 BA2 (Fallout 4, Fallout 76, Starfield)
    Ba2 { version: Ba2Version },
}

fn preserve_ba2_compression(builder: &mut Ba2Builder, states: &[BSAFileState]) {
    for state in states {
        if let BSAFileState::BA2(file) = state {
            builder.set_file_compression(&file.path, file.compressed);
        }
    }
}

/// Collect staged file paths from a staging directory.
/// Returns (archive_path, disk_path) pairs. Does NOT read file contents.
fn collect_staged_files(
    staging_dir: &std::path::Path,
    archive_kind: &ArchiveKind,
) -> Result<Vec<(String, PathBuf)>> {
    let mut files = Vec::new();

    for entry in WalkDir::new(staging_dir) {
        let entry = entry.context("Could not enumerate archive staging files")?;
        if !entry.file_type().is_file() {
            continue;
        }
        let file_path = entry.into_path();

        let rel_path = file_path
            .strip_prefix(staging_dir)
            .with_context(|| format!("Failed to get relative path for {}", file_path.display()))?;

        let archive_path = match archive_kind {
            ArchiveKind::Bsa { .. } => rel_path.to_string_lossy().replace('/', "\\"),
            ArchiveKind::Ba2 { .. } => rel_path.to_string_lossy().replace('\\', "/"),
        };

        files.push((archive_path, file_path));
    }

    Ok(files)
}

fn validate_members<'a>(
    paths: impl IntoIterator<Item = &'a str>,
    states: &[BSAFileState],
) -> Result<()> {
    let expected: BTreeSet<_> = states
        .iter()
        .map(|s| sidecar::normalize_manifest_path(s.path()))
        .collect();
    let mut actual = BTreeSet::new();
    for path in paths {
        let normalized = sidecar::normalize_manifest_path(path);
        anyhow::ensure!(
            actual.insert(normalized),
            "Duplicate staged archive path: {path}"
        );
    }
    let missing: Vec<_> = expected.difference(&actual).collect();
    let extra: Vec<_> = actual.difference(&expected).collect();
    anyhow::ensure!(
        missing.is_empty() && extra.is_empty(),
        "Archive membership differs from Wabbajack: {} missing ({:?}), {} unexpected ({:?})",
        missing.len(),
        &missing[..missing.len().min(8)],
        extra.len(),
        &extra[..extra.len().min(8)]
    );
    Ok(())
}

/// Handle a CreateBSA directive
///
/// Reads files from the staging directory and builds a BSA/BA2 archive.
pub fn handle_create_bsa(ctx: &ProcessContext, directive: &CreateBSADirective) -> Result<()> {
    // Staging directory: {output_dir}/TEMP_BSA_FILES/{temp_id}/
    let staging_dir = ctx
        .config
        .output_dir
        .join("TEMP_BSA_FILES")
        .join(directive.temp_id.to_string());

    if !staging_dir.exists() {
        anyhow::bail!("Staging directory not found: {}", staging_dir.display());
    }

    // Get archive settings from state
    let archive_kind = match &directive.state {
        BSAState::BSA(state) => {
            tracing::info!(
                "Creating TES4 BSA: {} (version {}, magic {})",
                directive.to,
                state.version,
                state.magic
            );
            let version = match state.version {
                103 => Version::v103,
                104 => Version::v104,
                105 => Version::v105,
                _ => Version::v105,
            };
            let flags = ArchiveFlags::from_bits_truncate(state.archive_flags);
            let types = ArchiveTypes::from_bits_truncate(state.file_flags as u16);
            ArchiveKind::Bsa {
                version,
                flags,
                types,
            }
        }
        BSAState::BA2(state) => {
            let version = Ba2Version::from_u32(state.version);
            tracing::info!(
                "Creating FO4 BA2: {} (type {:?}, version {:?})",
                directive.to,
                state.archive_type,
                version
            );
            ArchiveKind::Ba2 { version }
        }
    };

    // Collect staged file paths (no data read yet)
    let mut files = collect_staged_files(&staging_dir, &archive_kind)?;
    // DDS cache receipts are not archive members. An explicitly authored file
    // with the same extension is still retained and checked below.
    let expected: BTreeSet<_> = directive
        .file_states
        .iter()
        .map(|s| sidecar::normalize_manifest_path(s.path()))
        .collect();
    files.retain(|(name, _)| {
        !name.ends_with(".modyssey-hash")
            || expected.contains(&sidecar::normalize_manifest_path(name))
    });
    validate_members(
        files.iter().map(|(name, _)| name.as_str()),
        &directive.file_states,
    )?;

    if files.is_empty() {
        anyhow::bail!(
            "No files found in staging directory: {}",
            staging_dir.display()
        );
    }

    // Write the archive
    let output_path = ctx.resolve_output_path(&directive.to);
    ctx.dir_cache.ensure_parent_dirs(&output_path)?;
    let temporary = tempfile::Builder::new()
        .prefix(".modyssey-build-")
        .suffix(&format!(
            ".{}",
            output_path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("bsa")
        ))
        .tempfile_in(
            output_path
                .parent()
                .context("Archive has no parent directory")?,
        )?;
    let build_path = temporary.path();

    match archive_kind {
        ArchiveKind::Bsa {
            version,
            flags,
            types,
        } => {
            // Build BSA — files are read from disk on demand during build()
            let build_result = {
                let mut builder = BsaBuilder::new()
                    .with_version(version)
                    .with_flags(flags)
                    .with_types(types);

                for (path, disk_path) in &files {
                    builder.add_file(path, disk_path.clone());
                }

                builder.build(build_path)
            };

            // If it fails with overflow, retry without compression
            if let Err(e) = build_result {
                let err_str = format!("{:?}", e);
                if err_str.contains("overflow") {
                    tracing::warn!("BSA build failed with overflow, retrying without compression");

                    let flags_no_compress = flags & !ArchiveFlags::COMPRESSED;

                    let mut builder = BsaBuilder::new()
                        .with_version(version)
                        .with_flags(flags_no_compress)
                        .with_types(types);

                    for (path, disk_path) in &files {
                        builder.add_file(path, disk_path.clone());
                    }

                    builder.build(build_path).with_context(|| {
                        format!(
                            "Failed to build BSA (uncompressed): {}",
                            output_path.display()
                        )
                    })?;
                } else {
                    return Err(e).with_context(|| {
                        format!("Failed to build BSA: {}", output_path.display())
                    });
                }
            }
        }
        ArchiveKind::Ba2 { version } => {
            // BA2 builder reads files from disk on demand during build(),
            // keeping peak memory at ~1 file per rayon thread.
            let mut builder = Ba2Builder::from_name(&directive.to)
                .with_version(version)
                .with_compression(Ba2CompressionFormat::Zlib);

            for (path, disk_path) in &files {
                builder.add_file(path, disk_path.clone());
            }

            preserve_ba2_compression(&mut builder, &directive.file_states);

            builder
                .build(build_path)
                .with_context(|| format!("Failed to build BA2: {}", output_path.display()))?;
        }
    }

    let written = crate::bsa::list_archive_files(build_path)?;
    validate_members(
        written.iter().map(|f| f.path.as_str()),
        &directive.file_states,
    )?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(&output_path)
        .with_context(|| format!("Could not replace {}", output_path.display()))?;

    // Write sidecar hash cache so re-runs can skip this BSA
    if let Err(e) = sidecar::write_sidecar(&output_path, &directive.hash) {
        tracing::warn!(
            "Failed to write BSA sidecar for {}: {}",
            output_path.display(),
            e
        );
    }

    // Write per-file manifest for partial reuse on future updates.
    // Files are still in staging and likely in page cache from the build.
    let manifest_entries: Vec<(String, String)> = files
        .par_iter()
        .filter_map(|(archive_path, disk_path)| {
            crate::hash::compute_file_hash(disk_path)
                .ok()
                .map(|h| (sidecar::normalize_manifest_path(archive_path), h))
        })
        .collect();

    if !manifest_entries.is_empty() {
        if let Err(e) = sidecar::write_manifest(&output_path, &manifest_entries) {
            tracing::warn!(
                "Failed to write BSA manifest for {}: {}",
                output_path.display(),
                e
            );
        }
    }

    // Clean up staging directory
    if let Err(e) = fs::remove_dir_all(&staging_dir) {
        tracing::warn!(
            "Failed to clean up staging directory {}: {}",
            staging_dir.display(),
            e
        );
    }

    Ok(())
}

/// Check if a CreateBSA output already exists and is valid.
///
/// Uses sidecar hash cache: checks that the `.modyssey-hash` sidecar matches the
/// directive's expected hash and the file size hasn't changed (corruption check).
pub fn output_bsa_valid(ctx: &ProcessContext, directive: &CreateBSADirective) -> bool {
    let output_path = ctx.resolve_output_path(&directive.to);
    sidecar::sidecar_valid(&output_path, &directive.hash)
        && sidecar::read_manifest(&output_path).is_some_and(|manifest| {
            validate_members(manifest.keys().map(String::as_str), &directive.file_states).is_ok()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ba2::fo4::{Archive, ArchiveKey, FileWriteOptions};
    use ba2::prelude::*;

    #[test]
    fn archive_requires_all_authored_members_and_no_unexpected_files() {
        let states: Vec<BSAFileState> = serde_json::from_value(serde_json::json!([
            {"$type":"BA2FileState, Compression.BSA", "Path":"Textures\\\\A.dds", "Index":0, "Compressed":true},
            {"$type":"BA2FileState, Compression.BSA", "Path":"Textures/B.dds", "Index":1, "Compressed":true}
        ])).unwrap();
        assert!(validate_members(["textures/a.dds", "textures/b.dds"], &states).is_ok());
        let error = validate_members(["textures/a.dds"], &states)
            .unwrap_err()
            .to_string();
        assert!(error.contains("1 missing") && error.contains("textures/b.dds"));
        assert!(validate_members(
            ["textures/a.dds", "textures/b.dds", "textures/c.dds"],
            &states
        )
        .is_err());
        assert!(validate_members(
            ["textures/a.dds", "Textures/A.dds", "textures/b.dds"],
            &states
        )
        .is_err());
    }

    #[test]
    fn ba2_compression_preserves_wabbajack_file_states() -> Result<()> {
        // WAV/XWM must remain stored for Fallout's streaming reader. FUZ
        // containers and other files may be compressed in the same archive.
        // Read real serialized state fields so a future handler regression
        // cannot silently replace per-file flags with the archive default.
        let states: Vec<BSAFileState> = serde_json::from_value(serde_json::json!([
            {"$type":"BA2FileState, Compression.BSA", "Path":"Sound\\FX\\test.WAV", "Index":0, "Compressed":false},
            {"$type":"BA2FileState, Compression.BSA", "Path":"music/test.xwm", "Index":1, "Compressed":false},
            {"$type":"BA2FileState, Compression.BSA", "Path":"sound/voice/test.fuz", "Index":2, "Compressed":true},
            {"$type":"BA2FileState, Compression.BSA", "Path":"meshes/test.nif", "Index":3, "Compressed":true},
            {"$type":"BA2FileState, Compression.BSA", "Path":"other/stored.bin", "Index":4, "Compressed":false}
        ]))?;
        let directory = tempfile::tempdir()?;
        let payload = b"RIFF test data repeated for a compressible regression fixture".repeat(100);
        let staged = directory.path().join("staged");
        fs::write(&staged, &payload)?;

        for (index, (version, default)) in [
            (Ba2Version::V1, Ba2CompressionFormat::Zlib),
            (Ba2Version::V7, Ba2CompressionFormat::Zlib),
            (Ba2Version::V8, Ba2CompressionFormat::None),
        ]
        .into_iter()
        .enumerate()
        {
            let output = directory.path().join(format!("mixed-{index}.ba2"));
            let mut builder = Ba2Builder::from_name("Mixed - Main.ba2")
                .with_version(version)
                .with_compression(default);
            for state in &states {
                // Staging and state paths need not use the same slash/case.
                builder.add_file(
                    &state.path().replace('\\', "/").to_ascii_lowercase(),
                    staged.clone(),
                );
            }
            preserve_ba2_compression(&mut builder, &states);
            builder.build(&output)?;

            let (archive, options) = Archive::read(output.as_path())?;
            assert_eq!(archive.len(), states.len());
            let write_options: FileWriteOptions = options.into();
            for state in &states {
                let BSAFileState::BA2(expected) = state else {
                    unreachable!()
                };
                let key = ArchiveKey::from(expected.path.as_bytes());
                let file = archive.get(&key).context("missing rebuilt entry")?;
                for chunk in file.iter() {
                    assert_eq!(
                        chunk.decompressed_len().is_some(),
                        expected.compressed,
                        "compression changed for {}",
                        expected.path
                    );
                }
                let mut decoded = Vec::new();
                file.write(&mut decoded, &write_options)?;
                assert_eq!(decoded, payload, "payload changed for {}", expected.path);
            }
        }
        Ok(())
    }
}
