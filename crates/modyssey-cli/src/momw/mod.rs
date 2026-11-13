//! Modding-OpenMW (modding-openmw.com) list support.
//!
//! Modyssey reads the lists the site publishes (`/api/lists/<slug>` and
//! `/api/cfg-generator/<slug>`), plans the install, and produces a Fluorine
//! "Morrowind (OpenMW)" instance with one mod per OpenMW data directory.
//!
//! This is an independent implementation against the published JSON data; it
//! shares no code with the official `umo` downloader.

pub mod cli;
pub mod fetch;
pub mod hosted;
pub mod install;
pub mod instance;
pub mod lock;
pub mod nexus;
pub mod openmw;
pub mod pipeline;
pub mod plan;
pub mod schema;
pub mod source;
pub mod stage;
pub mod tools;

pub use plan::{build_plan, MomwPlan};
