//! Is `<file>.sig` a signature of this file by this public key?
//!
//! ```sh
//! cargo run -p oa-mod-launcher-core --example verify_file -- <file> \
//!     [--pubkey <content of a .pub file>] [--version <x.y.z>]
//! ```
//!
//! The release workflow runs it on the launcher installer before anything is
//! published. Without `--pubkey` the key is `plugins.updater.pubkey` of
//! `src-tauri/tauri.conf.json`, the key every installed launcher checks its
//! update against, so an installer signed with any other key stops here and
//! not on a friend's computer.
//!
//! `--version` also demands that the signature is bound to that version (the
//! `version:` field of its trusted comment). The updater is configured with
//! `requireSignedVersion` and refuses an installer without it.
//!
//! The check is the one the launcher runs on a manifest
//! ([`signature::matching_key`]), with one key.

use std::path::PathBuf;
use std::process::ExitCode;

use oa_mod_launcher_core::signature;

/// Found from the crate, not from the working directory, so the example gives
/// the same answer wherever it is started.
const CONFIG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../src-tauri/tauri.conf.json"
);

const USAGE: &str =
    "usage: verify_file <file> [--pubkey <content of a .pub file>] [--version <x.y.z>]";

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
    let mut path = None;
    let mut pubkey = None;
    let mut version = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--pubkey" => pubkey = Some(arguments.next().ok_or(USAGE)?),
            "--version" => version = Some(arguments.next().ok_or(USAGE)?),
            _ if path.is_none() && !argument.starts_with("--") => {
                path = Some(PathBuf::from(argument));
            }
            _ => return Err(USAGE.to_owned()),
        }
    }
    let path = path.ok_or(USAGE)?;
    let mut sig_path = path.clone().into_os_string();
    sig_path.push(".sig");
    let sig_path = PathBuf::from(sig_path);

    let (pubkey, key_name) = match pubkey {
        Some(pubkey) => (pubkey, "the key given with --pubkey".to_owned()),
        None => (configured_key()?, format!("the updater key in {CONFIG}")),
    };

    // Bytes, not text: the signature is over the exact bytes of the file.
    let bytes = std::fs::read(&path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let sig_text = std::fs::read_to_string(&sig_path)
        .map_err(|error| format!("could not read {}: {error}", sig_path.display()))?;

    signature::verify(&bytes, &sig_text, &[pubkey]).map_err(|_| {
        format!(
            "{} is not a signature of {} by {key_name}",
            sig_path.display(),
            path.display()
        )
    })?;

    // Read only now: the trusted comment is worth something once the
    // signature that covers it has been accepted.
    let signed = signature::signed_version(&sig_text);
    if let Some(wanted) = &version
        && signed.as_deref() != Some(wanted.as_str())
    {
        return Err(format!(
            "the signature is good, but it is bound to {}, not to version {wanted}",
            signed
                .as_deref()
                .map_or("no version".to_owned(), |found| format!("version {found}"))
        ));
    }

    Ok(format!(
        "verified: {} is signed by {key_name}{}",
        path.display(),
        signed.map_or(String::new(), |found| format!(", for version {found}"))
    ))
}

/// `plugins.updater.pubkey` of `tauri.conf.json`.
fn configured_key() -> Result<String, String> {
    let text = std::fs::read_to_string(CONFIG)
        .map_err(|error| format!("could not read {CONFIG}: {error}"))?;
    let config: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| format!("{CONFIG} is not JSON: {error}"))?;
    config["plugins"]["updater"]["pubkey"]
        .as_str()
        .map(str::to_owned)
        .ok_or(format!("{CONFIG} has no plugins.updater.pubkey"))
}
