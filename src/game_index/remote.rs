//! Read a `.wabbajack` file's modlist straight off the Wabbajack CDN without
//! downloading the whole file.
//!
//! A `.wabbajack` is a zip, and the CDN stores it as independently fetchable
//! parts with known offsets. A zip's directory sits at the end and the
//! `modlist` entry is one compressed JSON file, so reading just that touches a
//! few percent of the parts (about 11 MB of a 382 MB list in testing). Every
//! part that is fetched is checked against the hash in the CDN definition.

use crate::downloaders::wabbajack_cdn::{validate_definition, CdnFileDefinition, CdnPart};
use crate::modlist::GameFileSourceState;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::VecDeque;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::sync::Arc;
use std::time::Duration;

/// Parts kept in memory. Zip reading is nearly sequential, so a few is plenty.
const CACHE_PARTS: usize = 4;
const FETCH_ATTEMPTS: u32 = 3;
const PART_TIMEOUT: Duration = Duration::from_secs(120);
/// Refuse a `modlist` entry larger than this when decompressed.
const MAX_MODLIST_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Where parts come from: HTTP in production, memory in tests.
pub trait PartSource {
    fn fetch(&mut self, part: &CdnPart) -> Result<Vec<u8>>;
}

/// Fetches parts from `{base}/parts/{index}` with retries.
pub struct HttpPartSource {
    client: reqwest::blocking::Client,
    base: String,
}

impl HttpPartSource {
    pub fn new(base_url: &str) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(concat!("ds2modpack/", env!("CARGO_PKG_VERSION")))
            .timeout(PART_TIMEOUT)
            .build()
            .context("Failed to create HTTP client")?;
        let base = crate::downloaders::wabbajack_cdn::WabbajackCdnDownloader::remap_url(base_url);
        Ok(Self {
            client,
            base: base.trim_end_matches('/').to_string(),
        })
    }
}

impl PartSource for HttpPartSource {
    fn fetch(&mut self, part: &CdnPart) -> Result<Vec<u8>> {
        let url = format!("{}/parts/{}", self.base, part.index);
        let mut last = None;
        for attempt in 0..FETCH_ATTEMPTS {
            if attempt > 0 {
                std::thread::sleep(Duration::from_secs(1 << attempt));
            }
            let result = self
                .client
                .get(&url)
                .send()
                .and_then(|r| r.error_for_status())
                .and_then(|r| r.bytes());
            match result {
                Ok(bytes) => return Ok(bytes.to_vec()),
                Err(e) => last = Some(e),
            }
        }
        Err(last.expect("at least one attempt")).with_context(|| format!("Fetching {url}"))
    }
}

/// `Read + Seek` over a CDN file, fetching only the parts that get read.
pub struct CdnRangeReader<S: PartSource> {
    source: S,
    parts: Vec<CdnPart>,
    size: u64,
    pos: u64,
    cache: VecDeque<(usize, Arc<Vec<u8>>)>,
    fetched_bytes: u64,
    fetched_parts: usize,
}

impl<S: PartSource> CdnRangeReader<S> {
    pub fn new(source: S, definition: &CdnFileDefinition) -> Result<Self> {
        validate_definition(definition)?;
        let mut parts = definition.parts.clone();
        parts.sort_by_key(|p| p.offset);
        Ok(Self {
            source,
            parts,
            size: definition.size,
            pos: 0,
            cache: VecDeque::new(),
            fetched_bytes: 0,
            fetched_parts: 0,
        })
    }

    /// Bytes downloaded so far, for reporting.
    pub fn fetched_bytes(&self) -> u64 {
        self.fetched_bytes
    }

    pub fn fetched_parts(&self) -> usize {
        self.fetched_parts
    }

    fn part_data(&mut self, slot: usize) -> Result<Arc<Vec<u8>>> {
        if let Some((_, data)) = self.cache.iter().find(|(s, _)| *s == slot) {
            return Ok(data.clone());
        }
        let part = self.parts[slot].clone();
        let data = self.source.fetch(&part)?;
        if data.len() != part.size {
            bail!(
                "Part {} is {} bytes, definition says {}",
                part.index,
                data.len(),
                part.size
            );
        }
        if crate::hash::compute_bytes_hash(&data) != part.hash {
            bail!("Part {} checksum mismatch", part.index);
        }
        self.fetched_bytes += data.len() as u64;
        self.fetched_parts += 1;
        let data = Arc::new(data);
        if self.cache.len() == CACHE_PARTS {
            self.cache.pop_front();
        }
        self.cache.push_back((slot, data.clone()));
        Ok(data)
    }
}

impl<S: PartSource> Read for CdnRangeReader<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.pos >= self.size {
            return Ok(0);
        }
        let pos = self.pos;
        let slot = self
            .parts
            .partition_point(|p| p.offset as u64 + p.size as u64 <= pos);
        let data = self.part_data(slot).map_err(io::Error::other)?;
        let start = (pos - self.parts[slot].offset as u64) as usize;
        let n = buf.len().min(data.len() - start);
        buf[..n].copy_from_slice(&data[start..start + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl<S: PartSource> Seek for CdnRangeReader<S> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let target = match from {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::End(d) => self.size.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        match target {
            Some(t) => {
                self.pos = t;
                Ok(t)
            }
            None => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before start of file",
            )),
        }
    }
}

/// A game file a modlist pins, with the archive size modlists record for it.
#[derive(Debug, Clone)]
pub struct PinnedGameFile {
    pub state: GameFileSourceState,
    pub size: u64,
}

/// The game-file facts of one modlist.
#[derive(Debug, Clone, Default)]
pub struct ModlistGameFiles {
    pub game_type: String,
    pub files: Vec<PinnedGameFile>,
}

#[derive(Deserialize)]
struct SlimModlist {
    #[serde(rename = "GameType", default)]
    game_type: String,
    #[serde(rename = "Archives", default)]
    archives: Vec<SlimArchive>,
}

#[derive(Deserialize)]
struct SlimArchive {
    #[serde(rename = "Size", default)]
    size: u64,
    #[serde(rename = "State")]
    state: serde_json::Value,
}

/// Read the `modlist` entry of a `.wabbajack` zip and return its pinned game
/// files, without parsing the (large) directive list.
pub fn read_game_files<R: Read + Seek>(reader: R) -> Result<ModlistGameFiles> {
    let mut zip = zip::ZipArchive::new(reader).context("Not a readable .wabbajack zip")?;
    let entry = zip
        .by_name("modlist")
        .context("No 'modlist' entry in .wabbajack")?;
    if entry.size() > MAX_MODLIST_BYTES {
        bail!(
            "modlist entry is {} bytes; refusing to read it",
            entry.size()
        );
    }
    // Bounded by the declared size, which the zip reader enforces as it inflates.
    let slim: SlimModlist = serde_json::from_reader(BufReader::new(entry.take(MAX_MODLIST_BYTES)))
        .context("Failed to parse modlist JSON")?;

    let mut out = ModlistGameFiles {
        game_type: slim.game_type,
        files: Vec::new(),
    };
    for archive in slim.archives {
        let is_game_file = archive
            .state
            .get("$type")
            .and_then(|t| t.as_str())
            .is_some_and(|t| t.starts_with("GameFileSourceDownloader"));
        if !is_game_file {
            continue;
        }
        // A malformed entry must not sink the whole list.
        if let Ok(state) = serde_json::from_value::<GameFileSourceState>(archive.state) {
            out.files.push(PinnedGameFile {
                state,
                size: archive.size,
            });
        }
    }
    Ok(out)
}

/// What a remote read cost and found.
#[derive(Debug, Clone, Default)]
pub struct RemoteScan {
    pub files: ModlistGameFiles,
    pub fetched_bytes: u64,
    pub total_bytes: u64,
}

/// Blocking: read a CDN-hosted modlist's game files with ranged part fetches.
/// Call from `spawn_blocking`.
pub fn scan_cdn_modlist(base_url: &str, definition: &CdnFileDefinition) -> Result<RemoteScan> {
    let source = HttpPartSource::new(base_url)?;
    let mut reader = CdnRangeReader::new(source, definition)?;
    let files = read_game_files(&mut reader)?;
    Ok(RemoteScan {
        files,
        fetched_bytes: reader.fetched_bytes(),
        total_bytes: definition.size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    const PART: usize = 512;

    struct MemSource {
        data: Vec<u8>,
        corrupt: Option<usize>,
    }

    impl PartSource for MemSource {
        fn fetch(&mut self, part: &CdnPart) -> Result<Vec<u8>> {
            let mut bytes = self.data[part.offset..part.offset + part.size].to_vec();
            if self.corrupt == Some(part.index) {
                bytes[0] ^= 0xff;
            }
            Ok(bytes)
        }
    }

    /// A zip with filler entries, then `modlist`, then more filler.
    fn sample_zip(game_files: usize) -> Vec<u8> {
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        // Large incompressible-looking filler before and after the modlist.
        let filler: Vec<u8> = (0..200_000u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
            .collect();
        w.start_file("first.bin", opts).unwrap();
        w.write_all(&filler).unwrap();

        let mut archives = vec![serde_json::json!({
            "Size": 5, "State": {"$type": "NexusDownloader, Wabbajack.Lib", "ModID": 1}
        })];
        for i in 0..game_files {
            archives.push(serde_json::json!({
                "Size": 1000 + i,
                "State": {
                    "$type": "GameFileSourceDownloader, Wabbajack.Lib",
                    "Game": "SkyrimSpecialEdition",
                    "GameFile": format!("Data\\f{i}.esm"),
                    "GameVersion": "1.6.640.0",
                    "Hash": "AQIDBAUGBwg="
                }
            }));
        }
        // A broken game-file entry must be skipped, not fatal.
        archives.push(serde_json::json!({
            "Size": 1, "State": {"$type": "GameFileSourceDownloader, Wabbajack.Lib", "Game": 7}
        }));
        let modlist = serde_json::json!({
            "GameType": "SkyrimSpecialEdition", "Archives": archives, "Directives": [{"x": 1}]
        });
        w.start_file("modlist", opts).unwrap();
        w.write_all(modlist.to_string().as_bytes()).unwrap();

        w.start_file("last.bin", opts).unwrap();
        w.write_all(&filler).unwrap();
        w.finish().unwrap().into_inner()
    }

    fn definition(data: &[u8]) -> CdnFileDefinition {
        let parts = data
            .chunks(PART)
            .enumerate()
            .map(|(index, chunk)| CdnPart {
                hash: crate::hash::compute_bytes_hash(chunk),
                index,
                offset: index * PART,
                size: chunk.len(),
            })
            .collect();
        CdnFileDefinition {
            author: String::new(),
            server_assigned_unique_id: None,
            hash: crate::hash::compute_bytes_hash(data),
            munged_name: String::new(),
            original_file_name: "x.wabbajack".into(),
            size: data.len() as u64,
            parts,
        }
    }

    #[test]
    fn reads_modlist_from_a_fraction_of_the_parts() {
        let data = sample_zip(3);
        let def = definition(&data);
        let mut reader = CdnRangeReader::new(
            MemSource {
                data: data.clone(),
                corrupt: None,
            },
            &def,
        )
        .unwrap();
        let found = read_game_files(&mut reader).unwrap();

        assert_eq!(found.game_type, "SkyrimSpecialEdition");
        assert_eq!(found.files.len(), 3);
        assert_eq!(found.files[0].state.game_file, "Data\\f0.esm");
        assert_eq!(found.files[2].size, 1002);
        // The two 200 KB fillers are never needed in full.
        assert!(
            reader.fetched_bytes() < data.len() as u64 / 4,
            "fetched {} of {}",
            reader.fetched_bytes(),
            data.len()
        );
    }

    #[test]
    fn corrupt_part_is_rejected() {
        let data = sample_zip(1);
        let def = definition(&data);
        let last = def.parts.len() - 1; // holds the zip directory
        let mut reader = CdnRangeReader::new(
            MemSource {
                data,
                corrupt: Some(last),
            },
            &def,
        )
        .unwrap();
        let err = read_game_files(&mut reader).unwrap_err();
        assert!(format!("{err:#}").contains("checksum"), "{err:#}");
    }

    #[test]
    fn seek_bounds_are_checked() {
        let data = sample_zip(1);
        let def = definition(&data);
        let mut reader = CdnRangeReader::new(
            MemSource {
                data,
                corrupt: None,
            },
            &def,
        )
        .unwrap();
        assert!(reader.seek(SeekFrom::Current(-1)).is_err());
        assert_eq!(reader.seek(SeekFrom::End(0)).unwrap(), def.size);
        let mut buf = [0u8; 4];
        assert_eq!(reader.read(&mut buf).unwrap(), 0);
    }
}
