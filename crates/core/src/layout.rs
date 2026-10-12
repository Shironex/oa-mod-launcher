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

use std::path::{Path, PathBuf};

/// Overrides the data root. Used by local runs so that nothing is written to
/// the real per-user folder.
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

    /// The layout the launcher uses when it is started normally: the override
    /// from [`ROOT_ENV`] when set, otherwise the per-user folder.
    pub fn from_environment() -> Option<Self> {
        if let Some(root) = std::env::var_os(ROOT_ENV).filter(|value| !value.is_empty()) {
            return Some(Self::new(PathBuf::from(root)));
        }
        default_root().map(Self::new)
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
        if std::env::var_os(ROOT_ENV).is_none_or(|value| value.is_empty()) {
            assert!(layout.root().ends_with(FOLDER));
        }
    }
}
