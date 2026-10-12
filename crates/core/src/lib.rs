//! The rules of the OA Mod Launcher.
//!
//! So far this crate knows how to check a signature, download and verify a
//! file, unpack an archive safely, and find and validate the game folder. It
//! has no window code.
//!
//! All network access of the launcher is in [`net`].

pub mod archive;
pub mod error;
pub mod files;
pub mod game;
pub mod manifest;
pub mod net;
pub mod signature;
pub mod steam;
pub mod version;

pub use error::{CoreError, Result};
pub use manifest::Manifest;
pub use version::Version;
