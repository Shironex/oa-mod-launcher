//! The launcher's own update.
//!
//! The mod is updated by the rules in the core. The launcher itself is
//! updated by `tauri-plugin-updater`, driven from here: the page calls the two
//! commands below and makes no request of its own. It has no permission for
//! the commands of the plugin either, so this file is the only way in.
//!
//! Nothing here needs the game side. The commands work when the mod check is
//! offline, when the manifest signature is refused, and when the core
//! [`Launcher`](oa_mod_launcher_core::Launcher) could not be opened at all: a
//! launcher that is broken in one of those ways is exactly the one that has
//! to be able to replace itself. [`AppState`] is read for two things only, the
//! question "is an install or a removal running" and the log file, and both
//! are skipped when it has no launcher.
//!
//! There is one updater key, the one in `tauri.conf.json`, and no spare. If
//! it is ever lost, no installed launcher can update itself again and
//! everyone installs the next one by hand.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use oa_mod_launcher_core::Status;
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, State};
use tauri_plugin_updater::{Error, Update, Updater, UpdaterExt};
use tauri_specta::Event;

use crate::{AppState, log};

/// How long reading `latest.json` may take, from request to last byte. The
/// plugin sets no timeout of its own.
const CHECK_TIMEOUT: Duration = Duration::from_secs(15);

/// How long connecting to a host may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the installer download may deliver no bytes before it is given
/// up. A total time limit would also end a slow download that is still
/// moving, so the download has this one only.
const STALL_TIMEOUT: Duration = Duration::from_secs(30);

/// What a check for a newer launcher found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LauncherUpdate {
    /// This launcher is the newest one.
    None,
    /// A newer launcher can be installed.
    Available {
        /// Its version.
        version: String,
        /// What changed, as written in `latest.json`.
        notes: Option<String>,
    },
}

/// Sent while the new launcher is being downloaded.
#[derive(Debug, Clone, Serialize, Type, Event)]
pub struct LauncherUpdateProgress {
    /// Bytes received so far.
    #[specta(type = f64)]
    received: u64,
    /// The size of the download, 0 when the server did not say.
    #[specta(type = f64)]
    total: u64,
}

/// Whether the launcher is replacing itself. Kept apart from [`AppState`], so
/// it exists when the core launcher does not.
#[derive(Default)]
pub struct SelfUpdate {
    installing: AtomicBool,
}

impl SelfUpdate {
    /// True from the first byte of the download until the launcher exits, or
    /// until the update has failed.
    pub(crate) fn is_installing(&self) -> bool {
        self.installing.load(Ordering::Acquire)
    }

    /// Mark the update as running. Refused while one already is, so two
    /// clicks cannot start two downloads.
    fn begin(&self) -> Result<Running<'_>, String> {
        if self.installing.swap(true, Ordering::AcqRel) {
            return Err("The launcher update is already running".to_owned());
        }
        Ok(Running(&self.installing))
    }
}

/// Held while an update runs. Dropping it clears the mark on every path out,
/// which only happens when the update did not go through.
struct Running<'a>(&'a AtomicBool);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Ask whether a newer launcher exists. Reads `latest.json` only.
///
/// A development build answers "none" without a request, so `pnpm tauri dev`
/// never tries to replace itself.
#[tauri::command]
#[specta::specta]
pub async fn check_launcher_update(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<LauncherUpdate, String> {
    if cfg!(debug_assertions) {
        return Ok(LauncherUpdate::None);
    }

    let checked = match updater(&app) {
        Ok(updater) => updater.check().await,
        Err(error) => Err(error),
    };
    match checked {
        Ok(update) => {
            let found = describe(update.as_ref());
            note(&state, &format!("launcher update check: {found:?}"));
            Ok(found)
        }
        Err(error) => Err(failed(&state, "launcher update check", &error)),
    }
}

/// Download the newest launcher, check its signature and install it.
///
/// When it works this never answers: the plugin starts the installer and
/// ends this process, and the installer starts the new launcher.
#[tauri::command]
#[specta::specta]
pub async fn install_launcher_update(
    app: AppHandle,
    state: State<'_, AppState>,
    own: State<'_, SelfUpdate>,
) -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err("A development build does not update itself".to_owned());
    }

    let _running = own.begin()?;
    refuse_while_busy(&state)?;

    let (update, installer) = match fetch(&app).await {
        Ok(Some(fetched)) => fetched,
        Ok(None) => return Err("There is no newer launcher to install".to_owned()),
        Err(error) => return Err(failed(&state, "launcher update", &error)),
    };

    // Asked a second time: the download took a while, and an install or a
    // removal may have begun in the meantime. From here on the commands for
    // both refuse, because `own` is marked.
    refuse_while_busy(&state)?;

    note(&state, &format!("installing launcher {}", update.version));
    update
        .install(&installer)
        .map_err(|error| failed(&state, "launcher update", &error))?;

    // Not reached on Windows, where `install` has already ended the process.
    // Called from a command thread, `restart` asks the event loop to exit
    // first, so the single instance lock is released before the new process
    // looks for it.
    app.restart()
}

/// Check, download and verify the signature. The plugin refuses the bytes
/// itself when the key in `tauri.conf.json` did not sign them, or when the
/// signature is not bound to the version `latest.json` names.
async fn fetch(app: &AppHandle) -> Result<Option<(Update, Vec<u8>)>, Error> {
    let Some(update) = updater(app)?.check().await? else {
        return Ok(None);
    };

    // The plugin reports the size of each piece, not the sum.
    let mut received: u64 = 0;
    let installer = update
        .download(
            |piece, total| {
                received += piece as u64;
                let progress = LauncherUpdateProgress {
                    received,
                    total: total.unwrap_or(0),
                };
                let _ = progress.emit(app);
            },
            || {},
        )
        .await?;
    Ok(Some((update, installer)))
}

/// The plugin's updater with this launcher's rules for requests.
///
/// The plugin checks that the configured address is https, and nothing after
/// that: not the redirect GitHub answers with, and not the download address
/// inside `latest.json`. `https_only` closes both. The client settings travel
/// with the [`Update`], so they hold for the download as well.
fn updater(app: &AppHandle) -> Result<Updater, Error> {
    app.updater_builder()
        .timeout(CHECK_TIMEOUT)
        .configure_client(|client| {
            client
                .https_only(true)
                .connect_timeout(CONNECT_TIMEOUT)
                .read_timeout(STALL_TIMEOUT)
        })
        .build()
}

fn describe(update: Option<&Update>) -> LauncherUpdate {
    match update {
        None => LauncherUpdate::None,
        Some(update) => LauncherUpdate::Available {
            version: update.version.clone(),
            notes: update
                .body
                .as_deref()
                .map(str::trim)
                .filter(|notes| !notes.is_empty())
                .map(str::to_owned),
        },
    }
}

/// Refuse while an install or a removal is running.
///
/// The installer replaces the launcher and starts it again, which would cut
/// either of them off half way. A running game is no reason to wait: Steam
/// started it and the launcher holds nothing of it. A launcher that could not
/// be opened runs nothing, so there is nothing to wait for.
fn refuse_while_busy(state: &AppState) -> Result<(), String> {
    let Ok(launcher) = state.launcher() else {
        return Ok(());
    };
    match refusal(&launcher.snapshot().status) {
        Some(reason) => Err(reason.to_owned()),
        None => Ok(()),
    }
}

/// Why the launcher cannot be replaced right now, as a sentence for the page.
fn refusal(status: &Status) -> Option<&'static str> {
    match status {
        Status::Installing { .. } => {
            Some("Wait until the install has finished, then update the launcher")
        }
        Status::Removing => Some("Wait until the removal has finished, then update the launcher"),
        _ => None,
    }
}

/// An error of the plugin as a sentence for the page. The exact error goes to
/// `launcher.log`.
fn explain(error: &Error) -> String {
    match error {
        Error::Minisign(_)
        | Error::Base64(_)
        | Error::SignatureUtf8(_)
        | Error::SignedVersionMismatch { .. }
        | Error::MissingSignedVersion => {
            "The launcher update is not signed with a key this launcher trusts, so it was not installed"
        }
        Error::Reqwest(_) | Error::Network(_) | Error::ReleaseNotFound => {
            "Could not reach the launcher update server"
        }
        Error::Serialization(_)
        | Error::Semver(_)
        | Error::InvalidUpdaterFormat
        | Error::TargetNotFound(_)
        | Error::TargetsNotFound(_) => "The launcher update information could not be read",
        _ => "The launcher update could not be installed",
    }
    .to_owned()
}

/// Write the exact error to the log and hand out the sentence for the page.
fn failed(state: &AppState, what: &str, error: &Error) -> String {
    note(state, &format!("{what} failed: {error}"));
    explain(error)
}

/// One line in `launcher.log`, when there is a launcher to say where it is.
fn note(state: &AppState, message: &str) {
    if let Ok(launcher) = state.launcher() {
        log::write(launcher.layout(), message);
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use oa_mod_launcher_core::Phase;

    use super::*;

    /// `tauri.conf.json` as the build reads it.
    fn config() -> serde_json::Value {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json"))
            .expect("tauri.conf.json is readable");
        serde_json::from_str(&text).expect("tauri.conf.json is JSON")
    }

    /// The key id inside a key in the form the Tauri CLI writes it: base64 of
    /// a two line minisign `.pub` file. Fails when it is not such a key.
    fn key_id(encoded: &str) -> String {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .expect("the key is base64");
        let text = String::from_utf8(bytes).expect("the key file is text");
        minisign_verify::PublicKey::decode(&text).expect("the key file is a minisign public key");
        text.lines()
            .next()
            .and_then(|comment| comment.rsplit(' ').next())
            .expect("the key file has a comment line")
            .to_owned()
    }

    #[test]
    fn the_version_has_one_source() {
        // Without `version` in `tauri.conf.json`, Tauri takes the version of
        // the package. So the version the updater compares and the one the
        // `launcher.min` rule compares are the same `CARGO_PKG_VERSION`.
        assert!(
            config().get("version").is_none(),
            "tauri.conf.json must not set `version`: the launcher version lives in Cargo.toml only"
        );
    }

    #[test]
    fn the_updater_key_is_the_release_key_and_none_of_the_others() {
        let config = config();
        let configured = config["plugins"]["updater"]["pubkey"]
            .as_str()
            .expect("plugins.updater.pubkey is set");

        assert_eq!(key_id(configured), "A21E2DE99962E533");
        // Not the key that signs manifests, and not the development key:
        // each key signs one kind of file.
        for other in oa_mod_launcher_core::signature::trusted_keys() {
            assert_ne!(key_id(configured), key_id(&other));
        }
        assert_eq!(config["plugins"]["updater"]["requireSignedVersion"], true);
    }

    #[test]
    fn updates_come_from_the_launcher_repository_over_https() {
        let config = config();
        let endpoints = config["plugins"]["updater"]["endpoints"]
            .as_array()
            .expect("plugins.updater.endpoints is a list");
        assert_eq!(
            endpoints,
            &[serde_json::json!(
                "https://github.com/Shironex/oa-mod-launcher/releases/latest/download/latest.json"
            )]
        );
        assert_eq!(config["bundle"]["createUpdaterArtifacts"], true);
        assert!(
            config["plugins"]["updater"]
                .get("dangerousInsecureTransportProtocol")
                .is_none()
        );
    }

    #[test]
    fn the_launcher_is_not_replaced_during_an_install_or_a_removal() {
        assert!(
            refusal(&Status::Installing {
                phase: Phase::DownloadingMod
            })
            .is_some()
        );
        assert!(refusal(&Status::Removing).is_some());
        for free in [
            Status::UpToDate,
            Status::GameRunning,
            Status::GameNotFound,
            Status::Error {
                reason: "state.json cannot be read".to_owned(),
            },
        ] {
            assert_eq!(refusal(&free), None, "{free:?}");
        }
    }

    #[test]
    fn only_one_update_runs_at_a_time_and_a_failed_one_frees_the_next() {
        let own = SelfUpdate::default();
        assert!(!own.is_installing());

        let first = own.begin().expect("the first update starts");
        assert!(own.is_installing());
        assert!(own.begin().is_err());

        drop(first);
        assert!(!own.is_installing());
        assert!(own.begin().is_ok());
    }

    #[test]
    fn a_refused_signature_is_explained_without_the_library_text() {
        let refused = minisign_verify::PublicKey::decode("not a key").expect_err("it is no key");
        let sentence = explain(&Error::Minisign(refused));
        assert!(sentence.contains("not signed with a key this launcher trusts"));
        assert!(!sentence.ends_with('.'));
    }
}
