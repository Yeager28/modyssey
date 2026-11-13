//! Modyssey engine: modlist model, install pipeline, hashing, patching,
//! archive extraction, BSA/texture processing, settings and paths.
//!
//! Derivative work of CLF3 (MIT, © SulfurNitride) — see ATTRIBUTION.md and
//! LICENSE in the repository root.

pub mod archive;
pub mod bsa;
pub mod fluorine;
pub mod game_index;
pub mod gpu;
pub mod hash;
pub mod installer;
pub mod modlist;
pub mod octodiff;
pub mod paths;
pub mod secrets;
pub mod settings;
pub mod textures;

/// Download handlers (Nexus, Wabbajack CDN, HTTP, Mega, …) live in the
/// `modyssey-sources` crate; re-exported here under their historical
/// `modyssey::downloaders` path because the install pipeline drives them.
pub use modyssey_sources as downloaders;

/// Game detection lives in the `modyssey-games` crate; re-exported under its
/// historical `modyssey::game_finder` path.
pub use modyssey_games as game_finder;
