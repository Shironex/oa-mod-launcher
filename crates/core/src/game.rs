//! The game folder: what makes a folder the game.

use std::path::{Path, PathBuf};

use crate::error::{CoreError, Result};

/// The game's executable, in the root of its folder.
pub const EXE: &str = "Golf With Your Friends.exe";

/// The game's compiled code, next to the executable. Only an IL2CPP build has
/// it, and that is the only kind the mod works with.
pub const ASSEMBLY: &str = "GameAssembly.dll";

/// Every path the launcher reads, writes or deletes inside the game folder.
///
/// They are constants on purpose: nothing that comes from the network or from
/// a settings file ever becomes part of a path in the game folder. Each is
/// joined to the canonical game root through [`crate::files::inside`].
pub mod paths {
    /// The folder of BepInEx: its core, the plugins, the config, the caches.
    pub const BEPINEX: &str = "BepInEx";
    /// The .NET runtime BepInEx brings along.
    pub const DOTNET: &str = "dotnet";
    /// The file that proves `dotnet/` is a .NET runtime and not something
    /// else under that name. The folder is deleted only when it is there.
    pub const DOTNET_PROOF: &str = "dotnet/coreclr.dll";
    /// The proxy DLL that makes the game load BepInEx. Written last on an
    /// install and deleted first on a removal.
    pub const WINHTTP: &str = "winhttp.dll";
    /// Where an install of BepInEx is put together, inside the game folder
    /// so that moving it into place is a rename: a copy that stops half way
    /// stops in here, and the folder is deleted at the next start.
    pub const INSTALL_PART: &str = "oamod-install.part";
    /// The name [`INSTALL_PART`] gets once everything is in it. Its pieces
    /// are then renamed into place, `winhttp.dll` last. While this folder
    /// still holds `winhttp.dll`, the install did not finish, and whatever
    /// is missing from it was moved into the game folder by the launcher.
    pub const INSTALL_READY: &str = "oamod-install.ready";
    /// The settings of the loader, next to the proxy DLL.
    pub const DOORSTOP_CONFIG: &str = "doorstop_config.ini";
    /// The core assemblies of BepInEx.
    pub const CORE: &str = "BepInEx/core";
    /// The assembly whose hash says which BepInEx build this is. Only an
    /// IL2CPP build has it.
    pub const PROBE: &str = "BepInEx/core/BepInEx.Unity.IL2CPP.dll";
    /// Where plugins live, ours and other people's.
    pub const PLUGINS: &str = "BepInEx/plugins";
    /// Where patchers live. The mod has none, so all of them are foreign.
    pub const PATCHERS: &str = "BepInEx/patchers";
    /// Where BepInEx and the plugins keep their settings.
    pub const CONFIG: &str = "BepInEx/config";
    /// The name of the mod's folder in [`PLUGINS`].
    pub const MOD_FOLDER_NAME: &str = "OaMod";
    /// The mod's own folder.
    pub const MOD_DIR: &str = "BepInEx/plugins/OaMod";
    /// The mod.
    pub const MOD_DLL: &str = "BepInEx/plugins/OaMod/OaMod.dll";
    /// The mod while it is being written, renamed over [`MOD_DLL`] at the end.
    pub const MOD_DLL_NEW: &str = "BepInEx/plugins/OaMod/OaMod.dll.new";
    /// Debug symbols a local build leaves next to the mod.
    pub const MOD_PDB: &str = "BepInEx/plugins/OaMod/OaMod.pdb";
    /// The name of the mod's settings file in [`CONFIG`].
    pub const MOD_CONFIG_NAME: &str = "com.shirone.oamod.cfg";
    /// The mod's settings.
    pub const MOD_CONFIG: &str = "BepInEx/config/com.shirone.oamod.cfg";
    /// The name of BepInEx's own settings file in [`CONFIG`].
    pub const BEPINEX_CONFIG_NAME: &str = "BepInEx.cfg";
    /// Written by BepInEx on the first start. Until it exists, the next start
    /// takes minutes.
    pub const INTEROP_READY: &str = "BepInEx/interop/Assembly-CSharp.dll";
    /// What BepInEx remembers about the game build it prepared for.
    pub const INTEROP_HASH: &str = "BepInEx/interop/assembly-hash.txt";
}

/// Whether the game is running, as far as its folder can tell: Windows keeps
/// the executable of a running program open and refuses to open it for
/// writing with a sharing violation.
///
/// The file is opened without `create` and without `truncate`, and closed at
/// once, so the check changes nothing. On other systems the answer is always
/// `false`: the game does not run there.
///
/// Asking for write access is the only open that tells: Windows holds a
/// running executable as a mapped image, not through a file handle, so an
/// open for reading that shares nothing is granted while the program runs
/// (tried on a copy of a small program, running and not). Only the request
/// to write is refused.
pub fn is_running(root: &Path) -> bool {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    match std::fs::OpenOptions::new().write(true).open(root.join(EXE)) {
        Ok(_) => false,
        Err(error) => cfg!(windows) && error.raw_os_error() == Some(ERROR_SHARING_VIOLATION),
    }
}

/// Check that `dir` is an installed copy of the game and give its canonical
/// path, which is the root every later file operation is joined to.
///
/// # Errors
///
/// [`CoreError::BadGameFolder`] when the folder does not exist, is the root of
/// a drive, or lacks one of the two files.
pub fn validate(dir: &Path) -> Result<PathBuf> {
    let bad = |reason: &str| CoreError::BadGameFolder {
        path: dir.to_path_buf(),
        reason: reason.to_owned(),
    };

    let root = std::fs::canonicalize(dir).map_err(|_| bad("the folder does not exist"))?;
    // Before the file checks: whatever lies in the root of a drive, the
    // launcher must never treat a whole drive as a folder it may write to.
    if root.parent().is_none() {
        return Err(bad("the root of a drive is never the game folder"));
    }
    if !root.is_dir() {
        return Err(bad("it is a file, not a folder"));
    }
    for name in [EXE, ASSEMBLY] {
        if !root.join(name).is_file() {
            return Err(bad(&format!("\"{name}\" is not in it")));
        }
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reason(result: Result<PathBuf>) -> String {
        match result {
            Err(CoreError::BadGameFolder { reason, .. }) => reason,
            other => panic!("expected a refused folder, got {other:?}"),
        }
    }

    #[test]
    fn a_folder_with_both_files_is_the_game() {
        let temp = tempfile::tempdir().expect("a temporary directory");
        let dir = temp.path().join("Golf With Your Friends");
        std::fs::create_dir(&dir).expect("the game folder");
        std::fs::write(dir.join(EXE), b"MZ").expect("the exe");
        std::fs::write(dir.join(ASSEMBLY), b"MZ").expect("the assembly");

        // Through a path with a `..` in it, to see that the answer is canonical.
        let roundabout = dir.join("..").join("Golf With Your Friends");
        let root = validate(&roundabout).expect("the folder is accepted");

        assert_eq!(root, std::fs::canonicalize(&dir).expect("a real path"));
        assert!(root.join(EXE).is_file());
    }

    #[test]
    fn a_folder_without_one_of_the_files_is_refused() {
        for present in [EXE, ASSEMBLY] {
            let temp = tempfile::tempdir().expect("a temporary directory");
            std::fs::write(temp.path().join(present), b"MZ").expect("one file");
            let missing = if present == EXE { ASSEMBLY } else { EXE };

            assert!(reason(validate(temp.path())).contains(missing));
        }
    }

    #[test]
    fn a_folder_named_like_a_needed_file_does_not_count() {
        let temp = tempfile::tempdir().expect("a temporary directory");
        std::fs::create_dir(temp.path().join(EXE)).expect("a folder");
        std::fs::write(temp.path().join(ASSEMBLY), b"MZ").expect("the assembly");

        assert!(reason(validate(temp.path())).contains(EXE));
    }

    #[test]
    fn a_missing_folder_and_a_file_are_refused() {
        let temp = tempfile::tempdir().expect("a temporary directory");
        let file = temp.path().join(EXE);
        std::fs::write(&file, b"MZ").expect("a file");

        assert!(reason(validate(&temp.path().join("nowhere"))).contains("does not exist"));
        assert!(reason(validate(&file)).contains("not a folder"));
    }

    #[test]
    fn the_root_of_a_drive_is_refused_whatever_it_holds() {
        let temp = tempfile::tempdir().expect("a temporary directory");
        let drive = temp
            .path()
            .ancestors()
            .last()
            .expect("every path has a root");

        assert!(reason(validate(drive)).contains("root of a drive"));
    }

    #[test]
    fn a_game_that_is_not_running_reads_as_not_running_and_is_left_unchanged() {
        let temp = tempfile::tempdir().expect("a temporary directory");
        std::fs::write(temp.path().join(EXE), b"MZ the game").expect("the exe");

        assert!(!is_running(temp.path()));
        assert!(!is_running(&temp.path().join("nowhere")), "no exe, no game");
        assert_eq!(
            std::fs::read(temp.path().join(EXE)).expect("the exe"),
            b"MZ the game",
            "the check opens the file and writes nothing"
        );
    }

    /// What Windows does for a running program is stood in for by a handle
    /// that shares the file for reading only. That is the effect, not the
    /// mechanism: a running program is held as a mapped image and no handle,
    /// and a test cannot make one of those without starting a program.
    #[test]
    #[cfg(windows)]
    fn an_exe_that_is_held_open_reads_as_running() {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 1;
        let temp = tempfile::tempdir().expect("a temporary directory");
        let exe = temp.path().join(EXE);
        std::fs::write(&exe, b"MZ").expect("the exe");

        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&exe)
            .expect("the exe is held");
        assert!(is_running(temp.path()));

        drop(held);
        assert!(!is_running(temp.path()));
    }
}
