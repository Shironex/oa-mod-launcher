//! The game folder: what makes a folder the game.

use std::path::{Path, PathBuf};

use crate::error::{CoreError, Result};

/// The game's executable, in the root of its folder.
pub const EXE: &str = "Golf With Your Friends.exe";

/// The game's compiled code, next to the executable. Only an IL2CPP build has
/// it, and that is the only kind the mod works with.
pub const ASSEMBLY: &str = "GameAssembly.dll";

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
}
