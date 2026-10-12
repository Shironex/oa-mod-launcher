//! Unpacking a downloaded zip.
//!
//! The archive is checked as a whole before a single byte is written. An entry
//! with an absolute path, a drive letter or a `..` part, one whose name Windows
//! would rewrite (a trailing dot or space, a device name such as `CON`), or
//! one that is a symbolic link, makes the launcher refuse the entire archive:
//! the zip the release names never contains one, so its presence means the
//! file is not what it claims to be.
//!
//! The sizes an archive declares are checked first, and the bytes that are
//! really written are counted against the same cap, so an entry that unpacks
//! to more than it declares stops there.

use std::path::{Component, Path, PathBuf};

use crate::error::{CoreError, Result};
use crate::steam::is_bare_name;

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
    extract_within(archive, destination, MAX_ENTRIES, MAX_UNPACKED_BYTES)
}

/// [`extract`] with the two caps as arguments, so a test can reach them with
/// a small archive.
fn extract_within(
    archive: &Path,
    destination: &Path,
    max_entries: usize,
    max_bytes: u64,
) -> Result<()> {
    let file = std::fs::File::open(archive)
        .map_err(CoreError::io("open the downloaded archive", archive))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|error| refused(&error.to_string()))?;

    if zip.len() > max_entries {
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
        if total > max_bytes {
            return Err(refused("it unpacks to more than 2 GB"));
        }
        targets.push((destination.join(relative), entry.is_dir()));
    }

    std::fs::create_dir_all(destination)
        .map_err(CoreError::io("create the unpack directory", destination))?;

    let mut written: u64 = 0;
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
        // One byte past what is left, so "exactly the cap" and "more than
        // the cap" can be told apart without writing the rest.
        let mut capped = std::io::Read::take(&mut entry, max_bytes - written + 1);
        written += std::io::copy(&mut capped, &mut out)
            .map_err(CoreError::io("write an unpacked file", target))?;
        if written > max_bytes {
            return Err(refused("it unpacks to more than it declares"));
        }
    }

    Ok(())
}

fn refused(reason: &str) -> CoreError {
    CoreError::Archive(reason.to_owned())
}

/// The path of an entry as plain relative components, or `None` when the name
/// is absolute, has a drive letter or a `..` part, is empty, or has a part
/// that is not a plain name on Windows (see [`crate::steam::is_bare_name`]).
///
/// Backslashes count as separators on every system, and so do the Windows
/// rules for names, so a name that would only escape or land on another file
/// on Windows is refused on macOS too.
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
            Component::Normal(part) if part.to_str().is_some_and(is_bare_name) => path.push(part),
            Component::Normal(_) => return None,
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

    /// Rewrite the size every header of the zip at `path` declares for its
    /// entries, without touching the data: what a hostile archive does to get
    /// past a check of the declared sizes.
    fn declare_size(path: &Path, size: u32) {
        let mut bytes = std::fs::read(path).expect("the test archive");
        // (signature, where the uncompressed size is in that header)
        for (signature, offset) in [(b"PK\x03\x04", 22), (b"PK\x01\x02", 24)] {
            let mut at = 0;
            while let Some(found) = bytes[at..]
                .windows(4)
                .position(|window| window == signature)
            {
                let field = at + found + offset;
                bytes[field..field + 4].copy_from_slice(&size.to_le_bytes());
                at = field;
            }
        }
        std::fs::write(path, bytes).expect("the patched archive");
    }

    #[test]
    fn a_name_windows_would_rewrite_refuses_the_whole_archive() {
        for hostile in [
            // Win32 drops a trailing dot or space, so these would land on
            // the name without it.
            "winhttp.dll.",
            "winhttp.dll ",
            "BepInEx./core/a.dll",
            "dotnet /a.dll",
            // Device names, with or without an extension, in any case.
            "CON",
            "con.txt",
            "dotnet/NUL",
            "BepInEx/core/PRN.dll",
            "AUX",
            "COM1",
            "dotnet/com9.dll",
            "LPT1",
            "lpt9.tar.gz",
            "nul .txt",
        ] {
            assert_eq!(
                safe_relative_path(hostile),
                None,
                "{hostile:?} must be refused"
            );

            let temp = tempfile::tempdir().expect("a temporary directory");
            let archive = temp.path().join("bepinex.zip");
            let destination = temp.path().join("out");
            write_zip(&archive, &[("winhttp.dll", b"MZ"), (hostile, b"pwned")]);

            let result = extract(&archive, &destination);

            assert!(
                matches!(result, Err(CoreError::Archive(_))),
                "{hostile:?} must be refused"
            );
            assert!(
                !destination.exists(),
                "nothing is written when {hostile:?} is refused"
            );
        }
        // Names that only look alike are fine.
        for fine in ["console.dll", "dotnet/COM10.dll", "BepInEx/core/.hidden"] {
            assert!(
                safe_relative_path(fine).is_some(),
                "{fine:?} is a plain name"
            );
        }
    }

    #[test]
    fn an_archive_with_more_entries_than_the_cap_is_refused() {
        let temp = tempfile::tempdir().expect("a temporary directory");
        let archive = temp.path().join("bepinex.zip");
        let destination = temp.path().join("out");
        let names: Vec<String> = (0..=MAX_ENTRIES).map(|index| format!("f{index}")).collect();
        let entries: Vec<(&str, &[u8])> =
            names.iter().map(|name| (name.as_str(), &b""[..])).collect();
        write_zip(&archive, &entries);

        let result = extract(&archive, &destination);

        assert!(
            matches!(&result, Err(CoreError::Archive(reason)) if reason.contains("too many")),
            "{result:?}"
        );
        assert!(!destination.exists());
    }

    #[test]
    fn an_archive_that_declares_more_than_the_cap_is_refused_before_a_byte_is_written() {
        let temp = tempfile::tempdir().expect("a temporary directory");
        let archive = temp.path().join("bepinex.zip");
        let destination = temp.path().join("out");
        write_zip(&archive, &[("winhttp.dll", b"MZ")]);
        declare_size(
            &archive,
            u32::try_from(MAX_UNPACKED_BYTES + 1).expect("the cap fits a zip header"),
        );

        let result = extract(&archive, &destination);

        assert!(
            matches!(&result, Err(CoreError::Archive(reason)) if reason.contains("2 GB")),
            "{result:?}"
        );
        assert!(!destination.exists());
    }

    #[test]
    fn an_entry_that_unpacks_to_more_than_it_declares_stops_at_the_cap() {
        let temp = tempfile::tempdir().expect("a temporary directory");
        let archive = temp.path().join("bepinex.zip");
        let destination = temp.path().join("out");
        // 64 KiB of zeros that claim to be one byte: the declared sizes pass
        // a cap of 1 KiB, the bytes that would be written do not.
        write_zip(&archive, &[("dotnet/bomb.bin", &[0_u8; 64 * 1024])]);
        declare_size(&archive, 1);

        let result = extract_within(&archive, &destination, MAX_ENTRIES, 1024);

        assert!(matches!(result, Err(CoreError::Archive(_))), "{result:?}");
        let written = std::fs::metadata(destination.join("dotnet").join("bomb.bin"))
            .map_or(0, |metadata| metadata.len());
        assert!(written <= 1025, "{written} bytes were written past the cap");
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
