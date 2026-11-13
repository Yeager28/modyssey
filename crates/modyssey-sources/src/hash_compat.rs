//! Hash helpers for the Wabbajack xxHash64 format, local to the sources crate.
//!
//! `modyssey-core::hash` is the canonical implementation; this copy exists so
//! `modyssey-sources` can verify downloaded artifacts without depending on
//! the core crate (core depends on sources for download dispatch — the edge
//! would be circular). Keep the two implementations in sync; both are
//! round-trip tested against Wabbajack's `hash-file` output.

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

/// Compute xxHash64 of a file and return as base64 string (Wabbajack format).
///
/// Uses streaming to handle large files without loading into memory.
pub fn compute_file_hash(path: &Path) -> Result<String> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open file for hashing: {}", path.display()))?;

    let mut reader = BufReader::with_capacity(8 * 1024 * 1024, file); // 8MB buffer
    let mut buf = vec![0u8; 8 * 1024 * 1024]; // 8MB chunks
    let mut hasher = xxhash_rust::xxh64::Xxh64::new(0);

    loop {
        let bytes_read = reader
            .read(&mut buf)
            .with_context(|| format!("Failed to read file for hashing: {}", path.display()))?;

        if bytes_read == 0 {
            break;
        }

        hasher.update(&buf[..bytes_read]);
    }

    let hash = hasher.digest();

    // Convert to base64 (little-endian bytes, like Wabbajack)
    let hash_bytes = hash.to_le_bytes();
    let base64_hash = STANDARD.encode(hash_bytes);

    Ok(base64_hash)
}

/// Verify a file's hash matches the expected Wabbajack hash.
///
/// Returns Ok(true) if hash matches, Ok(false) if mismatch.
pub fn verify_file_hash(path: &Path, expected_hash: &str) -> Result<bool> {
    let actual_hash = compute_file_hash(path)?;
    Ok(actual_hash == expected_hash)
}

/// Verify a file's hash and return detailed result.
///
/// Returns (matches, actual_hash) for logging/debugging.
pub fn verify_file_hash_detailed(path: &Path, expected_hash: &str) -> Result<(bool, String)> {
    let actual_hash = compute_file_hash(path)?;
    let matches = actual_hash == expected_hash;
    Ok((matches, actual_hash))
}
