//! `state.json`: what the launcher installed, and the settings.
//!
//! The file is the launcher's memory of what it put into the game folder: the
//! mod build it wrote and whether BepInEx there is its own. What is really on
//! disk is read fresh every time (see [`crate::inspect`]); the state only says
//! which of it the launcher may call its own.
//!
//! It is written to a temporary file and renamed over the old one, so a crash
//! leaves either the old state or the new one, never half of each.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

/// The format number written to `state.json`.
pub const STATE_SCHEMA: u32 = 1;

/// Everything the launcher remembers between starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// Format number, see [`STATE_SCHEMA`].
    pub schema: u32,
    /// The game folder the player chose by hand. It wins over the folder
    /// Steam names for as long as it is a game folder, and it is kept until
    /// the player resets it.
    #[serde(default)]
    pub game_dir_override: Option<PathBuf>,
    /// Whether the launcher asks for updates when it opens.
    #[serde(default = "enabled")]
    pub check_on_start: bool,
    /// Set when the player removed everything: the launcher then offers the
    /// install quietly and does not push it.
    #[serde(default)]
    pub opted_out: bool,
    /// The mod build the launcher wrote.
    #[serde(default, rename = "mod")]
    pub mod_file: Option<ModRecord>,
    /// The BepInEx build the launcher wrote or took over.
    #[serde(default)]
    pub bepinex: Option<BepInExRecord>,
}

fn enabled() -> bool {
    true
}

impl Default for State {
    fn default() -> Self {
        Self {
            schema: STATE_SCHEMA,
            game_dir_override: None,
            check_on_start: true,
            opted_out: false,
            mod_file: None,
            bepinex: None,
        }
    }
}

/// The mod DLL the launcher wrote into the game folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModRecord {
    /// Its version, from the manifest.
    pub version: String,
    /// Its SHA-256. A DLL on disk with another hash is not this build.
    pub sha256: String,
}

/// Who put BepInEx into the game folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Owner {
    /// The launcher unpacked it.
    Launcher,
    /// It was there already, as the same build the mod needs.
    Adopted,
}

/// The BepInEx build the launcher wrote or took over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BepInExRecord {
    /// Who put it there.
    pub owner: Owner,
    /// Display text of the build, from the manifest.
    pub label: String,
    /// The SHA-256 of `BepInEx/core/BepInEx.Unity.IL2CPP.dll` of that build,
    /// so the copy on disk is recognised without a manifest too.
    pub probe_sha256: String,
    /// The SHA-256 of each root file of that build.
    #[serde(default)]
    pub root_files: BTreeMap<String, String>,
}

impl State {
    /// Read the state file. A missing file is a first run.
    ///
    /// A file that cannot be parsed is set aside as `state.json.broken` and
    /// the launcher starts as if it had installed nothing. That is safe: what
    /// is on disk is then treated as somebody else's, never as its own.
    ///
    /// # Errors
    ///
    /// [`CoreError::State`] when the file exists and cannot be read.
    pub fn load(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => String::new(),
            Err(error) => return Err(CoreError::State(error.to_string())),
        };
        match serde_json::from_str::<Self>(&text) {
            Ok(state) if state.schema == STATE_SCHEMA => Ok(state),
            _ => {
                let mut broken = path.as_os_str().to_owned();
                broken.push(".broken");
                let _ = std::fs::rename(path, broken);
                Ok(Self::default())
            }
        }
    }

    /// Write the state file through a temporary file and a rename.
    ///
    /// # Errors
    ///
    /// [`CoreError::Io`] when writing or renaming fails. The old file is
    /// intact in that case.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(CoreError::io("create the launcher folder", parent))?;
        }
        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);

        let text = serde_json::to_string_pretty(self).map_err(|error| CoreError::Io {
            operation: "write the state file",
            path: path.to_path_buf(),
            source: std::io::Error::other(error),
        })?;

        let write = || -> std::io::Result<()> {
            let mut file = std::fs::File::create(&temporary)?;
            file.write_all(text.as_bytes())?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, path)
        };
        write().map_err(CoreError::io("write the state file", path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_state() -> State {
        State {
            schema: STATE_SCHEMA,
            game_dir_override: Some(PathBuf::from(r"D:\Games\Golf With Your Friends")),
            check_on_start: false,
            opted_out: true,
            mod_file: Some(ModRecord {
                version: "0.1.0".to_owned(),
                sha256: "aa".repeat(32),
            }),
            bepinex: Some(BepInExRecord {
                owner: Owner::Adopted,
                label: "6.0.0-be.788".to_owned(),
                probe_sha256: "bb".repeat(32),
                root_files: BTreeMap::from([("winhttp.dll".to_owned(), "cc".repeat(32))]),
            }),
        }
    }

    #[test]
    fn the_file_round_trips_and_leaves_no_temporary_file() {
        let temp = tempfile::tempdir().expect("a temporary directory");
        let path = temp.path().join("nested").join("state.json");
        let state = full_state();

        state.save(&path).expect("the state is written");
        state
            .save(&path)
            .expect("writing over an existing file works");

        assert_eq!(State::load(&path).expect("the state is read"), state);
        let names: Vec<_> = std::fs::read_dir(path.parent().expect("a parent"))
            .expect("the folder is listed")
            .map(|entry| entry.expect("an entry").file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("state.json")]);
    }

    #[test]
    fn the_keys_on_disk_are_the_documented_ones() {
        let value = serde_json::to_value(full_state()).expect("the state is JSON");

        assert_eq!(value["mod"]["version"], "0.1.0");
        assert_eq!(value["bepinex"]["owner"], "adopted");
        assert_eq!(value["opted_out"], true);
        assert!(value.get("mod_file").is_none());
    }

    #[test]
    fn a_missing_file_is_a_first_run_and_a_broken_one_is_set_aside() {
        let temp = tempfile::tempdir().expect("a temporary directory");
        let path = temp.path().join("state.json");
        assert_eq!(State::load(&path).expect("a first run"), State::default());

        for broken in [&b"{ not json"[..], br#"{"schema":7}"#, &[0xff, 0xfe, 0x00]] {
            std::fs::write(&path, broken).expect("write a broken file");
            assert_eq!(
                State::load(&path).expect("a broken file is survivable"),
                State::default()
            );
            assert!(temp.path().join("state.json.broken").exists());
            assert!(!path.exists());
        }
    }

    #[test]
    fn a_file_that_cannot_be_read_is_an_error_and_is_left_alone() {
        let temp = tempfile::tempdir().expect("a temporary directory");
        let path = temp.path().join("state.json");
        // A folder under the name of the file: reading it fails for a reason
        // that is not "there is no file".
        std::fs::create_dir(&path).expect("a folder");

        assert!(matches!(State::load(&path), Err(CoreError::State(_))));
        assert!(path.is_dir());
    }

    #[test]
    fn an_older_file_without_new_fields_still_loads() {
        let temp = tempfile::tempdir().expect("a temporary directory");
        let path = temp.path().join("state.json");
        std::fs::write(&path, br#"{"schema":1}"#).expect("write the file");

        let state = State::load(&path).expect("the state is read");
        assert_eq!(state, State::default());
        assert!(state.check_on_start, "the default is to check on start");
    }
}
