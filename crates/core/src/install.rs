//! Installing and updating: BepInEx when the game has none, then the mod.
//!
//! Every download goes to the launcher's own `staging/` folder first and is
//! compared with the size and the SHA-256 the signed manifest names. Only
//! then is anything written into the game folder, in an order that leaves the
//! game either as it was or working:
//!
//! 1. Nothing of BepInEx is written when any of its files is already there.
//! 2. `dotnet/` and `BepInEx/` are copied first, the small root files next and
//!    `winhttp.dll` last. That file is what makes the game load BepInEx, so a
//!    copy that stops half way never loads.
//! 3. When a copy fails, exactly the files and folders it made are removed
//!    again, in reverse.
//! 4. The mod is written next to its final name and renamed over it.
//!
//! An update is the same flow: with BepInEx in place, only the mod is
//! replaced.

use std::path::{Path, PathBuf};

use crate::archive;
use crate::error::{CoreError, Result};
use crate::files;
use crate::game::paths;
use crate::inspect::{self, BepInEx};
use crate::launcher::{Activity, Launcher, Phase, Progress, Snapshot, clean_staging};
use crate::manifest::{self, Manifest, ROOT_FILES};
use crate::net;
use crate::state::{BepInExRecord, ModRecord, Owner};

type Report<'a> = &'a (dyn Fn(Phase, u64, u64) + Send + Sync);

impl Launcher {
    /// Install the mod, or replace it with the build the manifest names.
    /// BepInEx is installed first when the game folder has none.
    ///
    /// The manifest is read fresh at the start, so the install never works
    /// from what an earlier check found.
    ///
    /// # Errors
    ///
    /// [`CoreError::Busy`] while another step runs. The errors of the check
    /// ([`CoreError::BadSignature`], [`CoreError::BadManifest`],
    /// [`CoreError::LauncherTooOld`], [`CoreError::Http`]). Before anything is
    /// written: [`CoreError::GameNotFound`], [`CoreError::SteamBusy`],
    /// [`CoreError::GameRunning`], [`CoreError::UnsupportedBepInEx`],
    /// [`CoreError::Conflict`], [`CoreError::UnsafePath`]. For a download:
    /// [`CoreError::SizeMismatch`], [`CoreError::HashMismatch`],
    /// [`CoreError::Archive`]. After any of them the game folder holds what it
    /// held before, except that a BepInEx that was installed completely stays
    /// when only the mod failed.
    pub async fn install(
        &self,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<Snapshot> {
        let lease = self.lease(Activity::Installing(Phase::Checking))?;
        let report = |phase: Phase, received: u64, total: u64| {
            self.set_activity(Activity::Installing(phase));
            on_progress(Progress {
                phase,
                received,
                total,
            });
        };

        report(Phase::Checking, 0, 0);
        let result = match self.check_now().await {
            Ok(manifest) => self.install_steps(&manifest, &report).await,
            Err(error) => Err(error),
        };
        clean_staging(self.layout());
        drop(lease);
        result.map_err(|error| self.explained(error))?;
        Ok(self.snapshot())
    }

    async fn install_steps(&self, manifest: &Manifest, report: Report<'_>) -> Result<()> {
        let game = self.writable_game()?;
        let state = self.state()?;
        let facts = inspect::inspect(&game.root, &state, Some(&manifest.bepinex))?;

        let staging = self.layout().staging_dir();
        clean_staging(self.layout());
        std::fs::create_dir_all(&staging)
            .map_err(CoreError::io("create the staging folder", &staging))?;

        match &facts.bepinex {
            BepInEx::Unsupported { reason } => {
                return Err(CoreError::UnsupportedBepInEx(reason.clone()));
            }
            BepInEx::Absent => {
                let zip = staging.join("bepinex.zip.part");
                let bepinex = &manifest.bepinex;
                self.fetch(
                    &bepinex.url,
                    bepinex.size,
                    &bepinex.sha256,
                    &zip,
                    Phase::DownloadingBepInEx,
                    report,
                )
                .await?;

                report(Phase::InstallingBepInEx, 0, 0);
                let unpacked = staging.join("bepinex");
                let (root, expected) = (game.root.clone(), bepinex.clone());
                let running = self.config.running.clone();
                tokio::task::spawn_blocking(move || {
                    archive::extract(&zip, &unpacked)?;
                    check_unpacked(&unpacked, &expected)?;
                    place(&unpacked, &root, &|| {
                        if running(&root) {
                            Err(CoreError::GameRunning)
                        } else {
                            Ok(())
                        }
                    })
                })
                .await
                .map_err(|error| CoreError::Archive(format!("unpacking stopped: {error}")))??;

                // Written before the mod is fetched: whatever happens next,
                // the launcher knows this BepInEx is its own.
                self.update_state(|state| {
                    state.bepinex = Some(record(Owner::Launcher, bepinex, &bepinex.probe_sha256));
                })?;
            }
            BepInEx::Adopted { .. } => {
                // Recognised by the manifest and not yet on record: remember
                // that it is the pinned build and that it is not ours.
                let on_record = state.bepinex.as_ref().map(|record| &record.probe_sha256);
                if let Some(on_disk) = &facts.probe_sha256
                    && on_record != Some(on_disk)
                {
                    self.update_state(|state| {
                        state.bepinex = Some(record(Owner::Adopted, &manifest.bepinex, on_disk));
                    })?;
                }
            }
            // Ours is on record already. A foreign build is used as it is and
            // never recorded: the snapshot carries the warning.
            BepInEx::Ours { .. } | BepInEx::Foreign => {}
        }

        let wanted = &manifest.mod_file;
        if facts.mod_sha256.as_deref() != Some(wanted.sha256.as_str()) {
            let staged = staging.join("OaMod.dll.part");
            self.fetch(
                &wanted.url,
                wanted.size,
                &wanted.sha256,
                &staged,
                Phase::DownloadingMod,
                report,
            )
            .await?;

            report(Phase::InstallingMod, 0, 0);
            self.refuse_while_running(&game.root)?;
            write_mod(&game.root, &staged)?;
        }

        self.update_state(|state| {
            state.mod_file = Some(ModRecord {
                version: manifest.version.clone(),
                sha256: wanted.sha256.clone(),
            });
            state.opted_out = false;
        })
    }

    /// Download `url` to `destination` and compare it with the manifest.
    async fn fetch(
        &self,
        url: &str,
        size: u64,
        sha256: &str,
        destination: &Path,
        phase: Phase,
        report: Report<'_>,
    ) -> Result<()> {
        let url = net::checked_url(url)?;
        report(phase, 0, size);
        let downloaded = net::download(&self.client, &url, destination, size, &|received| {
            report(phase, received, size);
        })
        .await?;

        if downloaded.bytes != size {
            return Err(CoreError::SizeMismatch {
                expected: size,
                actual: downloaded.bytes,
            });
        }
        if downloaded.sha256 != sha256 {
            return Err(CoreError::HashMismatch {
                expected: sha256.to_owned(),
                actual: downloaded.sha256,
            });
        }
        Ok(())
    }
}

fn record(owner: Owner, bepinex: &manifest::BepInEx, probe_sha256: &str) -> BepInExRecord {
    BepInExRecord {
        owner,
        label: bepinex.label.clone(),
        probe_sha256: probe_sha256.to_owned(),
        root_files: bepinex.root_files.clone(),
    }
}

/// Check that the unpacked zip is the BepInEx build the manifest describes:
/// nothing at its top level but the two folders and the four root files, and
/// the probe and the root files with the checksums of the manifest.
fn check_unpacked(unpacked: &Path, expected: &manifest::BepInEx) -> Result<()> {
    let refused = |reason: String| CoreError::Archive(reason);

    let entries = std::fs::read_dir(unpacked)
        .map_err(CoreError::io("list the unpacked archive", unpacked))?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let known = [paths::BEPINEX, paths::DOTNET]
            .iter()
            .chain(&ROOT_FILES)
            .any(|known| name == std::ffi::OsStr::new(known));
        if !known {
            return Err(refused(format!(
                "it holds \"{}\", which is not part of BepInEx",
                name.to_string_lossy()
            )));
        }
    }
    for folder in [paths::BEPINEX, paths::DOTNET] {
        if !unpacked.join(folder).is_dir() {
            return Err(refused(format!("it has no {folder} folder")));
        }
    }

    let wanted = ROOT_FILES
        .iter()
        .map(|name| (*name, expected.root_files.get(*name)))
        .chain([(paths::PROBE, Some(&expected.probe_sha256))]);
    for (relative, sha256) in wanted {
        let file = unpacked.join(relative);
        if !file.is_file() {
            return Err(refused(format!("it does not contain {relative}")));
        }
        if Some(&files::sha256_file(&file)?.1) != sha256 {
            return Err(refused(format!(
                "{relative} in it is not the file the release names"
            )));
        }
    }
    Ok(())
}

/// Copy the unpacked BepInEx into the game folder `root`.
///
/// `still_allowed` is asked once more after every target was checked and
/// before the first byte is written.
fn place(unpacked: &Path, root: &Path, still_allowed: &dyn Fn() -> Result<()>) -> Result<()> {
    // What to copy, in the order it is written: the two folders, then the
    // root files with `winhttp.dll` last.
    let mut relatives: Vec<PathBuf> = Vec::new();
    for folder in [paths::DOTNET, paths::BEPINEX] {
        for (file, _) in files::scan_tree(&unpacked.join(folder))?.files {
            let relative = file
                .strip_prefix(unpacked)
                .map_err(|_| CoreError::Archive("an unpacked file is out of place".to_owned()))?;
            relatives.push(relative.to_path_buf());
        }
    }
    relatives.extend(ROOT_FILES.iter().map(PathBuf::from));

    // Every target is proven to be inside the game folder and free before
    // anything is written. A folder may exist; a file may not.
    let mut targets = Vec::with_capacity(relatives.len());
    for relative in &relatives {
        let target = files::inside(root, relative)?;
        if std::fs::symlink_metadata(&target).is_ok() {
            return Err(CoreError::Conflict { path: target });
        }
        targets.push((unpacked.join(relative), target));
    }
    still_allowed()?;

    let mut written: Vec<&Path> = Vec::new();
    let mut made: Vec<PathBuf> = Vec::new();
    let copied = targets.iter().try_for_each(|(source, target)| {
        make_parents(root, target, &mut made)?;
        let mut to = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)
            .map_err(files::write_error("write into the game folder", target))?;
        // On the list from the moment it exists, so a copy that stops half
        // way is removed too.
        written.push(target);
        let mut from = std::fs::File::open(source)
            .map_err(CoreError::io("read the unpacked archive", source))?;
        std::io::copy(&mut from, &mut to)
            .map_err(files::write_error("write into the game folder", target))?;
        to.sync_all()
            .map_err(files::write_error("write into the game folder", target))
    });

    if copied.is_err() {
        // Exactly what this call made, newest first. Best effort: what
        // cannot be removed now is found by the next look at the folder.
        for file in written.iter().rev() {
            let _ = std::fs::remove_file(file);
        }
        for folder in made.iter().rev() {
            let _ = std::fs::remove_dir(folder);
        }
    }
    copied
}

/// Create the folders `target` needs below `root`, one level at a time, and
/// note each one that was made.
fn make_parents(root: &Path, target: &Path, made: &mut Vec<PathBuf>) -> Result<()> {
    let mut missing: Vec<&Path> = target
        .ancestors()
        .skip(1)
        .take_while(|folder| *folder != root && !folder.is_dir())
        .collect();
    missing.reverse();
    for folder in missing {
        std::fs::create_dir(folder).map_err(files::write_error(
            "create a folder in the game folder",
            folder,
        ))?;
        made.push(folder.to_path_buf());
    }
    Ok(())
}

/// Put the downloaded mod into the game folder: written next to its final
/// name and renamed over it, so the plugin is the old build or the new one
/// and never half of each.
fn write_mod(root: &Path, staged: &Path) -> Result<()> {
    let folder = files::inside(root, paths::MOD_DIR)?;
    let new = files::inside(root, paths::MOD_DLL_NEW)?;
    let target = files::inside(root, paths::MOD_DLL)?;

    std::fs::create_dir_all(&folder)
        .map_err(files::write_error("create the mod folder", &folder))?;
    // A leftover of a run that was interrupted.
    files::delete_file(&new)?;
    std::fs::copy(staged, &new).map_err(files::write_error("write the mod", &new))?;
    files::rename_with_retries(&new, &target, "put the mod into place").inspect_err(|_| {
        let _ = files::delete_file(&new);
    })
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

    fn unpacked_in(base: &Path) -> PathBuf {
        let unpacked = base.join("unpacked");
        for relative in [
            "dotnet/coreclr.dll",
            "dotnet/shared/deep/System.dll",
            "BepInEx/core/BepInEx.Unity.IL2CPP.dll",
            "winhttp.dll",
            "doorstop_config.ini",
            ".doorstop_version",
            "changelog.txt",
        ] {
            put(&unpacked, relative, relative.as_bytes());
        }
        unpacked
    }

    fn allowed() -> Result<()> {
        Ok(())
    }

    #[test]
    fn a_copy_that_fails_removes_exactly_what_it_made() {
        let (_temp, base) = canonical_temp();
        let unpacked = unpacked_in(&base);
        let game = base.join("game");
        // The game already has a BepInEx folder with somebody's config in it,
        // and that must be all that is left afterwards.
        put(&game, "BepInEx/config/theirs.cfg", b"keep");
        // The last file to be copied cannot be read, so the copy fails after
        // everything else was written.
        std::fs::remove_file(unpacked.join("winhttp.dll")).expect("remove");
        std::fs::create_dir(unpacked.join("winhttp.dll")).expect("a folder in its place");

        let result = place(&unpacked, &game, &allowed);

        assert!(result.is_err());
        assert_eq!(
            std::fs::read(game.join("BepInEx/config/theirs.cfg")).expect("still there"),
            b"keep"
        );
        for gone in [
            "dotnet",
            "BepInEx/core",
            "doorstop_config.ini",
            "winhttp.dll",
        ] {
            assert!(!game.join(gone).exists(), "{gone} was rolled back");
        }
    }

    #[test]
    fn nothing_is_written_when_the_game_started_meanwhile() {
        let (_temp, base) = canonical_temp();
        let unpacked = unpacked_in(&base);
        let game = base.join("game");
        std::fs::create_dir(&game).expect("the game");

        let result = place(&unpacked, &game, &|| Err(CoreError::GameRunning));

        assert!(matches!(result, Err(CoreError::GameRunning)));
        assert_eq!(std::fs::read_dir(&game).expect("listed").count(), 0);
    }

    #[test]
    fn the_proxy_dll_is_written_last() {
        // The order is what makes a half finished install harmless, so it is
        // pinned here: the root files come after both folders, winhttp.dll
        // after everything.
        assert_eq!(ROOT_FILES.last(), Some(&paths::WINHTTP));
    }

    #[test]
    fn the_mod_replaces_the_old_build_and_leaves_no_temporary_file() {
        let (_temp, base) = canonical_temp();
        let staged = base.join("OaMod.dll.part");
        std::fs::write(&staged, b"new build").expect("the download");
        let game = base.join("game");
        put(&game, paths::MOD_DLL, b"old build");
        put(&game, paths::MOD_DLL_NEW, b"half of an interrupted run");

        write_mod(&game, &staged).expect("the mod is written");

        assert_eq!(
            std::fs::read(game.join(paths::MOD_DLL)).expect("the mod"),
            b"new build"
        );
        assert!(!game.join(paths::MOD_DLL_NEW).exists());
    }
}
