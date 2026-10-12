//! The commands the page may call, and the events it listens to.
//!
//! Every command is `async`: a synchronous Tauri command runs on the main
//! thread and would freeze the window while it waits for the disk or the
//! network. Almost every one answers with the next [`Snapshot`], so the page
//! never has to guess what changed.
//!
//! The launcher reads the game folder for every snapshot, so each call that is
//! not `async` in the core runs on a blocking thread, through [`blocking`].

use std::sync::Arc;

use oa_mod_launcher_core::{Launcher, Progress, RemovalOutcome, RemovalPlan, Snapshot, files};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use tauri_specta::Event;

use crate::updater::SelfUpdate;
use crate::{AppState, log};

/// Sent while the mod, or BepInEx before it, is downloaded and installed.
#[derive(Debug, Clone, Serialize, Type, Event)]
pub struct InstallProgress(pub Progress);

/// Sent when the launcher changed without a command asking for it: the game
/// was started or closed, or BepInEx finished preparing the game.
#[derive(Debug, Clone, Serialize, Type, Event)]
pub struct SnapshotChanged(pub Snapshot);

/// A folder the page may ask to see in the file manager.
#[derive(Debug, Clone, Copy, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Folder {
    /// The game folder.
    Game,
    /// The launcher's own folder, which holds `launcher.log`.
    Logs,
    /// Where other people's plugins are copied before a full removal.
    Backups,
}

/// Refuse to start something the launcher's own update would cut off: its
/// installer ends this process as soon as the download is checked.
fn refuse_during_self_update(own: &SelfUpdate) -> Result<(), String> {
    if own.is_installing() {
        return Err("The launcher is updating itself and restarts when that is done".to_owned());
    }
    Ok(())
}

/// Run `work` with the launcher on a thread that may wait for the disk.
async fn blocking<T, F>(state: &AppState, work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&Launcher) -> T + Send + 'static,
{
    let launcher = Arc::clone(state.launcher()?);
    tauri::async_runtime::spawn_blocking(move || work(&launcher))
        .await
        .map_err(|error| format!("The launcher stopped in the middle of a step: {error}"))
}

/// The same, for a step that can fail. `what` names it in the log.
async fn step<T, F>(state: &AppState, what: &'static str, work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&Launcher) -> oa_mod_launcher_core::Result<T> + Send + 'static,
{
    blocking(state, move |launcher| {
        let result = work(launcher).map_err(|error| error.to_string());
        match &result {
            Ok(_) => log::write(launcher.layout(), &format!("{what}: done")),
            Err(error) => log::write(launcher.layout(), &format!("{what} failed: {error}")),
        }
        result
    })
    .await?
}

/// What the window should show now. Reads local state only.
#[tauri::command]
#[specta::specta]
pub async fn snapshot(state: State<'_, AppState>) -> Result<Snapshot, String> {
    blocking(&state, Launcher::snapshot).await
}

/// Look at the disk again: the game folder, what is installed, whether the
/// game runs. Asks nobody on the network.
#[tauri::command]
#[specta::specta]
pub async fn refresh(state: State<'_, AppState>) -> Result<Snapshot, String> {
    blocking(&state, Launcher::refresh).await
}

/// Ask the release address which mod version is the newest.
#[tauri::command]
#[specta::specta]
pub async fn check_for_updates(state: State<'_, AppState>) -> Result<Snapshot, String> {
    let launcher = Arc::clone(state.launcher()?);
    let snapshot = launcher.check().await;
    log::write(
        launcher.layout(),
        &format!("update check: {:?}", snapshot.remote),
    );
    Ok(snapshot)
}

/// Install the mod, with BepInEx first when the game has none, or replace the
/// mod with the newest build.
#[tauri::command]
#[specta::specta]
pub async fn install(
    app: AppHandle,
    state: State<'_, AppState>,
    own: State<'_, SelfUpdate>,
) -> Result<Snapshot, String> {
    refuse_during_self_update(&own)?;
    let launcher = Arc::clone(state.launcher()?);
    let report = |progress: Progress| {
        let _ = InstallProgress(progress).emit(&app);
    };

    match launcher.install(&report).await {
        Ok(snapshot) => {
            log::write(
                launcher.layout(),
                &format!("installed {:?}", snapshot.mod_installed),
            );
            Ok(snapshot)
        }
        Err(error) => {
            log::write(launcher.layout(), &format!("install failed: {error}"));
            Err(error.to_string())
        }
    }
}

/// Ask Steam to start the game.
#[tauri::command]
#[specta::specta]
pub async fn start_game(state: State<'_, AppState>) -> Result<Snapshot, String> {
    step(&state, "game start", Launcher::start_game).await
}

/// What a removal would delete, keep and copy away. Changes nothing.
#[tauri::command]
#[specta::specta]
pub async fn removal_preview(state: State<'_, AppState>) -> Result<RemovalPlan, String> {
    blocking(&state, |launcher| {
        launcher
            .removal_preview()
            .map_err(|error| error.to_string())
    })
    .await?
}

/// Remove the mod and its settings. BepInEx and every other plugin stay.
#[tauri::command]
#[specta::specta]
pub async fn remove_mod(
    state: State<'_, AppState>,
    own: State<'_, SelfUpdate>,
) -> Result<Snapshot, String> {
    refuse_during_self_update(&own)?;
    step(&state, "remove only the mod", Launcher::remove_mod).await
}

/// Remove everything BepInEx brought into the game folder.
///
/// Answers with more than a snapshot: where other people's files were copied
/// to, and what was left in the game folder on purpose.
#[tauri::command]
#[specta::specta]
pub async fn remove_all(
    include_unrecognised: bool,
    state: State<'_, AppState>,
    own: State<'_, SelfUpdate>,
) -> Result<RemovalOutcome, String> {
    refuse_during_self_update(&own)?;
    step(&state, "remove everything", move |launcher| {
        launcher.remove_all(include_unrecognised)
    })
    .await
}

/// Let the player choose the game folder.
///
/// The dialog opens here and its answer never passes through the page, so the
/// page cannot name a folder. Closing the dialog changes nothing.
#[tauri::command]
#[specta::specta]
pub async fn pick_game_folder(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<Snapshot, String> {
    blocking(&state, move |launcher| {
        let picked = window
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("Choose the Golf With Your Friends folder")
            .blocking_pick_folder();
        let Some(picked) = picked else {
            return Ok(launcher.snapshot());
        };
        let folder = picked.into_path().map_err(|error| error.to_string())?;
        let result = launcher
            .set_game_dir(&folder)
            .map_err(|error| error.to_string());
        log::write(
            launcher.layout(),
            &match &result {
                Ok(_) => format!("game folder chosen: {}", files::display(&folder)),
                Err(error) => format!("game folder refused: {error}"),
            },
        );
        result
    })
    .await?
}

/// Forget the folder chosen by hand and go back to the one Steam names.
#[tauri::command]
#[specta::specta]
pub async fn reset_game_folder(state: State<'_, AppState>) -> Result<Snapshot, String> {
    step(&state, "reset the game folder", Launcher::reset_game_dir).await
}

/// Turn the update check on start on or off.
#[tauri::command]
#[specta::specta]
pub async fn set_check_on_start(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<Snapshot, String> {
    blocking(&state, move |launcher| {
        launcher
            .set_check_on_start(enabled)
            .map_err(|error| error.to_string())
    })
    .await?
}

/// Show a folder in the file manager of the system.
///
/// The page names one of three folders and never passes a path, so this cannot
/// be used to open anything else. The game folder is never created here.
#[tauri::command]
#[specta::specta]
pub async fn open_folder(folder: Folder, state: State<'_, AppState>) -> Result<(), String> {
    blocking(&state, move |launcher| {
        let layout = launcher.layout();
        let path = match folder {
            Folder::Game => launcher
                .game_dir()
                .ok_or_else(|| "There is no game folder to open yet".to_owned())?,
            Folder::Logs => layout.root().to_path_buf(),
            Folder::Backups => layout.backups_dir(),
        };
        if !matches!(folder, Folder::Game) {
            std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        }

        let program = if cfg!(windows) {
            "explorer.exe"
        } else if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        // Without the `\\?\` prefix of a canonical path, which the file
        // manager does not open.
        std::process::Command::new(program)
            .arg(files::display(&path))
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("Could not open the folder: {error}"))
    })
    .await?
}
