//! Modyssey — an open-source, multi-game modlist installer (Wabbajack alternative).
//!
//! Derivative work of CLF3 (MIT, © SulfurNitride) — see ATTRIBUTION.md and
//! LICENSE in the repository root.
//!
//! This crate is the CLI surface (binary `modyssey`): it re-exports the
//! engine crates (`modyssey-core`, `modyssey-sources`, `modyssey-games`) so
//! the historical `modyssey::…` paths keep working, and owns the tooling
//! modules that only the CLI uses (MOMW, TTW, collections, Steam login and
//! the egui helpers).

pub use modyssey_core::{
    archive, bsa, downloaders, fluorine, game_finder, game_index, gpu, hash, installer, modlist,
    octodiff, paths, secrets, settings, textures,
};

pub mod browser_gui;
pub mod collection;
pub mod collection_app;
pub mod collections_gui;
pub mod momw;
pub mod momw_gui;
pub mod steam;
pub mod ttw;
