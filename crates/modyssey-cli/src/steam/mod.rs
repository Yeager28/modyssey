//! Steam account login with a saved session.
//!
//! This is the foundation for fetching game files from Steam depots: it logs
//! in once (password plus Steam Guard), remembers Steam's refresh token, and
//! later reconnects from it without asking for anything.

// The binary compiles its own copy of this tree and only uses part of it.
#![allow(dead_code)]

pub mod cli;
pub mod session;
pub mod store;
