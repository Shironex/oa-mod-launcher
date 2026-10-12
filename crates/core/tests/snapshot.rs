//! The snapshot, read from a fake Steam and a fake game folder in a temporary
//! directory: which folder is the game, what is in it, and what the last
//! update check says about it.

mod support;

use oa_mod_launcher_core::game::paths;
use oa_mod_launcher_core::state::{ModRecord, State};
use oa_mod_launcher_core::{BepInEx, Launcher, Remote, Status, UNKNOWN_BUILD, Warning};
use support::{Release, Sandbox, TestServer, put, sha256, unpack_bepinex_by_hand};

#[test]
fn a_vanilla_game_is_found_and_nothing_is_installed() {
    let sandbox = Sandbox::new();
    let launcher = Launcher::open(sandbox.config_offline()).expect("the launcher opens");

    let snapshot = launcher.snapshot();

    assert_eq!(snapshot.status, Status::NotInstalled);
    assert_eq!(snapshot.launcher_version, "0.1.0");
    let game = snapshot.game.expect("the game is found");
    assert!(
        !game.dir.starts_with(r"\\?\"),
        "{} is shown plain",
        game.dir
    );
    assert!(game.dir.ends_with(support::INSTALL_DIR));
    assert_eq!(game.build_id.as_deref(), Some(support::BUILD_ID));
    assert!(!game.steam_busy);
    assert_eq!(snapshot.bepinex, BepInEx::Absent);
    assert_eq!(snapshot.mod_installed, None);
    assert_eq!(snapshot.remote, Remote::Unknown);
    assert!(snapshot.warnings.is_empty());
    assert!(snapshot.check_on_start);
    assert_eq!(launcher.game_dir(), Some(sandbox.game.clone()));
    assert_eq!(launcher.refresh(), launcher.snapshot());
}

#[test]
fn without_the_game_the_status_says_so() {
    let sandbox = Sandbox::new();
    std::fs::remove_file(sandbox.game.join(oa_mod_launcher_core::game::EXE)).expect("remove");
    let launcher = Launcher::open(sandbox.config_offline()).expect("the launcher opens");

    let snapshot = launcher.snapshot();

    assert_eq!(snapshot.status, Status::GameNotFound);
    assert_eq!(snapshot.game, None);
    assert_eq!(launcher.game_dir(), None);
}

#[test]
fn a_folder_chosen_by_hand_wins_while_it_is_the_game_and_is_kept_when_it_is_not() {
    let sandbox = Sandbox::new();
    let launcher = Launcher::open(sandbox.config_offline()).expect("the launcher opens");
    let other_library = sandbox.steam.parent().expect("the base").join("Elsewhere");
    let other = std::fs::canonicalize(support::fake_game_folder(&other_library)).expect("a game");

    // Not a game folder: refused, and nothing is stored.
    assert!(launcher.set_game_dir(&sandbox.steam).is_err());
    assert_eq!(sandbox.saved_state().game_dir_override, None);

    let chosen = launcher
        .set_game_dir(&other)
        .expect("the folder is the game");
    let game = chosen.game.expect("a game");
    assert!(game.dir.contains("Elsewhere"));
    assert_eq!(game.build_id, None, "Steam says nothing about this folder");
    assert_eq!(launcher.game_dir(), Some(other.clone()));

    // The folder stops being the game: discovery answers, the choice stays.
    std::fs::remove_file(other.join(oa_mod_launcher_core::game::EXE)).expect("remove");
    assert_eq!(launcher.game_dir(), Some(sandbox.game.clone()));
    assert!(sandbox.saved_state().game_dir_override.is_some());

    // It is the game again: the choice wins again.
    std::fs::write(other.join(oa_mod_launcher_core::game::EXE), b"MZ").expect("the exe");
    assert_eq!(launcher.game_dir(), Some(other));

    launcher.reset_game_dir().expect("the reset is stored");
    assert_eq!(sandbox.saved_state().game_dir_override, None);
    assert_eq!(launcher.game_dir(), Some(sandbox.game.clone()));
}

#[test]
fn choosing_the_folder_steam_names_keeps_what_steam_says_about_it() {
    let sandbox = Sandbox::new();
    let launcher = Launcher::open(sandbox.config_offline()).expect("the launcher opens");

    let snapshot = launcher
        .set_game_dir(&sandbox.game)
        .expect("the folder is the game");

    let game = snapshot.game.expect("a game");
    assert_eq!(game.build_id.as_deref(), Some(support::BUILD_ID));
}

#[test]
fn a_busy_steam_is_reported_and_never_blocks_reading() {
    let sandbox = Sandbox::new();
    support::write_app_manifest_in_state(&sandbox.steam, support::INSTALL_DIR, 1026);
    let launcher = Launcher::open(sandbox.config_offline()).expect("the launcher opens");

    let snapshot = launcher.snapshot();

    assert!(snapshot.game.expect("the game is still found").steam_busy);
    assert_eq!(snapshot.status, Status::NotInstalled);
}

#[test]
fn a_running_game_is_the_status_and_ends_by_itself() {
    let sandbox = Sandbox::new();
    let launcher = Launcher::open(sandbox.config_offline()).expect("the launcher opens");

    sandbox
        .running
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(launcher.refresh().status, Status::GameRunning);

    sandbox
        .running
        .store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(launcher.refresh().status, Status::NotInstalled);
}

#[test]
fn an_unreadable_state_file_is_an_error_until_it_is_readable_again() {
    let sandbox = Sandbox::new();
    let state_file = sandbox.data.join("state.json");
    std::fs::create_dir_all(&state_file).expect("a folder where the file belongs");
    let launcher = Launcher::open(sandbox.config_offline()).expect("the launcher still opens");

    assert!(matches!(launcher.snapshot().status, Status::Error { .. }));
    assert!(
        launcher.set_check_on_start(false).is_err(),
        "no setting is written over a state that could not be read"
    );

    std::fs::remove_dir(&state_file).expect("the cause is gone");
    assert_eq!(launcher.snapshot().status, Status::NotInstalled);
    let snapshot = launcher.set_check_on_start(false).expect("stored");
    assert!(!snapshot.check_on_start);
    assert!(!sandbox.saved_state().check_on_start);
}

#[test]
fn a_game_folder_with_a_link_where_bepinex_belongs_is_an_error_not_a_guess() {
    let sandbox = Sandbox::new();
    let elsewhere = sandbox.steam.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("another folder");
    support::link_dir(&elsewhere, &sandbox.game.join("BepInEx"));
    let launcher = Launcher::open(sandbox.config_offline()).expect("the launcher opens");

    assert!(matches!(launcher.snapshot().status, Status::Error { .. }));
}

#[test]
fn a_mod_the_launcher_did_not_install_is_an_unknown_build_and_an_update() {
    let sandbox = Sandbox::new();
    unpack_bepinex_by_hand(&sandbox.game);
    put(&sandbox.game, paths::MOD_DLL, b"MZ a local build");
    put(&sandbox.game, paths::INTEROP_READY, b"generated");
    put(&sandbox.game, "BepInEx/plugins/Other/Other.dll", b"MZ");
    let launcher = Launcher::open(sandbox.config_offline()).expect("the launcher opens");

    let snapshot = launcher.snapshot();

    assert_eq!(snapshot.mod_installed.as_deref(), Some(UNKNOWN_BUILD));
    assert_eq!(snapshot.status, Status::UpdateAvailable);
    assert_eq!(snapshot.foreign_plugins, ["Other"]);
    // Nothing is known to compare the loader with, so it is not ours.
    assert_eq!(snapshot.bepinex, BepInEx::Foreign);
    assert_eq!(snapshot.warnings, [Warning::ForeignBepInEx]);
}

#[test]
fn a_mod_the_state_records_shows_its_version_and_waits_for_its_first_run() {
    let sandbox = Sandbox::new();
    unpack_bepinex_by_hand(&sandbox.game);
    put(&sandbox.game, paths::MOD_DLL, b"MZ OaMod 0.1.0");
    State {
        mod_file: Some(ModRecord {
            version: "0.1.0".to_owned(),
            sha256: sha256(b"MZ OaMod 0.1.0"),
        }),
        ..State::default()
    }
    .save(&sandbox.data.join("state.json"))
    .expect("a state file");
    let launcher = Launcher::open(sandbox.config_offline()).expect("the launcher opens");

    let snapshot = launcher.snapshot();
    assert_eq!(snapshot.mod_installed.as_deref(), Some("0.1.0"));
    assert_eq!(snapshot.status, Status::FirstRunNeeded);

    // BepInEx has prepared the game: the first run is over.
    put(&sandbox.game, paths::INTEROP_READY, b"generated");
    assert_eq!(launcher.refresh().status, Status::UpToDate);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_check_says_what_is_waiting_and_how_large_it_is() {
    let sandbox = Sandbox::new();
    let server = TestServer::start().await;
    let release = Release::new("0.1.0");
    server.publish(&release);
    let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");

    let snapshot = launcher.check().await;

    assert_eq!(
        snapshot.remote,
        Remote::UpdateAvailable {
            version: "0.1.0".to_owned(),
            size: (release.mod_bytes.len() + release.zip.len()) as u64,
        }
    );
    assert_eq!(snapshot.notes.as_deref(), Some("About 0.1.0"));
    assert_eq!(snapshot.status, Status::NotInstalled);
    assert!(snapshot.warnings.is_empty());

    // With the pinned BepInEx already there, only the mod is counted, and
    // the loader is recognised by the manifest.
    unpack_bepinex_by_hand(&sandbox.game);
    let snapshot = launcher.refresh();
    assert_eq!(
        snapshot.remote,
        Remote::UpdateAvailable {
            version: "0.1.0".to_owned(),
            size: release.mod_bytes.len() as u64,
        }
    );
    assert_eq!(
        snapshot.bepinex,
        BepInEx::Adopted {
            label: "6.0.0-be.788".to_owned()
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_game_build_the_release_does_not_name_only_warns() {
    let sandbox = Sandbox::new();
    let server = TestServer::start().await;
    let mut release = Release::new("0.1.0");
    release.builds = vec!["19000000".to_owned()];
    server.publish(&release);
    let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");

    let snapshot = launcher.check().await;

    assert_eq!(
        snapshot.warnings,
        [Warning::GameBuildUntested {
            build: support::BUILD_ID.to_owned()
        }]
    );
    assert!(matches!(snapshot.remote, Remote::UpdateAvailable { .. }));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_manifest_of_the_last_start_still_recognises_the_loader_offline() {
    let sandbox = Sandbox::new();
    let server = TestServer::start().await;
    server.publish(&Release::new("0.1.0"));
    Launcher::open(sandbox.config(&server))
        .expect("the launcher opens")
        .check()
        .await;
    unpack_bepinex_by_hand(&sandbox.game);

    // A new start, without a network.
    let launcher = Launcher::open(sandbox.config_offline()).expect("the launcher opens");
    // The cache was written for files of the server, which this start does
    // not accept: it is ignored, and the loader counts as unknown.
    assert_eq!(launcher.snapshot().bepinex, BepInEx::Foreign);

    // The same start with the server's addresses, and the server gone.
    drop(server);
    let mut config = sandbox.config_offline();
    config.download_prefix = sandbox_prefix(&sandbox);
    let launcher = Launcher::open(config).expect("the launcher opens");
    let snapshot = launcher.check().await;

    assert!(matches!(snapshot.remote, Remote::Offline { .. }));
    assert_eq!(
        snapshot.bepinex,
        BepInEx::Adopted {
            label: "6.0.0-be.788".to_owned()
        }
    );
}

/// The download prefix the cached manifest of `sandbox` was written for.
fn sandbox_prefix(sandbox: &Sandbox) -> String {
    let text = std::fs::read_to_string(sandbox.data.join("cache").join("manifest.json"))
        .expect("the cached manifest");
    let manifest: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    let url = manifest["mod"]["url"].as_str().expect("an address");
    url[..url.rfind('/').expect("a path") + 1].to_owned()
}
