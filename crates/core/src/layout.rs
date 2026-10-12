//! Where the launcher keeps its own things on disk.
//!
//! ```text
//! <root>/
//!   state.json            what the launcher installed and the settings
//!   staging/              downloads and unpacking
//!   cache/manifest.json   the last manifest that was read, for offline starts
//!   backups/<time>/       other people's plugins, copied before a full removal
//!   launcher.log
//! ```
//!
//! Per user, no administrator rights. The mod itself does not live here: it
//! goes into the game folder, see [`crate::game`].
//!
//! Where the root is, is the user's business: the root or a folder above it
//! may be a link. Nothing below the root may be one. Before the launcher
//! writes to or deletes one of these paths it asks [`Layout::checked`], which
//! refuses a path that goes through a junction or a symbolic link.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::error::{CoreError, Result};
use crate::files;

/// Overrides the data root, in a development build only. Used by local runs
/// so that nothing is written to the real per-user folder. A release build
/// does not read it, for the reason it does not trust the development key:
/// nothing in the environment may redirect what it writes and deletes.
pub const ROOT_ENV: &str = "OA_MOD_LAUNCHER_ROOT";

/// The name of the folder under the per-user application data directory.
const FOLDER: &str = "OaModLauncher";

/// The directory layout under one data root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    /// A layout under `root`. Nothing is created until it is needed.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The layout the launcher uses when it is started normally: the per-user
    /// folder, or in a development build the override from [`ROOT_ENV`] when
    /// it is set.
    pub fn from_environment() -> Option<Self> {
        root_from(std::env::var_os(ROOT_ENV), cfg!(debug_assertions)).map(Self::new)
    }

    /// `path`, which is one of the paths of this layout, once it is proven
    /// that no part of it below the root is a link. Asked before every write
    /// and every delete in the data root.
    ///
    /// # Errors
    ///
    /// [`CoreError::UnsafePath`] when a junction or a symbolic link is where
    /// one of the launcher's own files or folders belongs.
    pub fn checked(&self, path: &Path) -> Result<PathBuf> {
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| CoreError::UnsafePath {
                path: path.to_path_buf(),
                reason: "it is not in the launcher's own folder".to_owned(),
            })?;
        files::inside(&self.root, relative)
    }

    /// The data root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `state.json`.
    pub fn state_file(&self) -> PathBuf {
        self.root.join("state.json")
    }

    /// `staging/`.
    pub fn staging_dir(&self) -> PathBuf {
        self.root.join("staging")
    }

    /// `cache/manifest.json`.
    pub fn cached_manifest(&self) -> PathBuf {
        self.root.join("cache").join("manifest.json")
    }

    /// `backups/`.
    pub fn backups_dir(&self) -> PathBuf {
        self.root.join("backups")
    }

    /// `launcher.log`.
    pub fn launcher_log(&self) -> PathBuf {
        self.root.join("launcher.log")
    }
}

/// The data root: `override_root` in a development build when it is set,
/// otherwise the per-user folder.
fn root_from(override_root: Option<OsString>, development: bool) -> Option<PathBuf> {
    match override_root.filter(|value| !value.is_empty()) {
        Some(root) if development => Some(PathBuf::from(root)),
        _ => default_root(),
    }
}

/// `%LOCALAPPDATA%\OaModLauncher` on Windows.
#[cfg(windows)]
fn default_root() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty())?;
    Some(PathBuf::from(base).join(FOLDER))
}

/// Other systems are not release targets. The data directory under the home
/// folder keeps a development build from writing into the home folder itself.
#[cfg(not(windows))]
fn default_root() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|value| !value.is_empty())?;
    Some(
        PathBuf::from(home)
            .join(".local")
            .join("share")
            .join(FOLDER),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_path_stays_under_the_root() {
        let layout = Layout::new("root");
        for path in [
            layout.state_file(),
            layout.staging_dir(),
            layout.cached_manifest(),
            layout.backups_dir(),
            layout.launcher_log(),
        ] {
            assert!(
                path.starts_with("root"),
                "{} is outside the root",
                path.display()
            );
        }
    }

    #[test]
    fn the_default_root_is_the_launcher_folder_of_this_user() {
        // Read only: the test never sets a variable, so it holds whatever the
        // override is set to on the machine that runs it.
        let layout = Layout::from_environment().expect("a data root");
        if !cfg!(debug_assertions)
            || std::env::var_os(ROOT_ENV).is_none_or(|value| value.is_empty())
        {
            assert!(layout.root().ends_with(FOLDER));
        }
    }

    #[test]
    fn only_a_development_build_takes_the_data_root_from_the_environment() {
        let elsewhere = || Some(OsString::from("elsewhere"));

        assert_eq!(
            root_from(elsewhere(), true),
            Some(PathBuf::from("elsewhere"))
        );
        let released = root_from(elsewhere(), false).expect("a data root");
        assert!(released.ends_with(FOLDER), "a release build ignores it");
        // Unset or empty is no override in either build.
        for development in [true, false] {
            for unset in [None, Some(OsString::new())] {
                let root = root_from(unset, development).expect("a data root");
                assert!(root.ends_with(FOLDER));
            }
        }
    }

    #[test]
    fn a_path_of_the_layout_through_a_link_is_refused() {
        let temp = tempfile::tempdir().expect("a temporary directory");
        let layout = Layout::new(temp.path().join("data"));
        let elsewhere = temp.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).expect("another folder");
        std::fs::create_dir_all(layout.root()).expect("the data root");

        assert!(
            layout.checked(&layout.staging_dir()).is_ok(),
            "not there yet"
        );
        files::tests::link_dir(&elsewhere, &layout.staging_dir());
        files::tests::link_dir(&elsewhere, &layout.root().join("cache"));

        for path in [
            layout.staging_dir(),
            layout.staging_dir().join("bepinex"),
            layout.cached_manifest(),
            elsewhere.join("state.json"),
        ] {
            assert!(
                matches!(layout.checked(&path), Err(CoreError::UnsafePath { .. })),
                "{} must be refused",
                path.display()
            );
        }
        assert!(layout.checked(&layout.state_file()).is_ok());
        assert!(layout.checked(&layout.backups_dir()).is_ok());
    }
}
