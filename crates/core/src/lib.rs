//! The rules of the OA Mod Launcher.
//!
//! This crate finds the game, reads the signed release manifest, installs
//! BepInEx and the mod into the game folder, and removes them again. It has no
//! window code: the shell calls a [`Launcher`] and shows its [`Snapshot`].
//!
//! All network access of the launcher is in [`net`].

pub mod archive;
pub mod error;
pub mod files;
pub mod game;
pub mod layout;
pub mod manifest;
pub mod net;
pub mod signature;
pub mod state;
pub mod steam;
pub mod version;

pub use error::{CoreError, Result};
pub use layout::Layout;
pub use manifest::Manifest;
pub use state::State;
pub use version::Version;
