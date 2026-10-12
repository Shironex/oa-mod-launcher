//! Installing and updating: BepInEx when the game has none, then the mod.
//!
//! Every download goes to the launcher's own `staging/` folder first (emptied
//! before, and refused when a link stands in for it or is in it) and is
//! compared with the size and the SHA-256 the signed manifest names. Only
//! then is anything written into the game folder, in an order that leaves the
//! game either as it was or working:
//!
//! 1. Nothing of BepInEx is written when any of its files is already there.
//! 2. BepInEx is copied into a staging folder inside the game folder, not to
//!    its final names. A copy that stops half way stops in there.
//! 3. From there each piece is renamed into place: `dotnet/`, `BepInEx/core`,
//!    the small root files and `winhttp.dll` last. That file is what makes
//!    the game load BepInEx, so an install that stops half way never loads.
//! 4. When a step fails, what was moved is taken out again. When the launcher
//!    itself dies, the next start does the same, before anything else is
//!    done with the game folder: see [`clear_interrupted`].
//! 5. The mod is written next to its final name and renamed over it.
//!
//! An update is the same flow: with BepInEx in place, only the mod is
//! replaced.

use std::path::Path;

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
        let _ = clean_staging(self.layout());
        drop(lease);
        result.map_err(|error| self.explained(error))?;
        Ok(self.snapshot())
    }

    async fn install_steps(&self, manifest: &Manifest, report: Report<'_>) -> Result<()> {
        let game = self.writable_game()?;
        let state = self.state()?;
        let facts = inspect::inspect(&game.root, &state, Some(&manifest.bepinex))?;

        // A staging folder that is a link, or has one in it, stops the
        // install here, before anything is downloaded into it.
        let staging = clean_staging(self.layout())?;
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
/// nothing at its top level but the two folders and the four root files, no
/// file in `BepInEx/` outside `BepInEx/core`, and the probe and the root
/// files with the checksums of the manifest.
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
    // Only `BepInEx/core` is moved into the game folder, so a file anywhere
    // else in `BepInEx/` would be dropped without a word. The build the
    // release names has none.
    let core = unpacked.join(paths::CORE);
    for (file, _) in files::scan_tree(&unpacked.join(paths::BEPINEX))?.files {
        if !file.starts_with(&core) {
            let name = file.strip_prefix(unpacked).unwrap_or(&file);
            return Err(refused(format!(
                "it holds \"{}\", and BepInEx/core is the only folder of BepInEx it may bring",
                name.display()
            )));
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

/// What is renamed out of the staging folder into the game folder, in that
/// order: the two folders, then the root files with `winhttp.dll` last.
fn pieces() -> impl DoubleEndedIterator<Item = &'static str> {
    [paths::DOTNET, paths::CORE].into_iter().chain(ROOT_FILES)
}

/// Put the unpacked BepInEx into the game folder `root`.
///
/// Nothing is copied to its final name. Everything is first copied into
/// [`paths::INSTALL_PART`] inside the game folder, which is then renamed to
/// [`paths::INSTALL_READY`], and from there each piece is renamed into place:
/// `dotnet`, `BepInEx/core`, the small root files and `winhttp.dll` last. A
/// rename is all or nothing, so no folder of BepInEx is ever half there under
/// its own name, and what a launcher that died left behind is told from the
/// two staging names alone, see [`clear_interrupted`].
///
/// `still_allowed` is asked once more after every target was checked and
/// before the first byte is written, and again before every later step: the
/// rest of `dotnet` after its proof file, each further copy, and each move.
/// When it refuses or a step fails, what this call did is undone.
fn place(unpacked: &Path, root: &Path, still_allowed: &dyn Fn() -> Result<()>) -> Result<()> {
    // The leftovers of a run that died, before the targets are looked at.
    clear_interrupted(root)?;
    // Every target is proven to be inside the game folder and free before
    // anything is written.
    for piece in pieces() {
        let target = files::inside(root, piece)?;
        if std::fs::symlink_metadata(&target).is_ok() {
            return Err(CoreError::Conflict { path: target });
        }
    }
    still_allowed()?;

    let placed = stage(unpacked, root, still_allowed).and_then(|()| move_in(root, still_allowed));
    if placed.is_err() {
        // Best effort: what cannot be undone now is undone by the next
        // start, from the same two names.
        let _ = clear_interrupted(root);
    }
    placed
}

/// Copy the unpacked BepInEx into [`paths::INSTALL_PART`] in the game folder
/// and rename that to [`paths::INSTALL_READY`] once all of it is there.
fn stage(unpacked: &Path, root: &Path, still_allowed: &dyn Fn() -> Result<()>) -> Result<()> {
    const WRITE: &str = "write into the game folder";
    let part = files::inside(root, paths::INSTALL_PART)?;
    std::fs::create_dir(&part).map_err(files::write_error(WRITE, &part))?;

    let copy = |relative: &Path| -> Result<()> {
        let (source, target) = (unpacked.join(relative), part.join(relative));
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(files::write_error(WRITE, parent))?;
        }
        let mut to = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(files::write_error(WRITE, &target))?;
        let mut from = std::fs::File::open(&source)
            .map_err(CoreError::io("read the unpacked archive", &source))?;
        std::io::copy(&mut from, &mut to).map_err(files::write_error(WRITE, &target))?;
        to.sync_all().map_err(files::write_error(WRITE, &target))
    };
    let out_of_place = |_| CoreError::Archive("an unpacked file is out of place".to_owned());

    // The file that proves `dotnet/` is the runtime goes first, so there is
    // no moment at which the folder holds files of the runtime without it.
    let proof = Path::new(paths::DOTNET_PROOF);
    if unpacked.join(proof).is_file() {
        copy(proof)?;
    }
    for folder in [paths::DOTNET, paths::CORE] {
        still_allowed()?;
        let tree = files::scan_tree(&unpacked.join(folder))?;
        for (file, _) in &tree.files {
            let relative = file.strip_prefix(unpacked).map_err(out_of_place)?;
            if relative != proof {
                copy(relative)?;
            }
        }
    }
    still_allowed()?;
    for name in ROOT_FILES {
        copy(Path::new(name))?;
    }

    still_allowed()?;
    let ready = files::inside(root, paths::INSTALL_READY)?;
    files::rename_with_retries(&part, &ready, WRITE)
}

/// Rename every piece out of [`paths::INSTALL_READY`] into the game folder,
/// `winhttp.dll` last, and remove the folder they came from.
fn move_in(root: &Path, still_allowed: &dyn Fn() -> Result<()>) -> Result<()> {
    const MOVE: &str = "put BepInEx into place";
    let ready = files::inside(root, paths::INSTALL_READY)?;
    let bepinex = files::inside(root, paths::BEPINEX)?;
    if !bepinex.is_dir() {
        std::fs::create_dir(&bepinex).map_err(files::write_error(MOVE, &bepinex))?;
    }
    for piece in pieces() {
        still_allowed()?;
        let target = files::inside(root, piece)?;
        // A rename replaces a file that is in its way. Nothing was there a
        // moment ago, and nothing may be replaced now.
        if std::fs::symlink_metadata(&target).is_ok() {
            return Err(CoreError::Conflict { path: target });
        }
        files::rename_with_retries(&ready.join(piece), &target, MOVE)?;
    }
    // Empty by now. A leftover of it is removed by the next start.
    let _ = files::delete_tree(&ready);
    Ok(())
}

/// Undo an install of BepInEx that did not finish: what a launcher that was
/// killed, or a step that failed, left in the game folder `root`.
///
/// It is told from the two staging names alone:
///
/// - [`paths::INSTALL_PART`] is a copy that stopped half way. Nothing was
///   moved out of it, so deleting it is all there is to do.
/// - [`paths::INSTALL_READY`] that still holds `winhttp.dll` is an install
///   that stopped while its pieces were moved. Each piece that is no longer
///   in it was moved into the game folder by the launcher, into a place that
///   was empty, so it is deleted there. Then the folder goes.
/// - [`paths::INSTALL_READY`] without `winhttp.dll` is an install whose last
///   move was made. Only the empty folder is removed.
///
/// Only the constant names are ever deleted, through the safe deletes: a link
/// under one of them refuses the whole step. Running it again is fine.
pub(crate) fn clear_interrupted(root: &Path) -> Result<()> {
    let ready = files::inside(root, paths::INSTALL_READY)?;
    let waiting = |piece: &str| std::fs::symlink_metadata(ready.join(piece)).is_ok();
    if ready.is_dir() && waiting(paths::WINHTTP) {
        // Newest first, as a failed copy was always taken back.
        for piece in pieces().rev().filter(|piece| !waiting(piece)) {
            let moved = files::inside(root, piece)?;
            if [paths::DOTNET, paths::CORE].contains(&piece) {
                files::delete_tree(&moved)?;
            } else {
                files::delete_file(&moved)?;
            }
        }
        // The folder of BepInEx, when the install made it: `remove_dir`
        // removes it only when nothing else is in it.
        let _ = std::fs::remove_dir(files::inside(root, paths::BEPINEX)?);
        // Everything that was moved is out again. The staged proxy DLL goes
        // first, so that a delete of the staging folder that stops half way
        // is not read as "moved" by the next run: without it the folder only
        // says "nothing left to undo".
        files::delete_file(&ready.join(paths::WINHTTP))?;
    }
    files::delete_tree(&ready)?;
    files::delete_tree(&files::inside(root, paths::INSTALL_PART)?)?;
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
    use std::path::PathBuf;

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
        assert_eq!(pieces().last(), Some(paths::WINHTTP));
        assert_eq!(
            pieces().take(2).collect::<Vec<_>>(),
            [paths::DOTNET, paths::CORE]
        );
    }

    #[test]
    fn a_placement_puts_everything_into_place_and_leaves_no_staging_folder() {
        let (_temp, base) = canonical_temp();
        let unpacked = unpacked_in(&base);
        let game = base.join("game");
        put(&game, "BepInEx/config/theirs.cfg", b"keep");
        // What a launcher that died in an earlier copy left behind.
        put(&game, "oamod-install.part/dotnet/half.dll", b"half");

        place(&unpacked, &game, &allowed).expect("BepInEx is placed");

        for relative in [
            "dotnet/coreclr.dll",
            "dotnet/shared/deep/System.dll",
            "BepInEx/core/BepInEx.Unity.IL2CPP.dll",
            "winhttp.dll",
            "doorstop_config.ini",
            ".doorstop_version",
            "changelog.txt",
        ] {
            assert_eq!(
                std::fs::read(game.join(relative)).expect("placed"),
                relative.as_bytes()
            );
        }
        assert!(game.join("BepInEx/config/theirs.cfg").is_file());
        assert!(!game.join(paths::INSTALL_PART).exists());
        assert!(!game.join(paths::INSTALL_READY).exists());
    }

    #[test]
    fn an_install_whose_last_move_was_made_is_not_taken_back() {
        let (_temp, base) = canonical_temp();
        let unpacked = unpacked_in(&base);
        let game = base.join("game");
        std::fs::create_dir(&game).expect("the game");
        place(&unpacked, &game, &allowed).expect("BepInEx is placed");
        // The launcher died after winhttp.dll was moved and before the empty
        // staging folder was removed.
        std::fs::create_dir_all(game.join(paths::INSTALL_READY).join("BepInEx"))
            .expect("the emptied staging folder");

        clear_interrupted(&game).expect("cleared");

        assert!(!game.join(paths::INSTALL_READY).exists());
        for kept in ["dotnet/coreclr.dll", paths::PROBE, "winhttp.dll"] {
            assert!(game.join(kept).is_file(), "{kept} stays");
        }
    }

    #[test]
    fn only_what_left_the_staging_folder_is_taken_back() {
        let (_temp, base) = canonical_temp();
        let game = base.join("game");
        // The staging folder still holds dotnet and winhttp.dll, so neither
        // was moved: a `dotnet` in the game folder is somebody else's. The
        // core folder and the config are gone from it, so those were moved.
        put(&game, "oamod-install.ready/dotnet/coreclr.dll", b"staged");
        put(&game, "oamod-install.ready/winhttp.dll", b"staged");
        put(&game, "oamod-install.ready/changelog.txt", b"staged");
        put(&game, "dotnet/theirs.dll", b"theirs");
        put(&game, "changelog.txt", b"theirs");
        put(&game, "BepInEx/core/moved.dll", b"moved");
        put(&game, "BepInEx/config/theirs.cfg", b"theirs");
        put(&game, "doorstop_config.ini", b"moved");

        clear_interrupted(&game).expect("cleared");

        let mut left: Vec<_> = files::scan_tree(&game)
            .expect("a scan")
            .files
            .into_iter()
            .map(|file| file.0.strip_prefix(&game).expect("below").to_path_buf())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                PathBuf::from("BepInEx").join("config").join("theirs.cfg"),
                PathBuf::from("changelog.txt"),
                PathBuf::from("dotnet").join("theirs.dll"),
            ]
        );
        assert!(!game.join("BepInEx/core").exists());
    }

    #[test]
    fn a_link_under_a_staging_name_is_refused_and_its_target_survives() {
        for name in [paths::INSTALL_PART, paths::INSTALL_READY] {
            let (_temp, base) = canonical_temp();
            let unpacked = unpacked_in(&base);
            let game = base.join("game");
            let elsewhere = base.join("elsewhere");
            std::fs::create_dir(&game).expect("the game");
            // It looks like an install that stopped while moving, and it is
            // a link to a folder that is not the launcher's.
            put(&elsewhere, "winhttp.dll", b"not ours");
            put(&game, "doorstop_config.ini", b"not ours either");
            files::tests::link_dir(&elsewhere, &game.join(name));

            assert!(matches!(
                clear_interrupted(&game),
                Err(CoreError::UnsafePath { .. })
            ));
            assert!(matches!(
                place(&unpacked, &game, &allowed),
                Err(CoreError::UnsafePath { .. })
            ));

            assert!(elsewhere.join("winhttp.dll").is_file(), "{name}");
            assert!(game.join("doorstop_config.ini").is_file(), "{name}");
            assert!(!game.join("dotnet").exists(), "{name}");
        }
    }

    #[test]
    fn a_zip_with_a_file_in_bepinex_outside_core_is_refused() {
        use sha2::Digest;
        let (_temp, base) = canonical_temp();
        let unpacked = unpacked_in(&base);
        let sha256 = |relative: &str| crate::net::hex(&sha2::Sha256::digest(relative.as_bytes()));
        let expected = manifest::BepInEx {
            label: "test".to_owned(),
            url: String::new(),
            size: 1,
            sha256: String::new(),
            probe_sha256: sha256(paths::PROBE),
            root_files: ROOT_FILES
                .iter()
                .map(|name| ((*name).to_owned(), sha256(name)))
                .collect(),
        };
        check_unpacked(&unpacked, &expected).expect("the plain build passes");
        // An empty folder is nothing that could be dropped.
        std::fs::create_dir_all(unpacked.join("BepInEx/plugins")).expect("an empty folder");
        check_unpacked(&unpacked, &expected).expect("an empty folder passes");

        put(&unpacked, "BepInEx/plugins/Extra/Extra.dll", b"extra");

        let refused = check_unpacked(&unpacked, &expected);
        assert!(
            matches!(&refused, Err(CoreError::Archive(reason)) if reason.contains("Extra.dll")),
            "{refused:?}"
        );
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
