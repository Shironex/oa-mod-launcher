//! Unpacking a downloaded zip.
//!
//! The archive is checked as a whole before a single byte is written. An entry
//! with an absolute path, a drive letter or a `..` part, or one that is a
//! symbolic link, makes the launcher refuse the entire archive: the zip the
//! release names never contains one, so its presence means the file is not
//! what it claims to be.

use std::path::{Component, Path, PathBuf};

use crate::error::{CoreError, Result};

/// The most an archive may unpack to. The one zip the launcher unpacks is
/// under 100 MB, so anything near this is a mistake or a decompression bomb.
pub const MAX_UNPACKED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// The most entries an archive may hold.
pub const MAX_ENTRIES: usize = 20_000;

/// Unpack `archive` into the new directory `destination`.
///
/// Blocking: call it from a blocking thread.
///
/// # Errors
///
/// [`CoreError::Archive`] when the zip is damaged or holds a refused entry,
/// [`CoreError::Io`] when a file cannot be written.
pub fn extract(archive: &Path, destination: &Path) -> Result<()> {
    let file = std::fs::File::open(archive)
        .map_err(CoreError::io("open the downloaded archive", archive))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|error| refused(&error.to_string()))?;

    if zip.len() > MAX_ENTRIES {
        return Err(refused("it has too many entries"));
    }

    // First pass: decide where every entry goes, and refuse the archive if any
    // of them is not a plain relative path.
    let mut targets = Vec::with_capacity(zip.len());
    let mut total: u64 = 0;
    for index in 0..zip.len() {
        let entry = zip
            .by_index(index)
            .map_err(|error| refused(&error.to_string()))?;
        if entry.is_symlink() {
            return Err(refused(&format!("\"{}\" is a symbolic link", entry.name())));
        }
        let relative = safe_relative_path(entry.name())
            .ok_or_else(|| refused(&format!("\"{}\" would be written outside", entry.name())))?;
        total = total.saturating_add(entry.size());
        if total > MAX_UNPACKED_BYTES {
            return Err(refused("it unpacks to more than 2 GB"));
        }
        targets.push((destination.join(relative), entry.is_dir()));
    }

    std::fs::create_dir_all(destination)
        .map_err(CoreError::io("create the unpack directory", destination))?;

    for (index, (target, is_dir)) in targets.iter().enumerate() {
        if *is_dir {
            std::fs::create_dir_all(target)
                .map_err(CoreError::io("create an unpacked folder", target))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(CoreError::io("create an unpacked folder", parent))?;
        }
        let mut entry = zip
            .by_index(index)
            .map_err(|error| refused(&error.to_string()))?;
        let mut out = std::fs::File::create(target)
            .map_err(CoreError::io("write an unpacked file", target))?;
        std::io::copy(&mut entry, &mut out)
            .map_err(CoreError::io("write an unpacked file", target))?;
    }

    Ok(())
}

fn refused(reason: &str) -> CoreError {
    CoreError::Archive(reason.to_owned())
}

/// The path of an entry as plain relative components, or `None` when the name
/// is absolute, has a drive letter or a `..` part, or is empty.
///
/// Backslashes count as separators on every system, so a name that would only
/// escape on Windows is refused on macOS too.
fn safe_relative_path(name: &str) -> Option<PathBuf> {
    if name.contains(':') || name.contains('\0') {
        return None;
    }
    let normalised = name.replace('\\', "/");
    if normalised.starts_with('/') {
        return None;
    }

    let mut path = PathBuf::new();
    for component in Path::new(&normalised).components() {
        match component {
            Component::Normal(part) => path.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if path.as_os_str().is_empty() {
        None
    } else {
        Some(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Write a zip holding `entries` to `path`.
    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).expect("create the test archive");
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in entries {
            writer
                .start_file(*name, options)
                .expect("start a test entry");
            writer.write_all(body).expect("write a test entry");
        }
        writer.finish().expect("finish the test archive");
    }

    #[test]
    fn unpacks_files_and_nested_folders() {
        let temp = tempfile::tempdir().expect("a temporary directory");
        let archive = temp.path().join("bepinex.zip");
        let destination = temp.path().join("out");
        write_zip(
            &archive,
            &[
                ("winhttp.dll", b"MZ"),
                ("BepInEx/core/BepInEx.Core.dll", b"core"),
                ("doorstop_config.ini", b"[General]"),
            ],
        );

        extract(&archive, &destination).expect("the archive unpacks");

        assert_eq!(
            std::fs::read(destination.join("winhttp.dll")).expect("the proxy"),
            b"MZ"
        );
        assert_eq!(
            std::fs::read(
                destination
                    .join("BepInEx")
                    .join("core")
                    .join("BepInEx.Core.dll")
            )
            .expect("the nested file"),
            b"core"
        );
    }

    #[test]
    fn one_escaping_entry_refuses_the_whole_archive() {
        for hostile in [
            "../escaped.txt",
            "BepInEx/../../escaped.txt",
            "/escaped.txt",
            "C:/escaped.txt",
        ] {
            let temp = tempfile::tempdir().expect("a temporary directory");
            let archive = temp.path().join("bepinex.zip");
            let destination = temp.path().join("nested").join("out");
            write_zip(&archive, &[("winhttp.dll", b"MZ"), (hostile, b"pwned")]);

            let result = extract(&archive, &destination);

            assert!(
                matches!(result, Err(CoreError::Archive(_))),
                "{hostile} must be refused"
            );
            assert!(
                !destination.exists(),
                "nothing is written when {hostile} is refused"
            );
            assert!(!temp.path().join("escaped.txt").exists());
            assert!(!temp.path().join("nested").join("escaped.txt").exists());
        }
    }

    #[test]
    fn entry_names_are_judged_the_same_on_every_system() {
        assert_eq!(
            safe_relative_path("dotnet/a.txt"),
            Some(PathBuf::from("dotnet").join("a.txt"))
        );
        assert_eq!(safe_relative_path("./a.txt"), Some(PathBuf::from("a.txt")));
        for refused in [
            "..\\a.txt",
            "dotnet\\..\\..\\a.txt",
            "\\a.txt",
            "C:\\a.txt",
            "..",
            "",
            "/",
        ] {
            assert_eq!(
                safe_relative_path(refused),
                None,
                "{refused:?} must be refused"
            );
        }
    }

    #[test]
    fn a_file_that_is_not_a_zip_is_refused() {
        let temp = tempfile::tempdir().expect("a temporary directory");
        let archive = temp.path().join("bepinex.zip");
        std::fs::write(&archive, b"<html>not a zip</html>").expect("write the fake archive");

        let result = extract(&archive, &temp.path().join("out"));

        assert!(matches!(result, Err(CoreError::Archive(_))));
    }
}
