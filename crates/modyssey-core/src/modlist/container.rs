//! `.modyssey` container format v1 (PLAN.md §4).
//!
//! A `.modyssey` file is a ZIP (ZIP64-capable) archive whose root entries are:
//!
//! - `MODYSSY.v1.marker` — JSON `{"format":"modyssey","version":1}`
//! - `manifest.json` — the Wabbajack-compatible manifest plus the v1 fields
//!   (`FormatVersion`, `License`, `ToolVersion`, optional `BlobIndex`)
//! - `<uuid4>` entries — binary blobs (inline files, patch deltas, remapped
//!   bodies), named exactly like Wabbajack's so conversion is a straight copy
//!
//! `.wabbajack` files (ZIP containers with a `modlist` entry) keep their own
//! reader in [`super::parse_wabbajack_file`]; this module detects the
//! container type so every entry point (install, info, update) can dispatch
//! on it. A Wabbajack manifest is a strict subset of the v1 manifest schema,
//! so both container readers produce the same [`Modlist`] model.

use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::path::Path;

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use tracing::info;
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;
use zip::ZipArchive;
use zip::ZipWriter;

use super::types::{BlobIndexEntry, DEFAULT_LICENSE, UNDECLARED_LICENSE};
use super::{validate_modlist_paths, Modlist};

/// Name of the magic marker entry at the root of a `.modyssey` container.
pub const MARKER_ENTRY_NAME: &str = "MODYSSY.v1.marker";
/// Name of the manifest entry at the root of a `.modyssey` container.
pub const MANIFEST_ENTRY_NAME: &str = "manifest.json";
/// The `format` value inside a valid marker.
pub const FORMAT_NAME: &str = "modyssey";
/// The container/manifest format version this build reads and writes.
pub const FORMAT_VERSION: u32 = 1;
/// Entry name of the Wabbajack manifest inside a `.wabbajack` container.
pub const MODLIST_ENTRY_NAME: &str = "modlist";
/// Blobs at or above this uncompressed size are written as ZIP64 entries so
/// containers may legally contain blobs > 4 GiB (PLAN.md §4.2).
const ZIP64_ENTRY_SIZE_THRESHOLD: u64 = u32::MAX as u64;
/// 7z archive signature (`37 7A BC AF 27 1C`).
const SEVENZIP_MAGIC: [u8; 6] = [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C];

/// Contents of the `MODYSSY.v1.marker` entry.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Marker {
    format: String,
    version: u32,
}

impl Marker {
    fn v1() -> Self {
        Self {
            format: FORMAT_NAME.to_string(),
            version: FORMAT_VERSION,
        }
    }
}

/// Which modlist container a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerFormat {
    /// A `.modyssey` file (marker + `manifest.json` + blobs).
    Modyssey,
    /// A `.wabbajack` file (ZIP with a `modlist` entry).
    Wabbajack,
}

/// Statistics reported by a successful conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvertStats {
    pub modlist_name: String,
    pub modlist_version: String,
    pub archives: usize,
    pub directives: usize,
    /// Blob entries copied into the new container.
    pub blobs: usize,
    /// Uncompressed bytes of those blobs.
    pub blob_bytes: u64,
    /// Size of the produced `.modyssey` file.
    pub output_bytes: u64,
}

/// Detect which container format `path` is.
///
/// Detection order (PLAN.md §4.2): extension first (`.modyssey` must contain
/// the marker; `.wabbajack` goes to the Wabbajack reader), then content
/// sniffing for files with unknown extensions. Unrecognized containers
/// produce an error naming what was found.
pub fn detect_container_format(path: &Path) -> Result<ContainerFormat> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());

    match extension.as_deref() {
        Some("modyssey") => {
            let mut archive = open_zip(path)?;
            if has_entry(&archive, MARKER_ENTRY_NAME) {
                Ok(ContainerFormat::Modyssey)
            } else if has_entry(&archive, MODLIST_ENTRY_NAME) {
                bail!(
                    "{} contains a Wabbajack 'modlist' entry but no {} marker — it looks \
                     like a .wabbajack file with the wrong extension. Rename it to \
                     .wabbajack and try again.",
                    path.display(),
                    MARKER_ENTRY_NAME
                );
            } else {
                bail!(
                    "{} is not a valid .modyssey file: no {} marker entry. {}",
                    path.display(),
                    MARKER_ENTRY_NAME,
                    describe_entries(&mut archive)
                );
            }
        }
        Some("wabbajack") => Ok(ContainerFormat::Wabbajack),
        _ => {
            let mut magic = [0u8; 6];
            let read = File::open(path)
                .and_then(|mut f| f.read(&mut magic))
                .with_context(|| format!("Failed to read {}", path.display()))?;
            if magic == SEVENZIP_MAGIC {
                bail!(
                    "{} is a 7z archive. Modyssey reads ZIP-based .wabbajack files and \
                     .modyssey files; this container is neither.",
                    path.display()
                );
            }
            if read >= 2 && &magic[..2] == b"PK" {
                let mut archive = open_zip(path)?;
                if has_entry(&archive, MARKER_ENTRY_NAME) {
                    return Ok(ContainerFormat::Modyssey);
                }
                if has_entry(&archive, MODLIST_ENTRY_NAME) {
                    return Ok(ContainerFormat::Wabbajack);
                }
                bail!(
                    "{} is a ZIP archive but not a modlist container: no {} marker and no \
                     'modlist' entry. {}",
                    path.display(),
                    MARKER_ENTRY_NAME,
                    describe_entries(&mut archive)
                );
            }
            bail!(
                "{} is not a modlist container: expected a ZIP-based .wabbajack file or a \
                 .modyssey file, found magic bytes {}.",
                path.display(),
                hex_prefix(&magic, read)
            );
        }
    }
}

/// Parse a modlist from whichever container format `path` uses.
///
/// This is the entry point for install/info/update: `.modyssey` files get
/// marker, format-version and BlobIndex verification before anything is
/// downloaded; `.wabbajack` files take the unchanged Wabbajack reader.
pub fn parse_modlist_container(path: &Path) -> Result<Modlist> {
    match detect_container_format(path)? {
        ContainerFormat::Modyssey => parse_modyssey_file(path),
        ContainerFormat::Wabbajack => super::parse_wabbajack_file(path),
    }
}

/// Open a `.modyssey` file: verify the marker, parse the manifest, enforce
/// the format-version gate, and verify the BlobIndex (if present).
pub fn parse_modyssey_file(path: &Path) -> Result<Modlist> {
    info!("Opening modyssey file: {}", path.display());

    let file = File::open(path).with_context(|| format!("Failed to open: {}", path.display()))?;
    let mut archive =
        ZipArchive::new(BufReader::new(file)).context("Failed to read as ZIP archive")?;

    // --- marker: format name + container version -------------------------
    let marker: Marker = {
        let mut entry = archive
            .by_name(MARKER_ENTRY_NAME)
            .with_context(|| format!("No {MARKER_ENTRY_NAME} entry: not a valid .modyssey file"))?;
        let mut json = String::new();
        entry
            .read_to_string(&mut json)
            .context("Failed to read marker entry")?;
        serde_json::from_str(&json).context("Marker entry is not valid JSON")?
    };
    if marker.format != FORMAT_NAME {
        bail!(
            "Marker names format {:?}, expected {:?} — {} is not a Modyssey container file.",
            marker.format,
            FORMAT_NAME,
            path.display()
        );
    }
    if marker.version > FORMAT_VERSION {
        bail!(
            "This .modyssey file uses container format version {}, which this Modyssey \
             (supporting version {FORMAT_VERSION}) cannot read — a newer Modyssey is required.",
            marker.version
        );
    }
    if marker.version == 0 {
        bail!("Marker declares format version 0 — the .modyssey file is malformed.");
    }

    // --- manifest ---------------------------------------------------------
    let mut manifest_bytes = Vec::new();
    archive
        .by_name(MANIFEST_ENTRY_NAME)
        .with_context(|| format!("No {MANIFEST_ENTRY_NAME} entry in .modyssey file"))?
        .read_to_end(&mut manifest_bytes)
        .context("Failed to read manifest entry")?;

    let modlist: Modlist = serde_json::from_slice(&manifest_bytes)
        .context("Failed to parse modyssey manifest JSON")?;

    // --- FormatVersion gate (required in v1 files) ------------------------
    match modlist.format_version {
        Some(v) if v == FORMAT_VERSION => {}
        Some(v) if v > FORMAT_VERSION => bail!(
            "This .modyssey file uses manifest FormatVersion {v}, which this Modyssey \
             (supporting version {FORMAT_VERSION}) cannot read — a newer Modyssey is required."
        ),
        Some(0) => bail!("Manifest declares FormatVersion 0 — the .modyssey file is malformed."),
        Some(v) => bail!("Manifest declares unsupported FormatVersion {v}."),
        None => bail!(
            "Manifest has no FormatVersion. .modyssey manifests must declare it (a missing \
             FormatVersion means a Wabbajack-legacy manifest — install those as .wabbajack)."
        ),
    }

    // --- BlobIndex verification (early corruption detection) --------------
    if let Some(index) = &modlist.blob_index {
        verify_blob_index(&mut archive, index).with_context(|| {
            format!(
                "Refusing to install '{}': the .modyssey file is corrupted",
                modlist.name
            )
        })?;
    }

    info!(
        "Parsed modyssey modlist '{}' v{} - {} archives, {} directives",
        modlist.name,
        modlist.version,
        modlist.archives.len(),
        modlist.directives.len()
    );

    validate_modlist_paths(&modlist)?;
    Ok(modlist)
}

/// Verify every BlobIndex entry against the container: the entry must exist,
/// its uncompressed size and xxHash64 must match. Any mismatch is a hard,
/// pre-install error.
fn verify_blob_index(
    archive: &mut ZipArchive<BufReader<File>>,
    index: &[BlobIndexEntry],
) -> Result<()> {
    const MAX_REPORTED: usize = 5;
    let mut problems = Vec::new();
    let mut total = 0usize;
    let mut record = |what: String| {
        total += 1;
        if problems.len() < MAX_REPORTED {
            problems.push(what);
        }
    };

    for expected in index {
        let mut entry = match archive.by_name(&expected.entry) {
            Ok(entry) => entry,
            Err(_) => {
                record(format!(
                    "blob {:?}: listed in BlobIndex but missing from the container",
                    expected.entry
                ));
                continue;
            }
        };
        let (size, hash) = hash_reader(&mut entry)
            .with_context(|| format!("Failed to read blob entry {:?}", expected.entry))?;
        if size != expected.size {
            record(format!(
                "blob {:?}: size {} does not match BlobIndex size {}",
                expected.entry, size, expected.size
            ));
        }
        if hash != expected.hash {
            record(format!(
                "blob {:?}: xxHash64 {} does not match BlobIndex hash {}",
                expected.entry, hash, expected.hash
            ));
        }
    }

    if total > 0 {
        bail!(
            "BlobIndex verification failed: {} problem(s). The .modyssey file is corrupted or \
             has been tampered with.\n  {}{}",
            total,
            problems.join("\n  "),
            if total > problems.len() {
                format!("\n  ... and {} more", total - problems.len())
            } else {
                String::new()
            }
        );
    }
    Ok(())
}

/// Convert a `.wabbajack` file to a `.modyssey` file, losslessly.
///
/// The output container carries the marker, the Wabbajack manifest fields
/// verbatim (plus `FormatVersion: 1`, a defaulted `License`, our
/// `ToolVersion`, and a complete `BlobIndex`), and every blob entry copied
/// byte-for-byte (stored uncompressed, per PLAN.md §4.2).
pub fn convert_wabbajack_to_modyssey(input: &Path, output: &Path) -> Result<ConvertStats> {
    info!("Converting {} -> {}", input.display(), output.display());

    let file = File::open(input).with_context(|| format!("Failed to open: {}", input.display()))?;
    let mut src = ZipArchive::new(BufReader::new(file)).context("Failed to read as ZIP archive")?;

    if has_entry(&src, MARKER_ENTRY_NAME) {
        bail!(
            "{} is already a .modyssey file — conversion only takes .wabbajack inputs.",
            input.display()
        );
    }

    // --- read the Wabbajack manifest --------------------------------------
    let mut modlist_bytes = Vec::new();
    if !has_entry(&src, MODLIST_ENTRY_NAME) {
        bail!(
            "No 'modlist' entry in {} — not a .wabbajack file. {}",
            input.display(),
            describe_entries(&mut src)
        );
    }
    src.by_name(MODLIST_ENTRY_NAME)
        .context("Failed to open the 'modlist' entry")?
        .read_to_end(&mut modlist_bytes)
        .context("Failed to read modlist entry")?;

    // Typed parse first: validates the manifest parses with our model and
    // gives us checked stats. The verbatim JSON is what gets re-emitted.
    let typed: Modlist =
        serde_json::from_slice(&modlist_bytes).context("Failed to parse Wabbajack modlist JSON")?;
    validate_modlist_paths(&typed)?;

    let mut manifest = serde_json::from_slice::<serde_json::Value>(&modlist_bytes)
        .context("Failed to parse Wabbajack modlist JSON")?
        .as_object()
        .context("Wabbajack modlist JSON is not an object")?
        .clone();

    // --- pass 1: hash every blob entry, building the BlobIndex ------------
    let mut blob_names: Vec<String> = Vec::new();
    for name in src.file_names() {
        let name = name.context("Failed to decode a container entry name")?;
        if name != MODLIST_ENTRY_NAME {
            blob_names.push(name.into_owned());
        }
    }
    let mut blob_index = Vec::with_capacity(blob_names.len());
    for name in &blob_names {
        let mut entry = src
            .by_name(name)
            .with_context(|| format!("Failed to read blob entry {name:?}"))?;
        let (size, hash) = hash_reader(&mut entry)?;
        blob_index.push(BlobIndexEntry {
            entry: name.clone(),
            size,
            hash,
        });
    }

    // --- manifest v1 fields ------------------------------------------------
    manifest.insert("FormatVersion".into(), serde_json::json!(FORMAT_VERSION));
    let license = match manifest.get("License") {
        Some(serde_json::Value::String(declared)) if !declared.trim().is_empty() => {
            declared.clone()
        }
        Some(_) => UNDECLARED_LICENSE.to_string(),
        None => DEFAULT_LICENSE.to_string(),
    };
    manifest.insert("License".into(), serde_json::json!(license));
    manifest.insert(
        "ToolVersion".into(),
        serde_json::json!(format!("modyssey {}", env!("CARGO_PKG_VERSION"))),
    );
    manifest.insert("BlobIndex".into(), serde_json::to_value(&blob_index)?);
    let manifest_bytes = serde_json::to_vec_pretty(&serde_json::Value::Object(manifest))
        .context("Failed to serialize manifest")?;

    // --- pass 2: write the .modyssey container -----------------------------
    let marker_bytes = serde_json::to_vec(&Marker::v1()).context("Failed to serialize marker")?;

    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create output directory {}", parent.display())
            })?;
        }
    }
    let out_file =
        File::create(output).with_context(|| format!("Failed to create {}", output.display()))?;
    let mut zip = ZipWriter::new(std::io::BufWriter::new(out_file));

    let deflate = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file(MARKER_ENTRY_NAME, deflate)
        .context("Failed to write marker entry")?;
    zip.write_all(&marker_bytes)?;
    zip.start_file(MANIFEST_ENTRY_NAME, deflate)
        .context("Failed to write manifest entry")?;
    zip.write_all(&manifest_bytes)?;

    let mut blob_bytes_copied = 0u64;
    for blob in &blob_index {
        let mut options =
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        if blob.size > ZIP64_ENTRY_SIZE_THRESHOLD {
            options = options.large_file(true);
        }
        zip.start_file(&blob.entry, options)
            .with_context(|| format!("Failed to start blob entry {:?}", blob.entry))?;
        let mut entry = src
            .by_name(&blob.entry)
            .with_context(|| format!("Failed to read blob entry {:?}", blob.entry))?;
        blob_bytes_copied += std::io::copy(&mut entry, &mut zip)
            .with_context(|| format!("Failed to copy blob entry {:?}", blob.entry))?
            as u64;
    }

    zip.finish().context("Failed to finish .modyssey file")?;

    let output_bytes = fs::metadata(output)
        .with_context(|| format!("Failed to stat {}", output.display()))?
        .len();

    info!(
        "Converted {} ({} blobs, {} bytes) -> {}",
        input.display(),
        blob_index.len(),
        blob_bytes_copied,
        output.display()
    );

    Ok(ConvertStats {
        modlist_name: typed.name,
        modlist_version: typed.version,
        archives: typed.archives.len(),
        directives: typed.directives.len(),
        blobs: blob_index.len(),
        blob_bytes: blob_bytes_copied,
        output_bytes,
    })
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn open_zip(path: &Path) -> Result<ZipArchive<BufReader<File>>> {
    let file = File::open(path).with_context(|| format!("Failed to open: {}", path.display()))?;
    ZipArchive::new(BufReader::new(file)).context("Failed to read as ZIP archive")
}

fn has_entry(archive: &ZipArchive<BufReader<File>>, name: &str) -> bool {
    archive
        .file_names()
        .any(|entry| entry.map(|entry| entry == name).unwrap_or(false))
}

/// A short human-readable summary of a container's entries for error
/// messages ("found 3 entries: a, b, c").
fn describe_entries(archive: &mut ZipArchive<BufReader<File>>) -> String {
    let names: Vec<String> = archive
        .file_names()
        .filter_map(|name| name.ok().map(|name| name.into_owned()))
        .take(5)
        .collect();
    let total = archive.len();
    if names.is_empty() {
        "The archive is empty.".to_string()
    } else if total > names.len() {
        format!(
            "Found {} entries, starting with: {}.",
            total,
            names
                .iter()
                .map(|n| format!("{n:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        format!(
            "Found {} entr{}: {}.",
            total,
            if total == 1 { "y" } else { "ies" },
            names
                .iter()
                .map(|n| format!("{n:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn hex_prefix(bytes: &[u8], len: usize) -> String {
    bytes[..len]
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Stream `reader` through xxHash64, returning `(size, hash)` with the hash
/// in the Wabbajack base64 encoding used throughout the engine.
fn hash_reader<R: Read>(reader: &mut R) -> Result<(u64, String)> {
    let mut hasher = xxhash_rust::xxh64::Xxh64::new(0);
    let mut buf = vec![0u8; 1024 * 1024];
    let mut size = 0u64;
    loop {
        let read = reader
            .read(&mut buf)
            .context("Failed to read while hashing")?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
        size += read as u64;
    }
    Ok((size, STANDARD.encode(hasher.digest().to_le_bytes())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Minimal Wabbajack-shaped manifest JSON (PascalCase, `$type` tags, no
    /// v1 fields) — the format subset.
    fn wj_manifest_json() -> serde_json::Value {
        serde_json::json!({
            "Name": "Test List", "Author": "Tester", "Description": "",
            "Version": "1.0.2", "WabbajackVersion": "4.3.2.2",
            "GameType": "DarkSoulsII", "IsNSFW": false,
            "Archives": [{
                "Hash": "AAAAAAAAAAA=", "Meta": "", "Name": "mod.7z", "Size": 17,
                "State": {"$type": "HttpDownloader, Wabbajack.Lib", "Url": "https://example.invalid/a"}
            }],
            "Directives": [{
                "$type": "InlineFile", "To": "mods\\a\\b.txt", "Hash": "AAAAAAAAAAA=",
                "Size": 3, "SourceDataID": "00000000-0000-0000-0000-000000000000"
            }]
        })
    }

    fn temp_path(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "modyssey-container-tests-{}-{tag}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        dir
    }

    /// Write a `.wabbajack` container: `modlist` + one blob per extra entry.
    fn write_wabbajack(path: &Path, manifest: &serde_json::Value, blobs: &[(&str, &[u8])]) {
        let file = File::create(path).unwrap();
        let mut zip = ZipWriter::new(std::io::BufWriter::new(file));
        let options = SimpleFileOptions::default();
        zip.start_file("modlist", options).unwrap();
        zip.write_all(serde_json::to_string(manifest).unwrap().as_bytes())
            .unwrap();
        for (name, data) in blobs {
            zip.start_file(*name, options).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }

    /// Write a `.modyssey` container: marker + manifest + blobs (stored).
    fn write_modyssey(
        path: &Path,
        marker: &Marker,
        manifest: &serde_json::Value,
        blobs: &[(&str, &[u8])],
    ) {
        let file = File::create(path).unwrap();
        let mut zip = ZipWriter::new(std::io::BufWriter::new(file));
        let deflate = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        zip.start_file(MARKER_ENTRY_NAME, deflate).unwrap();
        zip.write_all(serde_json::to_vec(marker).unwrap().as_slice())
            .unwrap();
        zip.start_file(MANIFEST_ENTRY_NAME, deflate).unwrap();
        zip.write_all(serde_json::to_vec(manifest).unwrap().as_slice())
            .unwrap();
        for (name, data) in blobs {
            zip.start_file(*name, stored).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }

    fn v1_manifest_json() -> serde_json::Value {
        let mut manifest = wj_manifest_json();
        let obj = manifest.as_object_mut().unwrap();
        obj.insert("FormatVersion".into(), serde_json::json!(FORMAT_VERSION));
        obj.insert("License".into(), serde_json::json!(DEFAULT_LICENSE));
        obj.insert("ToolVersion".into(), serde_json::json!("modyssey 9.9.9"));
        obj.insert(
            "BlobIndex".into(),
            serde_json::to_value(vec![BlobIndexEntry {
                entry: "00000000-0000-0000-0000-000000000000".into(),
                size: 3,
                hash: crate::hash::compute_bytes_hash(b"abc"),
            }])
            .unwrap(),
        );
        manifest
    }

    const BLOB: &str = "00000000-0000-0000-0000-000000000000";
    const BLOB_BYTES: &[u8] = b"abc";

    // ---- 1. marker round-trip ---------------------------------------------

    #[test]
    fn marker_serializes_to_the_spec_bytes() {
        assert_eq!(
            serde_json::to_string(&Marker::v1()).unwrap(),
            r#"{"format":"modyssey","version":1}"#
        );
        let marker: Marker = serde_json::from_str(r#"{"format":"modyssey","version":1}"#).unwrap();
        assert_eq!(marker, Marker::v1());
    }

    #[test]
    fn modyssey_file_round_trips() {
        let dir = temp_path("roundtrip");
        let path = dir.join("list.modyssey");
        write_modyssey(
            &path,
            &Marker::v1(),
            &v1_manifest_json(),
            &[(BLOB, BLOB_BYTES)],
        );

        let modlist = parse_modyssey_file(&path).unwrap();
        assert_eq!(modlist.name, "Test List");
        assert_eq!(modlist.format_version, Some(1));
        assert_eq!(modlist.license.as_deref(), Some(DEFAULT_LICENSE));
        assert_eq!(modlist.tool_version.as_deref(), Some("modyssey 9.9.9"));
        assert_eq!(modlist.blob_index.as_ref().map(|i| i.len()), Some(1));
        assert_eq!(modlist.archives.len(), 1);
        assert_eq!(modlist.directives.len(), 1);

        // A v1 file re-serialized through the typed model keeps the new
        // fields under their PascalCase names.
        let json = serde_json::to_value(&modlist).unwrap();
        assert_eq!(json["FormatVersion"], serde_json::json!(1));
        assert_eq!(json["License"], serde_json::json!(DEFAULT_LICENSE));
        assert_eq!(json["ToolVersion"], serde_json::json!("modyssey 9.9.9"));
        assert!(json["BlobIndex"].is_array());

        let _ = fs::remove_dir_all(&dir);
    }

    // ---- 2. WJ manifest parses as the subset ------------------------------

    #[test]
    fn wabbajack_manifest_parses_as_subset_with_new_fields_absent() {
        let json = wj_manifest_json();
        let modlist: Modlist = serde_json::from_value(json).unwrap();
        assert_eq!(modlist.format_version, None);
        assert_eq!(modlist.license, None);
        assert_eq!(modlist.blob_index, None);
        assert_eq!(modlist.tool_version, None);
        // The default notice applies at display time (MODLIST-POLICY.md).
        assert_eq!(modlist.effective_license(), DEFAULT_LICENSE);
    }

    #[test]
    fn empty_license_displays_as_undeclared() {
        let mut json = wj_manifest_json();
        json["License"] = serde_json::json!("  ");
        let modlist: Modlist = serde_json::from_value(json).unwrap();
        assert_eq!(modlist.effective_license(), UNDECLARED_LICENSE);

        let mut json = wj_manifest_json();
        json["License"] = serde_json::json!("MIT");
        let modlist: Modlist = serde_json::from_value(json).unwrap();
        assert_eq!(modlist.effective_license(), "MIT");
    }

    // ---- 3. BlobIndex mismatch detection -----------------------------------

    #[test]
    fn blob_index_mismatch_is_a_clear_error() {
        let dir = temp_path("blobindex");

        // wrong hash
        let mut manifest = v1_manifest_json();
        manifest["BlobIndex"][0]["Hash"] = serde_json::json!("BBBBBBBBBBB=");
        let path = dir.join("bad-hash.modyssey");
        write_modyssey(&path, &Marker::v1(), &manifest, &[(BLOB, BLOB_BYTES)]);
        let err = format!("{:#}", parse_modyssey_file(&path).unwrap_err());
        assert!(err.contains("xxHash64"), "{err}");
        assert!(err.contains(BLOB), "{err}");

        // wrong size
        let mut manifest = v1_manifest_json();
        manifest["BlobIndex"][0]["Size"] = serde_json::json!(4);
        let path = dir.join("bad-size.modyssey");
        write_modyssey(&path, &Marker::v1(), &manifest, &[(BLOB, BLOB_BYTES)]);
        let err = format!("{:#}", parse_modyssey_file(&path).unwrap_err());
        assert!(err.contains("size"), "{err}");

        // missing entry
        let mut manifest = v1_manifest_json();
        manifest["BlobIndex"][0]["Entry"] = serde_json::json!("11111111-1111-1111-111111111111");
        let path = dir.join("missing-entry.modyssey");
        write_modyssey(&path, &Marker::v1(), &manifest, &[(BLOB, BLOB_BYTES)]);
        let err = format!("{:#}", parse_modyssey_file(&path).unwrap_err());
        assert!(err.contains("missing from the container"), "{err}");

        // correct index passes
        let path = dir.join("good.modyssey");
        write_modyssey(
            &path,
            &Marker::v1(),
            &v1_manifest_json(),
            &[(BLOB, BLOB_BYTES)],
        );
        parse_modyssey_file(&path).unwrap();

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn absent_blob_index_is_accepted() {
        let dir = temp_path("no-blobindex");
        let mut manifest = v1_manifest_json();
        manifest.as_object_mut().unwrap().remove("BlobIndex");
        let path = dir.join("list.modyssey");
        write_modyssey(&path, &Marker::v1(), &manifest, &[(BLOB, BLOB_BYTES)]);
        parse_modyssey_file(&path).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    // ---- 4. FormatVersion gating -------------------------------------------

    #[test]
    fn newer_format_versions_are_rejected_with_clear_errors() {
        let dir = temp_path("versioning");

        // marker version 2
        let marker = Marker {
            format: FORMAT_NAME.into(),
            version: FORMAT_VERSION + 1,
        };
        let path = dir.join("marker-v2.modyssey");
        write_modyssey(&path, &marker, &v1_manifest_json(), &[(BLOB, BLOB_BYTES)]);
        let err = parse_modyssey_file(&path).unwrap_err().to_string();
        assert!(err.contains("newer Modyssey is required"), "{err}");

        // manifest FormatVersion 2
        let mut manifest = v1_manifest_json();
        manifest["FormatVersion"] = serde_json::json!(FORMAT_VERSION + 1);
        let path = dir.join("manifest-v2.modyssey");
        write_modyssey(&path, &Marker::v1(), &manifest, &[(BLOB, BLOB_BYTES)]);
        let err = parse_modyssey_file(&path).unwrap_err().to_string();
        assert!(err.contains("newer Modyssey is required"), "{err}");

        // manifest FormatVersion missing (required in v1 files)
        let mut manifest = v1_manifest_json();
        manifest.as_object_mut().unwrap().remove("FormatVersion");
        let path = dir.join("no-version.modyssey");
        write_modyssey(&path, &Marker::v1(), &manifest, &[(BLOB, BLOB_BYTES)]);
        let err = parse_modyssey_file(&path).unwrap_err().to_string();
        assert!(err.contains("FormatVersion"), "{err}");

        // wrong format name in marker
        let marker = Marker {
            format: "something-else".into(),
            version: 1,
        };
        let path = dir.join("wrong-format.modyssey");
        write_modyssey(&path, &marker, &v1_manifest_json(), &[(BLOB, BLOB_BYTES)]);
        let err = parse_modyssey_file(&path).unwrap_err().to_string();
        assert!(err.contains("not a Modyssey container"), "{err}");

        let _ = fs::remove_dir_all(&dir);
    }

    // ---- 5. convert-then-parse round-trip ----------------------------------

    #[test]
    fn convert_then_parse_round_trips() {
        let dir = temp_path("convert");
        let input = dir.join("list.wabbajack");
        let output = dir.join("list.modyssey");
        write_wabbajack(
            &input,
            &wj_manifest_json(),
            &[
                (BLOB, BLOB_BYTES),
                ("22222222-2222-2222-2222-222222222222", b"def"),
            ],
        );

        let stats = convert_wabbajack_to_modyssey(&input, &output).unwrap();
        assert_eq!(stats.modlist_name, "Test List");
        assert_eq!(stats.archives, 1);
        assert_eq!(stats.directives, 1);
        assert_eq!(stats.blobs, 2);
        assert_eq!(stats.blob_bytes, 6);
        assert!(stats.output_bytes > 0);

        // detection + parse of the produced file
        assert_eq!(
            detect_container_format(&output).unwrap(),
            ContainerFormat::Modyssey
        );
        let modlist = parse_modyssey_file(&output).unwrap();
        assert_eq!(modlist.name, "Test List");
        assert_eq!(modlist.wabbajack_version, "4.3.2.2"); // WJ field kept
        assert_eq!(modlist.format_version, Some(FORMAT_VERSION));
        assert_eq!(modlist.license.as_deref(), Some(DEFAULT_LICENSE));
        assert!(modlist
            .tool_version
            .as_deref()
            .unwrap()
            .starts_with("modyssey"));
        assert_eq!(modlist.blob_index.as_ref().map(|i| i.len()), Some(2));

        // WJ fields verbatim, including fields the typed model ignores
        let manifest: serde_json::Value = {
            let file = File::open(&output).unwrap();
            let mut zip = ZipArchive::new(BufReader::new(file)).unwrap();
            let mut bytes = Vec::new();
            zip.by_name(MANIFEST_ENTRY_NAME)
                .unwrap()
                .read_to_end(&mut bytes)
                .unwrap();
            serde_json::from_slice(&bytes).unwrap()
        };
        assert_eq!(manifest["WabbajackVersion"], serde_json::json!("4.3.2.2"));
        assert_eq!(manifest["GameType"], serde_json::json!("DarkSoulsII"));
        assert_eq!(manifest["FormatVersion"], serde_json::json!(1));

        // blobs copied byte-for-byte and stored uncompressed
        let file = File::open(&output).unwrap();
        let mut zip = ZipArchive::new(BufReader::new(file)).unwrap();
        for (name, expected) in [
            (BLOB, BLOB_BYTES),
            ("22222222-2222-2222-2222-222222222222", b"def"),
        ] {
            let mut entry = zip.by_name(name).unwrap();
            assert_eq!(entry.compression(), CompressionMethod::Stored);
            let mut data = Vec::new();
            entry.read_to_end(&mut data).unwrap();
            assert_eq!(data, expected);
        }

        // re-converting an already-converted file is rejected
        let err = convert_wabbajack_to_modyssey(&output, &output)
            .unwrap_err()
            .to_string();
        assert!(err.contains("already a .modyssey file"), "{err}");

        let _ = fs::remove_dir_all(&dir);
    }

    // ---- detection ----------------------------------------------------------

    #[test]
    fn detection_covers_extensions_and_content() {
        let dir = temp_path("detection");

        // .modyssey + marker
        let path = dir.join("a.modyssey");
        write_modyssey(
            &path,
            &Marker::v1(),
            &v1_manifest_json(),
            &[(BLOB, BLOB_BYTES)],
        );
        assert_eq!(
            detect_container_format(&path).unwrap(),
            ContainerFormat::Modyssey
        );

        // .wabbajack by extension
        let path = dir.join("b.wabbajack");
        write_wabbajack(&path, &wj_manifest_json(), &[]);
        assert_eq!(
            detect_container_format(&path).unwrap(),
            ContainerFormat::Wabbajack
        );

        // unknown extension, ZIP content with a modlist entry
        let path = dir.join("c.dat");
        write_wabbajack(&path, &wj_manifest_json(), &[]);
        assert_eq!(
            detect_container_format(&path).unwrap(),
            ContainerFormat::Wabbajack
        );

        // unknown extension, ZIP content with a marker
        let path = dir.join("d.dat");
        write_modyssey(&path, &Marker::v1(), &v1_manifest_json(), &[]);
        assert_eq!(
            detect_container_format(&path).unwrap(),
            ContainerFormat::Modyssey
        );

        // .modyssey extension but Wabbajack contents: clear error
        let path = dir.join("e.modyssey");
        write_wabbajack(&path, &wj_manifest_json(), &[]);
        let err = detect_container_format(&path).unwrap_err().to_string();
        assert!(err.contains("Rename it to .wabbajack"), "{err}");

        // unknown extension, not a zip: clear error naming the magic bytes
        let path = dir.join("f.dat");
        fs::write(&path, b"not an archive at all").unwrap();
        let err = detect_container_format(&path).unwrap_err().to_string();
        assert!(err.contains("magic bytes 6E 6F 74 20 61"), "{err}");

        // 7z signature with an unknown extension: clear error naming it
        let path = dir.join("g.dat");
        fs::write(&path, SEVENZIP_MAGIC).unwrap();
        let err = detect_container_format(&path).unwrap_err().to_string();
        assert!(err.contains("7z archive"), "{err}");

        // 7z signature with a .wabbajack extension: detection trusts the
        // extension (Wabbajack), and the WJ reader then fails on the zip
        // open — a clear error from the WJ path.
        let path = dir.join("g2.wabbajack");
        fs::write(&path, SEVENZIP_MAGIC).unwrap();
        assert_eq!(
            detect_container_format(&path).unwrap(),
            ContainerFormat::Wabbajack
        );
        let err = parse_modlist_container(&path).unwrap_err().to_string();
        assert!(err.contains("ZIP"), "{err}");

        // parse_modlist_container dispatches both ways
        let wj = dir.join("i.wabbajack");
        write_wabbajack(&wj, &wj_manifest_json(), &[]);
        assert_eq!(parse_modlist_container(&wj).unwrap().name, "Test List");
        let md = dir.join("j.modyssey");
        write_modyssey(
            &md,
            &Marker::v1(),
            &v1_manifest_json(),
            &[(BLOB, BLOB_BYTES)],
        );
        assert_eq!(parse_modlist_container(&md).unwrap().name, "Test List");

        let _ = fs::remove_dir_all(&dir);
    }
}
