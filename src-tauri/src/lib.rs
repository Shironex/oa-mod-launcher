//! The window around the launcher rules.
//!
//! This crate decides nothing. It opens the window, turns the methods of
//! [`oa_mod_launcher_core::Launcher`] into commands, and forwards progress and
//! changes to the page as events. The page makes no network request.
//!
//! The one thing that does not come from the core is the launcher's own
//! update, in [`updater`].

mod bindings;
pub mod commands;
mod log;
pub mod updater;

use std::sync::Arc;
use std::time::Duration;

use oa_mod_launcher_core::{Config, Launcher, Snapshot, Version};
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

/// How often the disk is looked at again. This is how the window notices that
/// the game was started or closed, and that its first run is over.
const POLL: Duration = Duration::from_secs(3);

/// What the commands share.
pub struct AppState {
    /// The launcher, or the reason it could not be opened.
    launcher: Result<Arc<Launcher>, String>,
}

impl AppState {
    pub(crate) fn launcher(&self) -> Result<&Arc<Launcher>, String> {
        self.launcher.as_ref().map_err(Clone::clone)
    }
}

/// The version of this launcher, from the package version.
fn launcher_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Version {
        major: 0,
        minor: 0,
        patch: 0,
    })
}

fn open_launcher() -> Result<Arc<Launcher>, String> {
    let config = Config::from_environment(launcher_version())
        .ok_or_else(|| "The per-user application folder could not be found".to_owned())?;
    let launcher = Launcher::open(config).map_err(|error| error.to_string())?;
    log::write(
        launcher.layout(),
        &format!("launcher {} started", launcher_version()),
    );
    Ok(launcher)
}

/// Whether the page has to be told: only when what it would show differs from
/// what it was told last.
fn changed(last: &mut Option<Snapshot>, now: Snapshot) -> Option<Snapshot> {
    if last.as_ref() == Some(&now) {
        return None;
    }
    *last = Some(now.clone());
    Some(now)
}

/// Look at the disk every [`POLL`] and tell the page when something changed.
///
/// A thread of its own: a snapshot reads the game folder, and the thread
/// sleeps between two of them. It ends with the process.
fn poll(app: AppHandle, launcher: Arc<Launcher>) {
    std::thread::spawn(move || {
        let mut last = Some(launcher.refresh());
        loop {
            std::thread::sleep(POLL);
            if let Some(snapshot) = changed(&mut last, launcher.refresh()) {
                let _ = commands::SnapshotChanged(snapshot).emit(&app);
            }
        }
    });
}

/// Build and run the application.
///
/// # Panics
///
/// When the window system cannot be started at all. There is nothing to show
/// an error in at that point.
pub fn run() {
    // One TLS crypto provider for the whole process, chosen here.
    //
    // Two are compiled in: the core's HTTP client is built for `aws-lc-rs`
    // and the updater plugin for `ring`. With both present `rustls` picks no
    // default by itself. The plugin would then make `ring` the default at its
    // first update check, and every HTTP client built after that moment would
    // use it, so the provider of a request would depend on which check ran
    // first. Naming it here settles that before any client exists: the one
    // the core is tested with. The call only fails when a default is already
    // set, which cannot be the case on the first line of the program.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let specta = bindings::builder();

    tauri::Builder::default()
        // First, so a second start hands over before anything else is set up.
        .plugin(tauri_plugin_single_instance::init(
            |app, _arguments, _directory| {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            },
        ))
        // Registered without a permission in `capabilities/`, so the page
        // cannot call the plugin. Only `pick_game_folder` opens a dialog.
        .plugin(tauri_plugin_dialog::init())
        // The same: only `updater.rs` drives it.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState {
            launcher: open_launcher(),
        })
        .manage(updater::SelfUpdate::default())
        .invoke_handler(specta.invoke_handler())
        .setup(move |app| {
            specta.mount_events(app);
            if let Ok(launcher) = app.state::<AppState>().launcher() {
                poll(app.handle().clone(), Arc::clone(launcher));
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("the window system starts");
}

#[cfg(test)]
mod tests {
    use oa_mod_launcher_core::{BepInEx, Remote, Status};

    use super::*;

    fn snapshot(status: Status) -> Snapshot {
        Snapshot {
            launcher_version: "0.1.0".to_owned(),
            status,
            game: None,
            bepinex: BepInEx::Absent,
            bepinex_wanted: None,
            game_dir_chosen: false,
            mod_installed: None,
            foreign_plugins: Vec::new(),
            remote: Remote::Unknown,
            warnings: Vec::new(),
            notes: None,
            check_on_start: true,
        }
    }

    #[test]
    fn the_page_is_told_only_when_the_snapshot_differs() {
        let mut last = Some(snapshot(Status::UpToDate));

        assert!(changed(&mut last, snapshot(Status::UpToDate)).is_none());
        assert_eq!(
            changed(&mut last, snapshot(Status::GameRunning)).map(|told| told.status),
            Some(Status::GameRunning)
        );
        // The new one is what the next tick is compared with.
        assert!(changed(&mut last, snapshot(Status::GameRunning)).is_none());
        assert!(changed(&mut last, snapshot(Status::UpToDate)).is_some());
    }
}
