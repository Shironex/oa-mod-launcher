//! Removing: only the mod, or everything BepInEx brought into the game folder.
//!
//! Deleting from a folder the launcher does not own is the most dangerous
//! thing it does, so the rules are narrow:
//!
//! - **Only named things.** What is deleted is a fixed list of constants in
//!   [`crate::game::paths`], each joined to the canonical game folder through
//!   [`crate::files::inside`]. No name from the network, from the state file
//!   or from a setting ever becomes a path that is deleted.
//! - **Never through a link.** Both folders are scanned before the first
//!   delete. A junction or a symbolic link anywhere in them refuses the whole
//!   removal, and what it points at is not touched.
//! - **Other people's files are copied first.** Plugins, patchers and
//!   settings that are not the mod's go to `backups/<time>/` in the launcher's
//!   own folder, and the copy is compared with the original, checksum by
//!   checksum, before anything is deleted. A link where `backups/` belongs stops the removal.
//! - **Vanilla first.** `winhttp.dll` goes first: from that moment the game
//!   starts without BepInEx, whatever happens to the rest.
//! - **Again is fine.** A removal that was interrupted is finished by running
//!   it again.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::error::{CoreError, Result};
use crate::files;
use crate::game::paths;
use crate::inspect::{self, BepInEx};
use crate::launcher::{Activity, Launcher, Snapshot};
use crate::manifest;
use crate::state::State;

/// The root files in the order they are deleted: the proxy DLL first.
const ROOT_FILES_IN_REMOVAL_ORDER: [&str; 4] = [
    paths::WINHTTP,
    paths::DOORSTOP_CONFIG,
    ".doorstop_version",
    "changelog.txt",
];

/// The root files that only mean something next to the proxy DLL. They are
/// deleted with it or kept with it: a `winhttp.dll` the launcher does not
/// recognise would be left without the config it reads.
const LOADER_FILES: [&str; 2] = [paths::DOORSTOP_CONFIG, ".doorstop_version"];

/// What "remove only OA Mod" deletes, in order. The folder is removed after
/// them when it is empty.
const MOD_FILES: [&str; 4] = [
    paths::MOD_DLL,
    paths::MOD_PDB,
    paths::MOD_DLL_NEW,
    paths::MOD_CONFIG,
];

/// What kind of thing a foreign item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ForeignKind {
    /// Something in `BepInEx/plugins` that is not the mod.
    Plugin,
    /// Something in `BepInEx/patchers`.
    Patcher,
    /// A settings file in `BepInEx/config` that is neither BepInEx's own nor
    /// the mod's.
    Config,
}

/// Something in the BepInEx folder that somebody else put there. A full
/// removal copies it to the backup folder before it deletes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ForeignItem {
    /// Where it is, relative to the game folder, with `/` between the parts.
    pub path: String,
    /// What kind of thing it is.
    pub kind: ForeignKind,
    /// Its size in bytes, everything in it for a folder.
    pub bytes: u64,
}

/// What a removal would do, for the window to show before it asks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RemovalPlan {
    /// The game folder, as it is shown to a person.
    pub game_dir: String,
    /// What kind of BepInEx is there.
    pub bepinex: BepInEx,
    /// The files "remove only OA Mod" would delete, relative to the game
    /// folder. Only the ones that exist.
    pub mod_files: Vec<String>,
    /// The folders a full removal would delete with everything in them.
    pub folders: Vec<String>,
    /// The root files a full removal would delete: the ones whose content
    /// the launcher recognises.
    pub root_files: Vec<String>,
    /// Root files of BepInEx a full removal keeps unless it is told to
    /// include them: the ones whose content the launcher does not recognise,
    /// and the ones in [`RemovalPlan::kept_with_proxy`].
    pub unrecognised_root_files: Vec<String>,
    /// The part of [`RemovalPlan::unrecognised_root_files`] the launcher does
    /// recognise and keeps all the same, because `winhttp.dll` is one it does
    /// not recognise: `doorstop_config.ini` and `.doorstop_version` stay or
    /// go with the proxy DLL that reads them.
    pub kept_with_proxy: Vec<String>,
    /// What a full removal never deletes although it has a name of BepInEx,
    /// because it is not what the name says: a `dotnet` folder that holds
    /// files and no .NET runtime, a folder under the name of a root file, or a file
    /// under the name `dotnet` or `BepInEx`.
    pub kept: Vec<String>,
    /// Other people's plugins, patchers and settings. A full removal copies
    /// them to the backup folder first.
    pub foreign: Vec<ForeignItem>,
    /// Whether "remove only OA Mod" should be the default button: there are
    /// foreign items, or BepInEx is not the launcher's own.
    pub mod_only_recommended: bool,
}

/// What a full removal did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RemovalOutcome {
    /// What the window should show now.
    pub snapshot: Snapshot,
    /// What was left in the game folder on purpose, relative to it:
    /// [`RemovalPlan::unrecognised_root_files`] when they were not included
    /// (the loader files kept with an unrecognised `winhttp.dll` among them),
    /// and everything in [`RemovalPlan::kept`].
    pub kept: Vec<String>,
    /// Where other people's files were copied to, when there were any.
    pub backup_dir: Option<String>,
}

/// The plan with the paths it was made from.
struct Plan {
    shown: RemovalPlan,
    foreign: Vec<PathBuf>,
}

impl Launcher {
    /// What a removal would do. Reads the game folder and changes nothing.
    ///
    /// # Errors
    ///
    /// [`CoreError::GameNotFound`] without a game folder, and
    /// [`CoreError::UnsafePath`] when a link is where a file or folder of
    /// BepInEx belongs: nothing can be removed until it is gone.
    pub fn removal_preview(&self) -> Result<RemovalPlan> {
        let state = self.state()?;
        let located = self.locate(&state)?;
        Ok(plan(&located.root, &state, self.reference().as_ref())?.shown)
    }

    /// Remove the mod and its settings. BepInEx and every other plugin stay.
    ///
    /// # Errors
    ///
    /// [`CoreError::Busy`], [`CoreError::GameNotFound`],
    /// [`CoreError::SteamBusy`], [`CoreError::GameRunning`],
    /// [`CoreError::UnsafePath`], [`CoreError::NotWritable`]. A delete the
    /// folder refuses while the game runs is [`CoreError::GameRunning`].
    pub fn remove_mod(&self) -> Result<Snapshot> {
        let lease = self.lease(Activity::Removing)?;
        let removed = self.remove_mod_steps();
        drop(lease);
        removed.map_err(|error| self.explained(error))?;
        Ok(self.snapshot())
    }

    fn remove_mod_steps(&self) -> Result<()> {
        let game = self.writable_game()?;
        // Every path is proven before the first delete.
        let targets = MOD_FILES
            .iter()
            .map(|relative| files::inside(&game.root, relative))
            .collect::<Result<Vec<_>>>()?;
        let folder = files::inside(&game.root, paths::MOD_DIR)?;

        for target in &targets {
            files::delete_file(target)?;
        }
        // Only when it is empty: `remove_dir` removes nothing else, and a
        // file somebody else put next to the mod keeps the folder.
        let _ = std::fs::remove_dir(&folder);
        self.update_state(|state| state.mod_file = None)
    }

    /// Remove everything BepInEx brought into the game folder, the mod
    /// included. Other people's plugins, patchers and settings are copied to
    /// the backup folder first.
    ///
    /// Root files whose content the launcher does not recognise are kept
    /// unless `include_unrecognised` is set, and with an unrecognised
    /// `winhttp.dll` its config and version file are kept too; the outcome
    /// names what was kept.
    ///
    /// # Errors
    ///
    /// As for [`Launcher::remove_mod`], and
    /// [`CoreError::BackupIncomplete`] when the copy of other people's files
    /// does not match the originals. Nothing is deleted in that case, nor
    /// when a link is found in what would be deleted, nor when the game was
    /// started or Steam became busy while the backup was made.
    pub fn remove_all(&self, include_unrecognised: bool) -> Result<RemovalOutcome> {
        let lease = self.lease(Activity::Removing)?;
        let removed = self.remove_all_steps(include_unrecognised);
        drop(lease);
        let (kept, backup_dir) = removed.map_err(|error| self.explained(error))?;
        Ok(RemovalOutcome {
            snapshot: self.snapshot(),
            kept,
            backup_dir: backup_dir.as_deref().map(files::display),
        })
    }

    fn remove_all_steps(
        &self,
        include_unrecognised: bool,
    ) -> Result<(Vec<String>, Option<PathBuf>)> {
        let game = self.writable_game()?;
        let root = &game.root;
        let state = self.state()?;
        // Scans both folders: a link anywhere in them stops here.
        let Plan { shown, foreign } = plan(root, &state, self.reference().as_ref())?;

        let mut root_files = shown.root_files.clone();
        let mut kept = shown.kept.clone();
        if include_unrecognised {
            root_files.extend(shown.unrecognised_root_files.iter().cloned());
        } else {
            kept.extend(shown.unrecognised_root_files.iter().cloned());
        }

        let backup_dir = if foreign.is_empty() {
            None
        } else {
            let layout = self.layout();
            let backups = layout.checked(&layout.backups_dir())?;
            Some(back_up(&backups, root, &foreign)?)
        };

        // Everything is asked again: the backup can take a while, and the
        // game may have been started or Steam may have begun an update.
        if self.writable_game()?.root != *root {
            return Err(CoreError::GameNotFound);
        }
        // Not the names of the plan but the constants again, in the order of
        // the constants: the plan only says which of them to delete.
        for name in ROOT_FILES_IN_REMOVAL_ORDER {
            if root_files.iter().any(|planned| planned == name) {
                files::delete_file(&files::inside(root, name)?)?;
            }
        }
        // Both folders only when the plan lists them: what has one of the
        // two names and is not the folder is in `kept`, and is left alone.
        for folder in [paths::DOTNET, paths::BEPINEX] {
            if shown.folders.iter().any(|planned| planned == folder) {
                files::delete_tree(&files::inside(root, folder)?)?;
            }
        }

        let left = inspect::inspect(root, &State::default(), None)?;
        self.update_state(|state| {
            state.mod_file = None;
            state.bepinex = None;
            if left.bepinex.is_absent() && left.mod_sha256.is_none() {
                state.opted_out = true;
            }
        })?;
        Ok((kept, backup_dir))
    }
}

/// Look at the game folder `root` and say what a removal would do.
fn plan(root: &Path, state: &State, reference: Option<&manifest::BepInEx>) -> Result<Plan> {
    let facts = inspect::inspect(root, state, reference)?;

    let mut mod_files = Vec::new();
    for relative in MOD_FILES {
        if files::inside(root, relative)?.is_file() {
            mod_files.push(relative.to_owned());
        }
    }

    let mut root_files = Vec::new();
    let mut unrecognised_root_files = Vec::new();
    let mut kept_with_proxy = Vec::new();
    let mut kept = Vec::new();
    for name in ROOT_FILES_IN_REMOVAL_ORDER {
        let path = files::inside(root, name)?;
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.is_file() {
            kept.push(name.to_owned());
            continue;
        }
        let on_disk = files::sha256_file(&path)?.1;
        let recorded = state
            .bepinex
            .as_ref()
            .and_then(|record| record.root_files.get(name));
        let named = reference.and_then(|reference| reference.root_files.get(name));
        // The proxy DLL is the first name of the list, so by the time its
        // loader files are judged it is known whether it stays.
        let proxy_stays = unrecognised_root_files
            .iter()
            .any(|kept| kept == paths::WINHTTP);
        if recorded != Some(&on_disk) && named != Some(&on_disk) {
            unrecognised_root_files.push(name.to_owned());
        } else if proxy_stays && LOADER_FILES.contains(&name) {
            unrecognised_root_files.push(name.to_owned());
            kept_with_proxy.push(name.to_owned());
        } else {
            root_files.push(name.to_owned());
        }
    }

    let mut folders = Vec::new();
    let dotnet = files::inside(root, paths::DOTNET)?;
    if dotnet.exists() {
        // A folder named `dotnet` is BepInEx's runtime only when the runtime
        // is in it. Anything else under that name is not ours to delete,
        // unless there is no file in it at all: then nothing can be lost.
        let empty_or_runtime = dotnet.is_dir()
            && (files::inside(root, paths::DOTNET_PROOF)?.is_file()
                || files::scan_tree(&dotnet).is_ok_and(|tree| tree.files.is_empty()));
        if empty_or_runtime {
            files::scan_tree(&dotnet)?;
            folders.push(paths::DOTNET.to_owned());
        } else {
            kept.push(paths::DOTNET.to_owned());
        }
    }
    let bepinex = files::inside(root, paths::BEPINEX)?;
    if bepinex.is_dir() {
        files::scan_tree(&bepinex)?;
        folders.push(paths::BEPINEX.to_owned());
    } else if std::fs::symlink_metadata(&bepinex).is_ok() {
        // A file under the name of the folder: not BepInEx, not ours.
        kept.push(paths::BEPINEX.to_owned());
    }

    let mut foreign = Vec::new();
    let mut foreign_shown = Vec::new();
    for (folder, kind, ours) in [
        (
            paths::PLUGINS,
            ForeignKind::Plugin,
            &[paths::MOD_FOLDER_NAME][..],
        ),
        (paths::PATCHERS, ForeignKind::Patcher, &[][..]),
        (
            paths::CONFIG,
            ForeignKind::Config,
            &[paths::BEPINEX_CONFIG_NAME, paths::MOD_CONFIG_NAME][..],
        ),
    ] {
        let dir = files::inside(root, folder)?;
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut names: Vec<_> = entries.flatten().map(|entry| entry.file_name()).collect();
        names.sort();
        for name in names {
            let shown = name.to_string_lossy().into_owned();
            if ours.contains(&shown.as_str()) {
                continue;
            }
            // A name from a folder listing is one path part. The scan above
            // has already refused a link among them.
            let path = dir.join(&name);
            let bytes = if path.is_dir() {
                files::scan_tree(&path)?.bytes()
            } else {
                std::fs::metadata(&path).map_or(0, |metadata| metadata.len())
            };
            foreign_shown.push(ForeignItem {
                path: format!("{folder}/{shown}"),
                kind,
                bytes,
            });
            foreign.push(path);
        }
    }

    let mod_only_recommended =
        !foreign.is_empty() || !matches!(facts.bepinex, BepInEx::Absent | BepInEx::Ours { .. });
    Ok(Plan {
        shown: RemovalPlan {
            game_dir: files::display(root),
            bepinex: facts.bepinex,
            mod_files,
            folders,
            root_files,
            unrecognised_root_files,
            kept_with_proxy,
            kept,
            foreign: foreign_shown,
            mod_only_recommended,
        },
        foreign,
    })
}

/// Copy `items`, which are files and folders below `root`, into a new folder
/// under `backups`, keeping their paths relative to `root`. Copied, not
/// moved: the game can be on another drive.
///
/// The copy is compared with the originals, file by file, before the folder
/// is handed out. The caller deletes nothing unless this returns.
fn back_up(backups: &Path, root: &Path, items: &[PathBuf]) -> Result<PathBuf> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let mut target = backups.join(stamp(now));
    let mut attempt = 2;
    // Whatever is under that name, a dangling link included, is in the way.
    while std::fs::symlink_metadata(&target).is_ok() {
        target = backups.join(format!("{}-{attempt}", stamp(now)));
        attempt += 1;
    }
    const COPY: &str = "copy a file to the backup folder";

    // Every file with where it goes: (the copy, the original).
    let mut copies: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut folders: Vec<PathBuf> = Vec::new();
    let relative = |path: &Path| {
        path.strip_prefix(root)
            .map(|relative| target.join(relative))
            .map_err(|_| CoreError::UnsafePath {
                path: path.to_path_buf(),
                reason: "it is not in the game folder".to_owned(),
            })
    };
    for item in items {
        if item.is_dir() {
            let tree = files::scan_tree(item)?;
            folders.push(relative(item)?);
            for folder in &tree.dirs {
                folders.push(relative(folder)?);
            }
            for (file, _) in tree.files {
                copies.push((relative(&file)?, file));
            }
        } else {
            copies.push((relative(item)?, item.clone()));
        }
    }

    // `create_dir`, not `create_dir_all`, for the backup folder itself: it
    // must be new, so everything below it is what this call put there.
    std::fs::create_dir_all(backups).map_err(CoreError::io(COPY, backups))?;
    std::fs::create_dir(&target).map_err(CoreError::io(COPY, &target))?;
    for folder in &folders {
        std::fs::create_dir_all(folder).map_err(CoreError::io(COPY, folder))?;
    }
    for (to, from) in &copies {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(CoreError::io(COPY, parent))?;
        }
        std::fs::copy(from, to).map_err(CoreError::io(COPY, to))?;
    }

    verify_backup(&target, &copies)?;
    Ok(target)
}

/// Compare the backup with what it was made from: the same number of files,
/// and every file there with the SHA-256 of its original, read again from
/// both places.
fn verify_backup(target: &Path, copies: &[(PathBuf, PathBuf)]) -> Result<()> {
    let incomplete = || CoreError::BackupIncomplete {
        path: target.to_path_buf(),
    };
    let found = if target.exists() {
        files::scan_tree(target)?.files.len()
    } else {
        0
    };
    if found != copies.len() {
        return Err(incomplete());
    }
    for (copy, original) in copies {
        let copied = files::sha256_file(copy).map_err(|_| incomplete())?;
        if copied != files::sha256_file(original)? {
            return Err(incomplete());
        }
    }
    Ok(())
}

/// `seconds` since 1970 as `2026-10-12_15-09-19` in UTC: a folder name a
/// person can read and that sorts by time.
fn stamp(seconds: u64) -> String {
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    // Days since 1970-01-01 to a date of the Gregorian calendar, by the
    // usual era arithmetic (an era is 400 years, counted from a 1st of March).
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}_{:02}-{:02}-{:02}",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical_temp() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().expect("a temporary directory");
        let root = std::fs::canonicalize(temp.path()).expect("a real path");
        (temp, root)
    }

    fn put(root: &Path, relative: &str, body: &[u8]) {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
        std::fs::write(path, body).expect("a file");
    }

    #[test]
    fn the_stamp_is_the_utc_date_and_time() {
        for (seconds, text) in [
            (0, "1970-01-01_00-00-00"),
            (951_782_400, "2000-02-29_00-00-00"),
            (951_868_799, "2000-02-29_23-59-59"),
            (1_709_251_200, "2024-03-01_00-00-00"),
            (1_791_817_759, "2026-10-12_15-09-19"),
            (4_102_444_800, "2100-01-01_00-00-00"),
        ] {
            assert_eq!(stamp(seconds), text);
        }
    }

    #[test]
    fn the_proxy_dll_is_deleted_first_and_every_root_file_is_on_the_list() {
        assert_eq!(ROOT_FILES_IN_REMOVAL_ORDER[0], paths::WINHTTP);
        for name in LOADER_FILES {
            assert!(ROOT_FILES_IN_REMOVAL_ORDER[1..].contains(&name));
        }
        let mut ours = ROOT_FILES_IN_REMOVAL_ORDER.to_vec();
        let mut theirs = manifest::ROOT_FILES.to_vec();
        ours.sort_unstable();
        theirs.sort_unstable();
        assert_eq!(ours, theirs);
    }

    #[test]
    fn a_backup_holds_every_file_under_its_path_in_the_game_folder() {
        let (_temp, base) = canonical_temp();
        let game = base.join("game");
        put(&game, "BepInEx/plugins/Other/Other.dll", b"plugin");
        put(&game, "BepInEx/plugins/Other/data/deep.bin", b"deep");
        put(&game, "BepInEx/config/other.cfg", b"setting");
        std::fs::create_dir_all(game.join("BepInEx/plugins/Empty")).expect("an empty folder");
        let items = [
            game.join("BepInEx/plugins/Other"),
            game.join("BepInEx/plugins/Empty"),
            game.join("BepInEx/config/other.cfg"),
        ];

        let first = back_up(&base.join("backups"), &game, &items).expect("the backup is made");
        let second = back_up(&base.join("backups"), &game, &items).expect("and a second one");

        assert_ne!(first, second, "a backup never writes over an older one");
        for backup in [first, second] {
            assert_eq!(
                std::fs::read(backup.join("BepInEx/plugins/Other/data/deep.bin")).expect("copied"),
                b"deep"
            );
            assert_eq!(
                std::fs::read(backup.join("BepInEx/config/other.cfg")).expect("copied"),
                b"setting"
            );
            assert!(backup.join("BepInEx/plugins/Empty").is_dir());
            assert_eq!(files::scan_tree(&backup).expect("a scan").files.len(), 3);
        }
        // Copied, not moved.
        assert!(game.join("BepInEx/plugins/Other/Other.dll").is_file());
    }

    #[test]
    fn a_backup_that_does_not_match_its_original_is_refused() {
        let (_temp, base) = canonical_temp();
        let (backup, game) = (base.join("backup"), base.join("game"));
        for name in ["a.dll", "b.dll"] {
            put(&game, name, name.as_bytes());
            put(&backup, name, name.as_bytes());
        }
        put(&game, "c.dll", b"c.dll");
        let copy = |name: &str| (backup.join(name), game.join(name));
        let refused = |copies: &[(PathBuf, PathBuf)]| {
            matches!(
                verify_backup(&backup, copies),
                Err(CoreError::BackupIncomplete { .. })
            )
        };

        assert!(verify_backup(&backup, &[copy("a.dll"), copy("b.dll")]).is_ok());
        assert!(refused(&[copy("a.dll")]), "a file too many");
        assert!(
            refused(&[copy("a.dll"), copy("b.dll"), copy("c.dll")]),
            "a missing file"
        );
        assert!(refused(&[copy("a.dll"), copy("c.dll")]), "another file");

        put(&backup, "b.dll", b"b.d");
        assert!(refused(&[copy("a.dll"), copy("b.dll")]), "a short file");
        put(&backup, "b.dll", b"B.DLL");
        assert!(
            refused(&[copy("a.dll"), copy("b.dll")]),
            "the same length with other bytes"
        );
    }

    #[test]
    fn something_outside_the_game_folder_is_never_backed_up() {
        let (_temp, base) = canonical_temp();
        let game = base.join("game");
        std::fs::create_dir_all(&game).expect("the game");
        put(&base, "elsewhere/secret.txt", b"secret");

        let result = back_up(
            &base.join("backups"),
            &game,
            &[base.join("elsewhere").join("secret.txt")],
        );

        assert!(matches!(result, Err(CoreError::UnsafePath { .. })));
    }
}
