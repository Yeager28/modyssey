//! The Nexus `files.json` listing of a mod, shared by the API-key client and
//! the hosted protocol (where Fluorine does the authenticated request).

use serde::{Deserialize, Serialize};

/// Response of `GET /v1/games/{domain}/mods/{mod_id}/files.json`.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct NexusFileListing {
    #[serde(default)]
    pub files: Vec<NexusFile>,
    #[serde(default)]
    pub file_updates: Vec<NexusFileUpdate>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NexusFile {
    pub file_id: u64,
    /// Display name chosen by the author, e.g. `Cutting Room Floor - Modular`.
    #[serde(default)]
    pub name: Option<String>,
    /// Uploaded archive name, e.g. `Patch for Purists-45096-5-1-2-1672944563.7z`.
    #[serde(default)]
    pub file_name: Option<String>,
    /// `MAIN`, `UPDATE`, `OPTIONAL`, `MISCELLANEOUS`, `OLD_VERSION`, `ARCHIVED`
    /// or `DELETED`; absent on some removed files.
    #[serde(default)]
    pub category_name: Option<String>,
    #[serde(default)]
    pub uploaded_timestamp: i64,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub size_in_bytes: Option<u64>,
}

/// One edge of an author's "this file replaces that file" chain.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NexusFileUpdate {
    pub old_file_id: u64,
    pub new_file_id: u64,
}
