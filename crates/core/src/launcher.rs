//! The launcher as one object: look, check, install, start, remove.
//!
//! The window never decides anything. It shows a [`Snapshot`] and calls a
//! method; each of them answers with the next snapshot. The snapshot is made
//! from what is on disk at that moment, so it is also what a poll every few
//! seconds calls to notice that the game was started or closed.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::Serialize;

use crate::error::{CoreError, Result};
use crate::files;
use crate::game;
use crate::inspect::{self, BepInEx, Facts};
use crate::layout::Layout;
use crate::manifest::{self, Manifest};
use crate::net;
use crate::signature;
use crate::state::State;
use crate::steam;
use crate::version::Version;

/// What [`Snapshot::mod_installed`] says for a mod DLL the launcher did not
/// install: a local build, or a file copied by hand.
pub const UNKNOWN_BUILD: &str = "unknown build";

/// The address Steam starts the game through.
pub const START_URL: &str = "steam://rungameid/431240";

/// How long to wait before a file and its signature are fetched a second time.
const SIGNATURE_RETRY_PAUSE: Duration = Duration::from_secs(1);

/// Says whether the game in this folder is running. See
/// [`game::is_running`], which is what a normal start uses.
pub type RunningProbe = Arc<dyn Fn(&Path) -> bool + Send + Sync>;

/// Hands an address to the system to open. A normal start uses
/// [`open_with_system`]; a test records the address and starts nothing.
pub type Opener = Arc<dyn Fn(&str) -> std::io::Result<()> + Send + Sync>;

/// What a [`Launcher`] is built from. Tests change every part of it; the
/// shell uses [`Config::from_environment`].
#[derive(Clone)]
pub struct Config {
    /// Where the launcher keeps its own files.
    pub layout: Layout,
    /// The URL of `manifest.json`.
    pub manifest_url: String,
    /// Where every file a manifest names must be, see
    /// [`manifest::DOWNLOAD_PREFIX`].
    pub download_prefix: String,
    /// The version of this launcher, compared with `launcher.min`.
    pub launcher_version: Version,
    /// The public keys whose signature on a manifest is accepted, each as the
    /// content of a `.pub` file of the Tauri CLI.
    pub trusted_keys: Vec<String>,
    /// The Steam folder to look for the game in. `None` asks the system
    /// where Steam is, which is what a normal start does.
    pub steam_root: Option<PathBuf>,
    /// Says whether the game is running.
    pub running: RunningProbe,
    /// Opens the address that starts the game.
    pub open: Opener,
}

impl Config {
    /// The configuration of a normal start: per-user folder and the release
    /// manifest, unless the two environment overrides are set. The trusted
    /// keys are [`signature::RELEASE_KEYS`], and in a development build also
    /// the development key. No environment variable changes them.
    pub fn from_environment(launcher_version: Version) -> Option<Self> {
        let manifest_url = net::manifest_url_from_environment();
        Some(Self {
            layout: Layout::from_environment()?,
            download_prefix: download_prefix_for(&manifest_url),
            manifest_url,
            launcher_version,
            trusted_keys: signature::trusted_keys(),
            steam_root: None,
            running: Arc::new(game::is_running),
            open: Arc::new(open_with_system),
        })
    }
}

/// Where the files of a manifest read from `manifest_url` must be. For the
/// release address, and in every release build, that is the mod's releases.
/// A development build that was pointed at another manifest accepts files
/// next to that manifest instead, so a local server can stand in.
fn download_prefix_for(manifest_url: &str) -> String {
    if cfg!(debug_assertions)
        && manifest_url != net::DEFAULT_FEED
        && let Some(end) = manifest_url.rfind('/')
    {
        return manifest_url[..=end].to_owned();
    }
    manifest::DOWNLOAD_PREFIX.to_owned()
}

/// Ask the system to open `address`. For [`START_URL`] that is Steam.
///
/// # Errors
///
/// The error of starting the helper program, or "unsupported" on a system
/// that is not Windows.
pub fn open_with_system(address: &str) -> std::io::Result<()> {
    if cfg!(windows) {
        std::process::Command::new("explorer.exe")
            .arg(address)
            .spawn()
            .map(|_| ())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "starting the game is only supported on Windows",
        ))
    }
}

/// The step an install is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// The manifest is read and the game folder is looked at.
    Checking,
    /// The BepInEx zip is arriving.
    #[serde(rename = "downloading_bepinex")]
    DownloadingBepInEx,
    /// BepInEx is unpacked and copied into the game folder.
    #[serde(rename = "installing_bepinex")]
    InstallingBepInEx,
    /// The mod is arriving.
    DownloadingMod,
    /// The mod is written into the game folder.
    InstallingMod,
}

/// One progress report of an install.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Progress {
    /// The current step.
    pub phase: Phase,
    /// Bytes received so far in this step.
    pub received: u64,
    /// Bytes this step expects. 0 for a step that downloads nothing.
    pub total: u64,
}

/// What the launcher is busy with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    /// Nothing.
    Idle,
    /// An install or an update.
    Installing(Phase),
    /// A removal.
    Removing,
}

/// The one thing the window shows as its main state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Status {
    /// The launcher's own settings or the game folder cannot be read.
    Error {
        /// Why, in one sentence.
        reason: String,
    },
    /// An install or an update is running.
    Installing {
        /// Its current step.
        phase: Phase,
    },
    /// A removal is running.
    Removing,
    /// No folder that is the game.
    GameNotFound,
    /// The game is running, so nothing may be changed.
    GameRunning,
    /// Nothing is installed and the player removed everything on purpose.
    Clean,
    /// The mod is not installed.
    NotInstalled,
    /// The mod is installed and BepInEx has not prepared the game yet: the
    /// next start takes minutes.
    FirstRunNeeded,
    /// Another mod build is waiting, or the one on disk is not one the
    /// launcher installed.
    UpdateAvailable,
    /// The mod is installed and current.
    UpToDate,
}

/// What [`status`] decides from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusFacts {
    /// Why the launcher cannot work at all, if it cannot.
    pub error: Option<String>,
    /// What the launcher is busy with.
    pub activity: Activity,
    /// Whether a game folder was found.
    pub game_found: bool,
    /// Whether the game is running.
    pub game_running: bool,
    /// Whether the game folder has no BepInEx at all.
    pub bepinex_absent: bool,
    /// Whether the mod DLL is there.
    pub mod_present: bool,
    /// Whether the player removed everything on purpose.
    pub opted_out: bool,
    /// Whether BepInEx has prepared the game.
    pub interop_ready: bool,
    /// Whether another mod build should replace the one on disk.
    pub update: bool,
}

/// Derive the status. The first rule that matches wins, in this order.
pub fn status(facts: &StatusFacts) -> Status {
    if let Some(reason) = &facts.error {
        return Status::Error {
            reason: reason.clone(),
        };
    }
    match facts.activity {
        Activity::Installing(phase) => return Status::Installing { phase },
        Activity::Removing => return Status::Removing,
        Activity::Idle => {}
    }
    if !facts.game_found {
        Status::GameNotFound
    } else if facts.game_running {
        Status::GameRunning
    } else if facts.bepinex_absent && !facts.mod_present && facts.opted_out {
        Status::Clean
    } else if !facts.mod_present {
        Status::NotInstalled
    } else if !facts.interop_ready {
        Status::FirstRunNeeded
    } else if facts.update {
        Status::UpdateAvailable
    } else {
        Status::UpToDate
    }
}

/// What the last update check found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Remote {
    /// No check has finished yet.
    Unknown,
    /// The check failed. What is installed still works.
    Offline {
        /// Why, in one sentence.
        reason: String,
    },
    /// The installed mod is the one the manifest names.
    UpToDate,
    /// Another mod build is waiting to be installed.
    UpdateAvailable {
        /// The version the manifest names.
        version: String,
        /// The size of the download in bytes, BepInEx included when it is
        /// not there yet.
        size: u64,
    },
    /// The release needs a newer launcher than this one.
    LauncherTooOld {
        /// The lowest launcher version the manifest accepts.
        required: String,
    },
}

/// Something the player should know that stops nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Warning {
    /// The installed game build is not one the mod was tried with.
    GameBuildUntested {
        /// Steam's build id of the installed game.
        build: String,
    },
    /// BepInEx in the game folder is a build the launcher does not know.
    ForeignBepInEx,
    /// The game was updated, so the next start takes minutes again.
    SlowNextStart,
    /// The release names another BepInEx build than the installed one. The
    /// launcher keeps the installed build.
    BepInExDiffers {
        /// The label of the installed build.
        installed: String,
        /// The label the release names.
        wanted: String,
    },
}

/// The game folder the launcher works on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GameInfo {
    /// The folder, as it is shown to a person.
    pub dir: String,
    /// Steam's build id of the installed game. `None` for a folder chosen by
    /// hand that Steam does not name.
    pub build_id: Option<String>,
    /// Whether Steam is downloading, updating or checking the game. Nothing
    /// is installed or removed meanwhile.
    pub steam_busy: bool,
}

/// Everything the window shows, at one moment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    /// The version of this launcher.
    pub launcher_version: String,
    /// The main state.
    pub status: Status,
    /// The game folder, when there is one.
    pub game: Option<GameInfo>,
    /// What kind of BepInEx the game folder holds.
    pub bepinex: BepInEx,
    /// The installed mod: its version, or [`UNKNOWN_BUILD`].
    pub mod_installed: Option<String>,
    /// The names in `BepInEx/plugins` that are not the mod.
    pub foreign_plugins: Vec<String>,
    /// The result of the last update check.
    pub remote: Remote,
    /// What the player should know.
    pub warnings: Vec<Warning>,
    /// One line about the newest release, from the manifest.
    pub notes: Option<String>,
    /// Whether the launcher checks for updates when it opens.
    pub check_on_start: bool,
}

/// The game folder and what Steam says about it.
#[derive(Debug, Clone)]
pub(crate) struct Located {
    /// The canonical game folder, the root of every path in it.
    pub(crate) root: PathBuf,
    pub(crate) build_id: Option<String>,
    pub(crate) steam_busy: bool,
}

/// The launcher. Cheap to share: every method takes `&self`.
pub struct Launcher {
    pub(crate) config: Config,
    pub(crate) client: reqwest::Client,
    inner: Mutex<Inner>,
    busy: Arc<AtomicBool>,
}

struct Inner {
    /// The state, or why it cannot be read.
    state: std::result::Result<State, String>,
    /// The last check: the manifest, or why there is none.
    last_check: Option<std::result::Result<Manifest, String>>,
    /// The manifest kept from an earlier start. It is only used to recognise
    /// files on disk, never to install from.
    cached: Option<Manifest>,
    activity: Activity,
}

/// Held while an install or a removal is in progress. Dropping it frees the
/// launcher for the next one and ends the activity, on every path out.
pub(crate) struct Lease<'a>(&'a Launcher);

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        self.0.lock().activity = Activity::Idle;
        self.0.busy.store(false, Ordering::Release);
    }
}

impl Launcher {
    /// Open the launcher: read the state and clear `staging/`.
    ///
    /// A state file that cannot be read does not stop the launcher from
    /// opening: the snapshot then has the status [`Status::Error`], and every
    /// later snapshot tries the file again.
    ///
    /// # Errors
    ///
    /// [`CoreError::Http`] when the network client cannot be set up.
    pub fn open(config: Config) -> Result<Arc<Self>> {
        let client = net::client(&config.launcher_version.to_string())?;
        let state = State::load(&config.layout.state_file()).map_err(|error| error.to_string());
        clean_staging(&config.layout);

        let cached = std::fs::read_to_string(config.layout.cached_manifest())
            .ok()
            .and_then(|text| Manifest::parse_from(&text, &config.download_prefix).ok());

        Ok(Arc::new(Self {
            config,
            client,
            inner: Mutex::new(Inner {
                state,
                last_check: None,
                cached,
                activity: Activity::Idle,
            }),
            busy: Arc::new(AtomicBool::new(false)),
        }))
    }

    /// The directory layout this launcher uses.
    pub fn layout(&self) -> &Layout {
        &self.config.layout
    }

    /// The game folder the launcher works on, when there is one.
    pub fn game_dir(&self) -> Option<PathBuf> {
        let state = self.state().ok()?;
        self.locate(&state).ok().map(|located| located.root)
    }

    /// What the window should show now, read fresh from disk.
    pub fn snapshot(&self) -> Snapshot {
        // A state file that could not be read is tried again every time, so
        // the error ends by itself when its cause is gone.
        {
            let mut inner = self.lock();
            if inner.state.is_err() {
                inner.state = State::load(&self.config.layout.state_file())
                    .map_err(|error| error.to_string());
            }
        }
        let (state, manifest, check, activity) = {
            let inner = self.lock();
            (
                inner.state.clone(),
                reference_manifest(&inner),
                inner.last_check.clone(),
                inner.activity,
            )
        };
        let mut error = state.as_ref().err().cloned();
        let state = state.unwrap_or_default();

        let located = self.locate(&state).ok();
        let facts = located.as_ref().and_then(|located| {
            inspect::inspect(
                &located.root,
                &state,
                manifest.as_ref().map(|manifest| &manifest.bepinex),
            )
            .map_err(|problem| error.get_or_insert(problem.to_string()).clone())
            .ok()
        });
        let game_running = located
            .as_ref()
            .is_some_and(|located| (self.config.running)(&located.root));

        let newest = match &check {
            Some(Ok(manifest)) => Some(manifest),
            _ => None,
        };
        let mod_installed = facts
            .as_ref()
            .and_then(|facts| facts.mod_sha256.as_deref())
            .map(|on_disk| installed_version(on_disk, &state, manifest.as_ref()));
        let remote = self.remote(&check, &state, facts.as_ref());
        let unknown_build = mod_installed.as_deref() == Some(UNKNOWN_BUILD);

        let status = status(&StatusFacts {
            error,
            activity,
            game_found: located.is_some(),
            game_running,
            bepinex_absent: facts
                .as_ref()
                .is_some_and(|facts| facts.bepinex.is_absent()),
            mod_present: mod_installed.is_some(),
            opted_out: state.opted_out,
            interop_ready: facts.as_ref().is_some_and(|facts| facts.interop_ready),
            update: unknown_build || matches!(remote, Remote::UpdateAvailable { .. }),
        });

        Snapshot {
            launcher_version: self.config.launcher_version.to_string(),
            status,
            warnings: warnings(located.as_ref(), facts.as_ref(), &state, manifest.as_ref()),
            game: located.map(|located| GameInfo {
                dir: files::display(&located.root),
                build_id: located.build_id,
                steam_busy: located.steam_busy,
            }),
            bepinex: facts
                .as_ref()
                .map_or(BepInEx::Absent, |facts| facts.bepinex.clone()),
            mod_installed,
            foreign_plugins: facts.map(|facts| facts.foreign_plugins).unwrap_or_default(),
            remote,
            notes: newest.and_then(|manifest| manifest.notes.clone()),
            check_on_start: state.check_on_start,
        }
    }

    /// The same as [`Launcher::snapshot`]. The name the poll calls.
    pub fn refresh(&self) -> Snapshot {
        self.snapshot()
    }

    /// Ask the release address which mod version is the newest.
    ///
    /// Never fails: a check that does not work becomes [`Remote::Offline`]
    /// and what is installed still works.
    pub async fn check(&self) -> Snapshot {
        let _ = self.check_now().await;
        self.snapshot()
    }

    /// Fetch, verify and parse the manifest, remember the result, and hand it
    /// out. An install starts with this, so it never works from a manifest
    /// that was read at another time.
    pub(crate) async fn check_now(&self) -> Result<Manifest> {
        let checked = self.fetch_manifest().await;
        let mut inner = self.lock();
        match &checked {
            Ok((manifest, bytes)) => {
                inner.last_check = Some(Ok(manifest.clone()));
                inner.cached = Some(manifest.clone());
                let cache = self.config.layout.cached_manifest();
                if let Some(parent) = cache.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&cache, bytes);
            }
            Err(error) => inner.last_check = Some(Err(error.to_string())),
        }
        drop(inner);

        let (manifest, _) = checked?;
        let required = manifest.minimum_launcher();
        if required > self.config.launcher_version {
            return Err(CoreError::LauncherTooOld {
                required: required.to_string(),
            });
        }
        Ok(manifest)
    }

    async fn fetch_manifest(&self) -> Result<(Manifest, Vec<u8>)> {
        let url = net::checked_url(&self.config.manifest_url)?;
        let bytes = self.fetch_signed(&url).await?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| CoreError::BadManifest("it is not text".to_owned()))?;
        let manifest = Manifest::parse_from(text, &self.config.download_prefix)?;
        Ok((manifest, bytes))
    }

    /// Fetch the file at `url` and its signature, and hand out the bytes only
    /// when a trusted key signed exactly them.
    ///
    /// A failed check is tried once more after a short pause. The file and
    /// its `.sig` are two separate downloads, so while a release is being
    /// published the launcher can get the new file with the old signature.
    async fn fetch_signed(&self, url: &url::Url) -> Result<Vec<u8>> {
        match self.fetch_signed_once(url).await {
            Err(CoreError::BadSignature) => {
                tokio::time::sleep(SIGNATURE_RETRY_PAUSE).await;
                self.fetch_signed_once(url).await
            }
            other => other,
        }
    }

    async fn fetch_signed_once(&self, url: &url::Url) -> Result<Vec<u8>> {
        let bytes = net::fetch_bytes(&self.client, url, "check for updates").await?;
        // A manifest without a signature file is as unsigned as one with a
        // wrong signature, and is reported the same way.
        let sig = net::fetch_bytes(
            &self.client,
            &net::signature_url(url),
            "verify the update information",
        )
        .await
        .map_err(|_| CoreError::BadSignature)?;
        let sig_text = std::str::from_utf8(&sig).map_err(|_| CoreError::BadSignature)?;
        signature::verify(&bytes, sig_text, &self.config.trusted_keys)?;
        Ok(bytes)
    }

    /// Ask Steam to start the game.
    ///
    /// The launcher gets no handle on the game: the next snapshots say
    /// [`Status::GameRunning`] for as long as it runs.
    ///
    /// # Errors
    ///
    /// [`CoreError::GameNotFound`] without a game folder,
    /// [`CoreError::GameRunning`] when it runs already and
    /// [`CoreError::Start`] when the system refuses.
    pub fn start_game(&self) -> Result<Snapshot> {
        let located = self.locate(&self.state()?)?;
        if (self.config.running)(&located.root) {
            return Err(CoreError::GameRunning);
        }
        (self.config.open)(START_URL).map_err(|error| CoreError::Start(error.to_string()))?;
        Ok(self.snapshot())
    }

    /// Use `dir` as the game folder from now on, when it is one.
    ///
    /// # Errors
    ///
    /// [`CoreError::BadGameFolder`] when it is not the game, and the errors of
    /// writing the state.
    pub fn set_game_dir(&self, dir: &Path) -> Result<Snapshot> {
        let root = game::validate(dir)?;
        // Stored as it is shown, so the state file holds a path a person can
        // read. It is made canonical again every time it is used.
        self.update_state(|state| state.game_dir_override = Some(files::display(&root).into()))?;
        Ok(self.snapshot())
    }

    /// Forget the folder chosen by hand and go back to the one Steam names.
    ///
    /// # Errors
    ///
    /// The errors of writing the state.
    pub fn reset_game_dir(&self) -> Result<Snapshot> {
        self.update_state(|state| state.game_dir_override = None)?;
        Ok(self.snapshot())
    }

    /// Turn the update check on start on or off.
    ///
    /// # Errors
    ///
    /// The errors of writing the state.
    pub fn set_check_on_start(&self, enabled: bool) -> Result<Snapshot> {
        self.update_state(|state| state.check_on_start = enabled)?;
        Ok(self.snapshot())
    }

    /// The state as it is now.
    pub(crate) fn state(&self) -> Result<State> {
        self.lock().state.clone().map_err(CoreError::State)
    }

    /// Change the state and write it. The change is kept in memory only when
    /// the file was written.
    pub(crate) fn update_state(&self, change: impl FnOnce(&mut State)) -> Result<()> {
        let mut inner = self.lock();
        let mut state = inner.state.clone().map_err(CoreError::State)?;
        change(&mut state);
        state.save(&self.config.layout.state_file())?;
        inner.state = Ok(state);
        Ok(())
    }

    /// The BepInEx block files on disk are recognised by: from the last
    /// check, else from the manifest kept from an earlier start.
    pub(crate) fn reference(&self) -> Option<manifest::BepInEx> {
        reference_manifest(&self.lock()).map(|manifest| manifest.bepinex)
    }

    /// Find the game folder: the one chosen by hand while it is a game
    /// folder, otherwise the one Steam names.
    pub(crate) fn locate(&self, state: &State) -> Result<Located> {
        let discovered = self
            .config
            .steam_root
            .clone()
            .map_or_else(steam::steam_root, Ok)
            .and_then(|root| steam::find_game(&root))
            .and_then(|found| {
                Ok(Located {
                    root: game::validate(&found.dir)?,
                    build_id: Some(found.build_id),
                    steam_busy: found.state_flags != 4,
                })
            });

        let chosen = state
            .game_dir_override
            .as_deref()
            .and_then(|dir| game::validate(dir).ok());
        match (chosen, discovered) {
            // The chosen folder is the one Steam names: Steam's facts apply.
            (Some(root), Ok(found)) if found.root == root => Ok(found),
            (Some(root), _) => Ok(Located {
                root,
                build_id: None,
                steam_busy: false,
            }),
            (None, discovered) => discovered,
        }
    }

    /// The game folder, once everything that must hold before a write holds:
    /// it is the game, Steam is not busy with it, and it is not running.
    pub(crate) fn writable_game(&self) -> Result<Located> {
        let located = self.locate(&self.state()?)?;
        if located.steam_busy {
            return Err(CoreError::SteamBusy);
        }
        self.refuse_while_running(&located.root)?;
        Ok(located)
    }

    /// Asked again right before the first write of every step.
    pub(crate) fn refuse_while_running(&self, root: &Path) -> Result<()> {
        if (self.config.running)(root) {
            Err(CoreError::GameRunning)
        } else {
            Ok(())
        }
    }

    /// Say why the game folder refused a write or a delete, when the game is
    /// the reason. Windows answers "access denied", not a sharing violation,
    /// for a DLL a running game has loaded, and that reads as a folder that
    /// is not writable. So the game is asked about once more, and a running
    /// game is what the player is told.
    pub(crate) fn explained(&self, error: CoreError) -> CoreError {
        if matches!(error, CoreError::NotWritable { .. })
            && self
                .game_dir()
                .is_some_and(|root| (self.config.running)(&root))
        {
            CoreError::GameRunning
        } else {
            error
        }
    }

    pub(crate) fn set_activity(&self, activity: Activity) {
        self.lock().activity = activity;
    }

    pub(crate) fn lease(&self, activity: Activity) -> Result<Lease<'_>> {
        if self.busy.swap(true, Ordering::AcqRel) {
            return Err(CoreError::Busy);
        }
        self.set_activity(activity);
        Ok(Lease(self))
    }

    fn remote(
        &self,
        check: &Option<std::result::Result<Manifest, String>>,
        state: &State,
        facts: Option<&Facts>,
    ) -> Remote {
        let manifest = match check {
            None => return Remote::Unknown,
            Some(Err(reason)) => {
                return Remote::Offline {
                    reason: reason.clone(),
                };
            }
            Some(Ok(manifest)) => manifest,
        };
        let required = manifest.minimum_launcher();
        if required > self.config.launcher_version {
            return Remote::LauncherTooOld {
                required: required.to_string(),
            };
        }

        // What is on disk decides, not what the state remembers: the mod is
        // current when the DLL is the one the manifest names.
        let current = match facts {
            Some(facts) => facts.mod_sha256.as_deref() == Some(manifest.mod_file.sha256.as_str()),
            None => state
                .mod_file
                .as_ref()
                .is_some_and(|record| record.sha256 == manifest.mod_file.sha256),
        };
        if current {
            return Remote::UpToDate;
        }
        let bepinex = if facts.is_some_and(|facts| facts.bepinex.is_absent()) {
            manifest.bepinex.size
        } else {
            0
        };
        Remote::UpdateAvailable {
            version: manifest.version.clone(),
            size: manifest.mod_file.size + bepinex,
        }
    }

    /// The lock is never held across an await, and no code panics while
    /// holding it, so a poisoned lock still holds a consistent value.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn reference_manifest(inner: &Inner) -> Option<Manifest> {
    match &inner.last_check {
        Some(Ok(manifest)) => Some(manifest.clone()),
        _ => inner.cached.clone(),
    }
}

/// The version of the mod DLL with the hash `on_disk`: the one the launcher
/// recorded, the one the manifest names, or [`UNKNOWN_BUILD`].
fn installed_version(on_disk: &str, state: &State, manifest: Option<&Manifest>) -> String {
    if let Some(record) = &state.mod_file
        && record.sha256 == on_disk
    {
        return record.version.clone();
    }
    if let Some(manifest) = manifest
        && manifest.mod_file.sha256 == on_disk
    {
        return manifest.version.clone();
    }
    UNKNOWN_BUILD.to_owned()
}

fn warnings(
    located: Option<&Located>,
    facts: Option<&Facts>,
    state: &State,
    manifest: Option<&Manifest>,
) -> Vec<Warning> {
    let mut warnings = Vec::new();
    if let Some(manifest) = manifest
        && let Some(build) = located.and_then(|located| located.build_id.as_ref())
        && !manifest.game.builds.contains(build)
    {
        warnings.push(Warning::GameBuildUntested {
            build: build.clone(),
        });
    }
    let Some(facts) = facts else {
        return warnings;
    };
    match &facts.bepinex {
        BepInEx::Foreign => warnings.push(Warning::ForeignBepInEx),
        BepInEx::Ours { label } | BepInEx::Adopted { label } => {
            if let Some(manifest) = manifest
                && let Some(record) = &state.bepinex
                && record.probe_sha256 != manifest.bepinex.probe_sha256
            {
                warnings.push(Warning::BepInExDiffers {
                    installed: label.clone(),
                    wanted: manifest.bepinex.label.clone(),
                });
            }
        }
        BepInEx::Absent | BepInEx::Unsupported { .. } => {}
    }
    if facts.slow_next_start {
        warnings.push(Warning::SlowNextStart);
    }
    warnings
}

/// Delete everything in `staging/`: the leftovers of an interrupted run.
/// The folder belongs to the launcher alone, and nothing in it is a link the
/// launcher made, so a tree with a link in it is left where it is.
pub(crate) fn clean_staging(layout: &Layout) {
    let staging = layout.staging_dir();
    let Ok(entries) = std::fs::read_dir(&staging) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if files::delete_tree(&path).is_err() {
            let _ = files::delete_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The facts of a game with the mod installed, prepared and current:
    /// the last row of the table. Each test changes what its row is about.
    fn settled() -> StatusFacts {
        StatusFacts {
            error: None,
            activity: Activity::Idle,
            game_found: true,
            game_running: false,
            bepinex_absent: false,
            mod_present: true,
            opted_out: false,
            interop_ready: true,
            update: false,
        }
    }

    /// Every later condition set at once, so a row is shown to win over all
    /// the rows below it.
    fn everything_at_once() -> StatusFacts {
        StatusFacts {
            error: Some("the settings are unreadable".to_owned()),
            activity: Activity::Installing(Phase::DownloadingMod),
            game_found: false,
            game_running: true,
            bepinex_absent: true,
            mod_present: false,
            opted_out: true,
            interop_ready: false,
            update: true,
        }
    }

    #[test]
    fn row_1_an_unreadable_state_is_an_error_before_anything_else() {
        assert_eq!(
            status(&everything_at_once()),
            Status::Error {
                reason: "the settings are unreadable".to_owned()
            }
        );
    }

    #[test]
    fn row_2_an_install_in_progress_shows_its_phase() {
        let facts = StatusFacts {
            error: None,
            ..everything_at_once()
        };
        assert_eq!(
            status(&facts),
            Status::Installing {
                phase: Phase::DownloadingMod
            }
        );
    }

    #[test]
    fn row_3_a_removal_in_progress_is_removing() {
        let facts = StatusFacts {
            error: None,
            activity: Activity::Removing,
            ..everything_at_once()
        };
        assert_eq!(status(&facts), Status::Removing);
    }

    #[test]
    fn row_4_without_a_game_folder_the_game_is_not_found() {
        let facts = StatusFacts {
            error: None,
            activity: Activity::Idle,
            ..everything_at_once()
        };
        assert_eq!(status(&facts), Status::GameNotFound);
    }

    #[test]
    fn row_5_a_running_game_wins_over_what_is_installed() {
        for base in [settled(), everything_at_once()] {
            let facts = StatusFacts {
                error: None,
                activity: Activity::Idle,
                game_found: true,
                game_running: true,
                ..base
            };
            assert_eq!(status(&facts), Status::GameRunning);
        }
    }

    #[test]
    fn row_6_nothing_installed_after_opting_out_is_clean() {
        let clean = StatusFacts {
            bepinex_absent: true,
            mod_present: false,
            opted_out: true,
            interop_ready: false,
            ..settled()
        };
        assert_eq!(status(&clean), Status::Clean);

        // Each of the three conditions is needed.
        for not_clean in [
            StatusFacts {
                opted_out: false,
                ..clean.clone()
            },
            StatusFacts {
                bepinex_absent: false,
                ..clean.clone()
            },
        ] {
            assert_eq!(status(&not_clean), Status::NotInstalled);
        }
        let with_mod = StatusFacts {
            mod_present: true,
            ..clean
        };
        assert_eq!(status(&with_mod), Status::FirstRunNeeded);
    }

    #[test]
    fn row_7_without_the_mod_it_is_not_installed() {
        let facts = StatusFacts {
            mod_present: false,
            update: true,
            ..settled()
        };
        assert_eq!(status(&facts), Status::NotInstalled);
    }

    #[test]
    fn row_8_the_mod_without_a_prepared_game_needs_a_first_run() {
        let facts = StatusFacts {
            interop_ready: false,
            update: true,
            ..settled()
        };
        assert_eq!(status(&facts), Status::FirstRunNeeded);
    }

    #[test]
    fn row_9_another_build_waiting_is_an_update() {
        let facts = StatusFacts {
            update: true,
            ..settled()
        };
        assert_eq!(status(&facts), Status::UpdateAvailable);
    }

    #[test]
    fn row_10_otherwise_it_is_up_to_date() {
        assert_eq!(status(&settled()), Status::UpToDate);
    }

    #[test]
    fn the_snapshot_is_tagged_the_way_the_window_reads_it() {
        let status = serde_json::to_value(Status::Installing {
            phase: Phase::DownloadingBepInEx,
        })
        .expect("JSON");
        assert_eq!(status["kind"], "installing");
        assert_eq!(status["phase"], "downloading_bepinex");

        let warning = serde_json::to_value(Warning::GameBuildUntested {
            build: "1".to_owned(),
        })
        .expect("JSON");
        assert_eq!(warning["kind"], "game_build_untested");
        assert_eq!(
            serde_json::to_value(BepInEx::Absent).expect("JSON")["kind"],
            "absent"
        );
    }

    #[test]
    fn only_a_development_build_takes_files_from_next_to_another_manifest() {
        assert_eq!(
            download_prefix_for(net::DEFAULT_FEED),
            manifest::DOWNLOAD_PREFIX
        );
        let local = download_prefix_for("http://127.0.0.1:8123/feed/manifest.json");
        if cfg!(debug_assertions) {
            assert_eq!(local, "http://127.0.0.1:8123/feed/");
        } else {
            assert_eq!(local, manifest::DOWNLOAD_PREFIX);
        }
    }
}
