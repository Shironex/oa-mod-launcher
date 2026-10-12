//! The gate before every publish: is this `manifest.json` signed by the
//! release key, and does it describe the DLL that is about to be published?
//!
//! ```sh
//! cargo run -p oa-mod-launcher-core --example verify_manifest -- path/to/manifest.json --mod path/to/OaMod-0.1.0.dll
//! ```
//!
//! It reads `manifest.json.sig` next to the file and runs the same check the
//! launcher runs, against the keys a release build trusts. The development key
//! is never among them, so a manifest signed with it by mistake fails here.
//! It also reads the manifest the way the launcher does, because a signed file
//! the launcher cannot parse helps nobody. With `--mod`, the size and the
//! SHA-256 of that file must be the ones the manifest names.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use oa_mod_launcher_core::Manifest;
use oa_mod_launcher_core::files;
use oa_mod_launcher_core::signature::{self, RELEASE_KEYS};

fn main() -> ExitCode {
    match check() {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(reason) => {
            eprintln!("NOT VERIFIED: {reason}");
            ExitCode::FAILURE
        }
    }
}

fn check() -> Result<String, String> {
    const USAGE: &str = "usage: verify_manifest <path to manifest.json> [--mod <path to the dll>]";
    let mut path = None;
    let mut mod_path = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--mod" {
            mod_path = Some(PathBuf::from(args.next().ok_or(USAGE)?));
        } else if path.is_none() {
            path = Some(PathBuf::from(arg));
        } else {
            return Err(USAGE.to_owned());
        }
    }
    let path = path.ok_or(USAGE)?;

    let text = signed_text(&path)?;
    let manifest = Manifest::parse(&text).map_err(|error| {
        format!("the signature is good, but the launcher would refuse the manifest: {error}")
    })?;
    let mut message = format!(
        "verified: manifest of version {} is signed by the release key",
        manifest.version
    );

    if let Some(mod_path) = mod_path {
        let (size, sha256) = files::sha256_file(&mod_path).map_err(|error| error.to_string())?;
        if size != manifest.mod_file.size {
            return Err(format!(
                "{} is {size} bytes, the manifest says {}",
                mod_path.display(),
                manifest.mod_file.size
            ));
        }
        if sha256 != manifest.mod_file.sha256 {
            return Err(format!(
                "{} has sha256 {sha256}, the manifest says {}",
                mod_path.display(),
                manifest.mod_file.sha256
            ));
        }
        message.push_str(&format!(
            "\nverified: {} is the mod the manifest names ({size} bytes, sha256 {sha256})",
            mod_path.display()
        ));
    }

    Ok(message)
}

/// The text of the file at `path`, once its `.sig` is accepted.
fn signed_text(path: &Path) -> Result<String, String> {
    let mut sig_path = path.as_os_str().to_owned();
    sig_path.push(".sig");
    let sig_path = PathBuf::from(sig_path);

    // Bytes, not text: the signature is over the exact bytes of the file.
    let bytes = std::fs::read(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let sig_text = std::fs::read_to_string(&sig_path)
        .map_err(|error| format!("could not read {}: {error}", sig_path.display()))?;

    signature::verify(&bytes, &sig_text, &RELEASE_KEYS).map_err(|_| {
        format!(
            "{} is not a signature of {} by the release key",
            sig_path.display(),
            path.display()
        )
    })?;

    String::from_utf8(bytes)
        .map_err(|_| format!("the signature is good, but {} is not text", path.display()))
}
