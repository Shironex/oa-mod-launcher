//! The few file system rules every write and every delete goes through.
//!
//! The launcher writes into a folder it does not own, the game folder, so the
//! rules are strict and in one place:
//!
//! - A path is built by [`inside`] only: a relative path joined to a root
//!   that was made canonical before, checked to stay under it.
//! - No part of such a path may be a reparse point (a junction or a symbolic
//!   link) when it is looked at: a path with one is refused, and so is a tree
//!   with one anywhere in it.
//!
//! What this does not give: the look and the delete are two steps, and
//! Windows offers no way in the standard library to make them one. Another
//! program that swaps a folder for a link in between is not shut out. The
//! window is kept small instead: [`delete_tree`] looks again at every folder
//! between the top of the tree and an entry right before it deletes that
//! entry, and [`delete_file`] looks at the entry itself. Every reparse point
//! is refused, whatever its kind, not only the ones that are links by name.

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::error::{CoreError, Result};
use crate::net::hex;

/// How often a rename is tried. Antivirus software can hold a freshly written
/// file for a moment.
const RENAME_ATTEMPTS: u32 = 6;

/// The pause between two rename attempts.
const RENAME_PAUSE: Duration = Duration::from_millis(250);

/// `path` as it is shown to a person: without the `\\?\` prefix that a
/// canonical Windows path carries.
pub fn display(path: &Path) -> String {
    let text = path.display().to_string();
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(plain) = text.strip_prefix(r"\\?\") {
        plain.to_owned()
    } else {
        text
    }
}

/// The length and the lowercase hex SHA-256 of the file at `path`.
///
/// # Errors
///
/// [`CoreError::Io`] when the file cannot be read.
pub fn sha256_file(path: &Path) -> Result<(u64, String)> {
    let failed = || CoreError::io("read the file", path);
    let mut file = std::fs::File::open(path).map_err(failed())?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let read = file.read(&mut buffer).map_err(failed())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total += read as u64;
    }
    Ok((total, hex(&hasher.finalize())))
}

/// Whether the entry these metadata describe is a reparse point: a junction,
/// a symbolic link or anything else that makes a path lead somewhere else.
/// Pass the result of `symlink_metadata`, which does not follow the link.
pub fn is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn unsafe_path(path: &Path, reason: &str) -> CoreError {
    CoreError::UnsafePath {
        path: path.to_path_buf(),
        reason: reason.to_owned(),
    }
}

/// `root` joined with `relative`, proven to stay inside `root`.
///
/// `root` must be canonical. `relative` may only have plain name parts: no
/// `..`, no root, no drive letter. Every part that already exists on disk is
/// looked at without following links, and a reparse point among them refuses
/// the whole path. Parts that do not exist yet are fine.
///
/// # Errors
///
/// [`CoreError::UnsafePath`] when the path could leave `root`.
pub fn inside(root: &Path, relative: impl AsRef<Path>) -> Result<PathBuf> {
    let relative = relative.as_ref();
    let mut path = root.to_path_buf();
    let mut parts = 0;
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(unsafe_path(
                &root.join(relative),
                "it is not a plain path below the folder",
            ));
        };
        path.push(name);
        parts += 1;
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if is_reparse_point(&metadata) => {
                return Err(unsafe_path(&path, "it is a link to somewhere else"));
            }
            _ => {}
        }
    }
    if parts == 0 || !path.starts_with(root) || path == root {
        return Err(unsafe_path(&path, "it is not below the folder"));
    }
    Ok(path)
}

/// Turn the failure of a write or a delete in the game folder into the
/// sentence that helps: a sharing violation means the game is running, a
/// refused access that the folder is not writable.
pub fn write_error(
    operation: &'static str,
    path: &Path,
) -> impl FnOnce(std::io::Error) -> CoreError {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    let path = path.to_path_buf();
    move |source| {
        if cfg!(windows) && source.raw_os_error() == Some(ERROR_SHARING_VIOLATION) {
            CoreError::GameRunning
        } else if source.kind() == std::io::ErrorKind::PermissionDenied {
            CoreError::NotWritable { path }
        } else {
            CoreError::Io {
                operation,
                path,
                source,
            }
        }
    }
}

/// Rename `from` over `to`, trying again for a moment when it fails.
pub fn rename_with_retries(from: &Path, to: &Path, operation: &'static str) -> Result<()> {
    let mut attempt = 1;
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) if attempt >= RENAME_ATTEMPTS => {
                return Err(write_error(operation, to)(error));
            }
            Err(_) => {
                attempt += 1;
                std::thread::sleep(RENAME_PAUSE);
            }
        }
    }
}

/// Delete one file. `Ok(false)` when it was not there.
///
/// `path` must come from [`inside`]. The entry itself is checked once more: a
/// reparse point or a folder under that name is refused, not removed.
pub fn delete_file(path: &Path) -> Result<bool> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(write_error("look at the file", path)(error)),
    };
    if is_reparse_point(&metadata) {
        return Err(unsafe_path(path, "it is a link to somewhere else"));
    }
    if metadata.is_dir() {
        return Err(unsafe_path(path, "a folder is where a file was expected"));
    }
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(first) if first.kind() == std::io::ErrorKind::PermissionDenied => {
            // A read-only file cannot be deleted on Windows until the flag is
            // cleared. When that is not the reason, the first error stands.
            let mut permissions = metadata.permissions();
            #[allow(
                clippy::permissions_set_readonly_false,
                reason = "the file is deleted in the next line"
            )]
            permissions.set_readonly(false);
            if std::fs::set_permissions(path, permissions).is_ok()
                && std::fs::remove_file(path).is_ok()
            {
                return Ok(true);
            }
            Err(write_error("delete the file", path)(first))
        }
        Err(error) => Err(write_error("delete the file", path)(error)),
    }
}

/// Every file and folder below `dir`, without following a link: files as
/// `(path, length)`, folders deepest last. `dir` itself is not listed.
///
/// # Errors
///
/// [`CoreError::UnsafePath`] when any entry is a reparse point, so a caller
/// that scans before it deletes or copies never starts on such a tree.
pub fn scan_tree(dir: &Path) -> Result<Tree> {
    let mut tree = Tree::default();
    scan_into(dir, &mut tree)?;
    Ok(tree)
}

/// What [`scan_tree`] found.
#[derive(Debug, Default)]
pub struct Tree {
    /// Every file, with its length.
    pub files: Vec<(PathBuf, u64)>,
    /// Every folder, a parent before its children.
    pub dirs: Vec<PathBuf>,
}

impl Tree {
    /// The sum of the file lengths.
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|file| file.1).sum()
    }
}

fn scan_into(dir: &Path, tree: &mut Tree) -> Result<()> {
    let failed = || CoreError::io("list the folder", dir);
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(failed())?
        .collect::<std::io::Result<_>>()
        .map_err(failed())?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let metadata =
            std::fs::symlink_metadata(&path).map_err(CoreError::io("look at the file", &path))?;
        if is_reparse_point(&metadata) {
            return Err(unsafe_path(&path, "it is a link to somewhere else"));
        }
        if metadata.is_dir() {
            tree.dirs.push(path.clone());
            scan_into(&path, tree)?;
        } else {
            tree.files.push((path, metadata.len()));
        }
    }
    Ok(())
}

/// Delete the folder `dir` and everything in it, without following a link.
/// `Ok(false)` when it was not there.
///
/// `dir` must come from [`inside`]. The tree is scanned first, so a reparse
/// point anywhere in it refuses the delete before a single file is gone. Each
/// entry is then deleted by its own path, children before their folder: files
/// first, folders deepest first. Right before each delete, every folder from
/// `dir` down to that entry is looked at again, and the delete stops when one
/// of them has become a link since the scan.
pub fn delete_tree(dir: &Path) -> Result<bool> {
    let metadata = match std::fs::symlink_metadata(dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(write_error("look at the folder", dir)(error)),
    };
    if is_reparse_point(&metadata) {
        return Err(unsafe_path(dir, "it is a link to somewhere else"));
    }
    if !metadata.is_dir() {
        return Err(unsafe_path(dir, "a file is where a folder was expected"));
    }
    let tree = scan_tree(dir)?;
    delete_scanned(dir, &tree)?;
    Ok(true)
}

/// Delete what a scan of `dir` found, and `dir` itself.
fn delete_scanned(dir: &Path, tree: &Tree) -> Result<()> {
    for (file, _) in &tree.files {
        still_plain(dir, file)?;
        delete_file(file)?;
    }
    for folder in tree.dirs.iter().rev().chain([&dir.to_path_buf()]) {
        still_plain(dir, folder)?;
        // `remove_dir` only removes an empty folder and never follows a link,
        // so even a folder swapped for a junction in the meantime costs the
        // link and not what it points at.
        match std::fs::remove_dir(folder) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(write_error("delete the folder", folder)(error)),
        }
    }
    Ok(())
}

/// Look again at every folder from `top` down to the one that holds `path`:
/// each must still be a folder and not a link. One that is gone is fine, the
/// entry below it is gone with it.
fn still_plain(top: &Path, path: &Path) -> Result<()> {
    let below = path
        .strip_prefix(top)
        .map_err(|_| unsafe_path(path, "it is not below the folder"))?;
    let check = |folder: &Path| match std::fs::symlink_metadata(folder) {
        Ok(metadata) if is_reparse_point(&metadata) => {
            Err(unsafe_path(folder, "it is a link to somewhere else"))
        }
        Ok(metadata) if !metadata.is_dir() => {
            Err(unsafe_path(folder, "a file is where a folder was expected"))
        }
        _ => Ok(()),
    };
    let mut folder = top.to_path_buf();
    check(&folder)?;
    // Not the last part: that is the entry itself, which its own delete
    // looks at.
    for part in below.parent().into_iter().flat_map(Path::components) {
        folder.push(part);
        check(&folder)?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A junction on Windows, a symbolic link elsewhere: `link` leads to the
    /// folder `target`. Neither needs special rights.
    pub(crate) fn link_dir(target: &Path, link: &Path) {
        #[cfg(windows)]
        {
            let status = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .stdout(std::process::Stdio::null())
                .status()
                .expect("cmd runs");
            assert!(status.success(), "the junction is made");
        }
        #[cfg(not(windows))]
        std::os::unix::fs::symlink(target, link).expect("the link is made");
    }

    fn canonical_temp() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().expect("a temporary directory");
        let root = std::fs::canonicalize(temp.path()).expect("a real path");
        (temp, root)
    }

    #[test]
    fn the_long_path_prefix_is_not_shown() {
        for (raw, shown) in [
            (r"\\?\C:\Games\Golf", r"C:\Games\Golf"),
            (r"\\?\UNC\server\share\Golf", r"\\server\share\Golf"),
            (r"C:\Games\Golf", r"C:\Games\Golf"),
            ("/home/me/Golf", "/home/me/Golf"),
        ] {
            assert_eq!(display(Path::new(raw)), shown);
        }
    }

    #[test]
    fn a_file_hashes_to_its_sha256() {
        let (_temp, root) = canonical_temp();
        let file = root.join("a.txt");
        std::fs::write(&file, b"abc").expect("a file");

        assert_eq!(
            sha256_file(&file).expect("the file is read"),
            (
                3,
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".to_owned()
            )
        );
        assert!(sha256_file(&root.join("missing")).is_err());
    }

    #[test]
    fn a_plain_relative_path_stays_inside() {
        let (_temp, root) = canonical_temp();

        let path = inside(&root, "BepInEx/plugins/OaMod/OaMod.dll").expect("a plain path");

        assert!(path.starts_with(&root));
        assert!(
            path.ends_with(
                Path::new("BepInEx")
                    .join("plugins")
                    .join("OaMod")
                    .join("OaMod.dll")
            )
        );
    }

    #[test]
    fn a_path_that_could_leave_the_root_is_refused() {
        let (_temp, root) = canonical_temp();
        for relative in [
            "",
            "..",
            "../outside.txt",
            "BepInEx/../../outside.txt",
            "/absolute",
            "C:/Windows/evil.dll",
            r"C:\Windows\evil.dll",
            r"\\server\share\evil.dll",
            "./BepInEx",
        ] {
            // A backslash and a colon are ordinary characters of a name
            // outside Windows.
            if !cfg!(windows) && relative.contains(['\\', ':']) {
                continue;
            }
            assert!(
                matches!(inside(&root, relative), Err(CoreError::UnsafePath { .. })),
                "{relative:?} must be refused"
            );
        }
    }

    #[test]
    fn a_path_through_a_link_is_refused_at_any_depth() {
        let (_temp, root) = canonical_temp();
        let game = root.join("game");
        let elsewhere = root.join("elsewhere");
        std::fs::create_dir_all(game.join("BepInEx")).expect("the game");
        std::fs::create_dir_all(&elsewhere).expect("another folder");
        link_dir(&elsewhere, &game.join("BepInEx").join("plugins"));

        for relative in ["BepInEx/plugins", "BepInEx/plugins/OaMod/OaMod.dll"] {
            assert!(
                matches!(inside(&game, relative), Err(CoreError::UnsafePath { .. })),
                "{relative} goes through a link"
            );
        }
        assert!(inside(&game, "BepInEx/core").is_ok());
    }

    #[test]
    fn a_tree_is_deleted_whole_and_a_missing_one_is_fine() {
        let (_temp, root) = canonical_temp();
        let dir = root.join("BepInEx");
        std::fs::create_dir_all(dir.join("core").join("deep")).expect("folders");
        std::fs::write(dir.join("core").join("a.dll"), b"a").expect("a file");
        std::fs::write(dir.join("core").join("deep").join("b.dll"), b"b").expect("a file");
        let read_only = dir.join("read-only.txt");
        std::fs::write(&read_only, b"c").expect("a file");
        let mut permissions = std::fs::metadata(&read_only)
            .expect("metadata")
            .permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&read_only, permissions).expect("read-only");

        assert_eq!(scan_tree(&dir).expect("a scan").bytes(), 3);
        assert!(delete_tree(&dir).expect("the tree is deleted"));
        assert!(!dir.exists());
        assert!(!delete_tree(&dir).expect("a second run is fine"));
        assert!(root.exists());
    }

    #[test]
    fn a_tree_with_a_link_in_it_is_not_touched_and_the_target_survives() {
        let (_temp, root) = canonical_temp();
        let dir = root.join("BepInEx");
        let elsewhere = root.join("elsewhere");
        std::fs::create_dir_all(dir.join("core")).expect("folders");
        std::fs::create_dir_all(&elsewhere).expect("another folder");
        std::fs::write(dir.join("core").join("a.dll"), b"a").expect("a file");
        std::fs::write(elsewhere.join("precious.txt"), b"keep").expect("a file");
        link_dir(&elsewhere, &dir.join("plugins"));

        assert!(matches!(
            delete_tree(&dir),
            Err(CoreError::UnsafePath { .. })
        ));
        assert!(
            dir.join("core").join("a.dll").exists(),
            "nothing is deleted when the scan finds a link"
        );
        assert!(elsewhere.join("precious.txt").exists());

        // The folder itself being a link is refused the same way.
        let linked = root.join("dotnet");
        link_dir(&elsewhere, &linked);
        assert!(matches!(
            delete_tree(&linked),
            Err(CoreError::UnsafePath { .. })
        ));
        assert!(matches!(
            delete_file(&linked),
            Err(CoreError::UnsafePath { .. })
        ));
        assert!(elsewhere.join("precious.txt").exists());
    }

    #[test]
    fn a_folder_swapped_for_a_link_after_the_scan_is_not_deleted_through() {
        let (_temp, root) = canonical_temp();
        let dir = root.join("BepInEx");
        let elsewhere = root.join("elsewhere");
        // The same names below both, so a delete by full path would find a
        // plain file at the end of the path either way.
        for base in [dir.join("plugins"), elsewhere.clone()] {
            std::fs::create_dir_all(base.join("deep")).expect("folders");
            std::fs::write(base.join("deep").join("precious.txt"), b"keep").expect("a file");
            std::fs::create_dir_all(base.join("empty")).expect("an empty folder");
        }
        std::fs::write(dir.join("a.dll"), b"a").expect("a file");

        let tree = scan_tree(&dir).expect("the scan finds no link");
        // Between the scan and the delete, somebody swaps a folder for a
        // link to another place.
        std::fs::remove_dir_all(dir.join("plugins")).expect("remove the folder");
        link_dir(&elsewhere, &dir.join("plugins"));

        let result = delete_scanned(&dir, &tree);

        assert!(
            matches!(result, Err(CoreError::UnsafePath { .. })),
            "{result:?}"
        );
        assert!(
            elsewhere.join("deep").join("precious.txt").is_file(),
            "a file behind the link survives"
        );
        assert!(
            elsewhere.join("empty").is_dir(),
            "and so does an empty folder behind it"
        );
    }

    #[test]
    fn deleting_a_file_says_whether_it_was_there() {
        let (_temp, root) = canonical_temp();
        let file = root.join("winhttp.dll");
        std::fs::write(&file, b"x").expect("a file");

        assert!(delete_file(&file).expect("deleted"));
        assert!(!delete_file(&file).expect("already gone"));

        std::fs::create_dir(&file).expect("a folder under the same name");
        assert!(matches!(
            delete_file(&file),
            Err(CoreError::UnsafePath { .. })
        ));
        assert!(file.is_dir());
    }
}
