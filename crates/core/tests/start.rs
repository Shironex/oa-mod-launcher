//! Starting the game and what the launcher shows around a start. The game is
//! never started here: the configuration records the address the launcher
//! would hand to the system.

mod support;

use std::sync::atomic::Ordering;
use std::time::{Duration, UNIX_EPOCH};

use oa_mod_launcher_core::game::{self, paths};
use oa_mod_launcher_core::launcher::START_URL;
use oa_mod_launcher_core::{CoreError, Launcher, Progress, Status, Warning};
use support::{Release, Sandbox, TestServer, put};

fn quiet(_progress: Progress) {}

fn opened(sandbox: &Sandbox) -> Vec<String> {
    sandbox.opened.lock().expect("the opened list").clone()
}

fn set_modified(sandbox: &Sandbox, relative: &str, seconds: u64) {
    std::fs::File::options()
        .write(true)
        .open(sandbox.game.join(relative))
        .expect("the file opens")
        .set_modified(UNIX_EPOCH + Duration::from_secs(seconds))
        .expect("the time is set");
}

#[test]
fn starting_hands_the_steam_address_to_the_system_and_nothing_else() {
    let sandbox = Sandbox::new();
    let launcher = Launcher::open(sandbox.config_offline()).expect("the launcher opens");

    launcher.start_game().expect("the start is asked for");

    assert_eq!(START_URL, "steam://rungameid/431240");
    assert_eq!(opened(&sandbox), [START_URL]);
}

#[test]
fn a_game_that_runs_or_is_not_there_is_not_started() {
    let sandbox = Sandbox::new();
    let launcher = Launcher::open(sandbox.config_offline()).expect("the launcher opens");

    sandbox.running.store(true, Ordering::SeqCst);
    assert!(matches!(launcher.start_game(), Err(CoreError::GameRunning)));
    sandbox.running.store(false, Ordering::SeqCst);

    std::fs::remove_file(sandbox.game.join(game::EXE)).expect("remove the exe");
    assert!(launcher.start_game().is_err());

    assert!(opened(&sandbox).is_empty());
}

#[test]
fn a_start_the_system_refuses_is_an_error_with_its_reason() {
    let sandbox = Sandbox::new();
    let mut config = sandbox.config_offline();
    config.open = std::sync::Arc::new(|_| Err(std::io::Error::other("no handler for steam")));
    let launcher = Launcher::open(config).expect("the launcher opens");

    let error = launcher.start_game().expect_err("the start fails");

    assert!(
        matches!(&error, CoreError::Start(reason) if reason.contains("no handler")),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_first_start_is_announced_and_ends_when_bepinex_has_prepared_the_game() {
    let sandbox = Sandbox::new();
    let server = TestServer::start().await;
    server.publish(&Release::new("0.1.0"));
    let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");

    let installed = launcher.install(&quiet).await.expect("the install works");
    assert_eq!(installed.status, Status::FirstRunNeeded);

    // The player starts the game: it runs, for minutes.
    let started = launcher.start_game().expect("the start is asked for");
    assert_eq!(started.status, Status::FirstRunNeeded);
    sandbox.running.store(true, Ordering::SeqCst);
    assert_eq!(launcher.refresh().status, Status::GameRunning);

    // BepInEx writes its interop assemblies, the player closes the game.
    put(&sandbox.game, paths::INTEROP_READY, b"generated");
    put(&sandbox.game, paths::INTEROP_HASH, b"hash");
    sandbox.running.store(false, Ordering::SeqCst);

    let after = launcher.refresh();
    assert_eq!(after.status, Status::UpToDate);
    assert!(after.warnings.is_empty());
    assert_eq!(opened(&sandbox), [START_URL]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_game_update_warns_that_the_next_start_is_slow() {
    let sandbox = Sandbox::new();
    let server = TestServer::start().await;
    server.publish(&Release::new("0.1.0"));
    let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");
    launcher.install(&quiet).await.expect("the install works");
    put(&sandbox.game, paths::INTEROP_READY, b"generated");
    put(&sandbox.game, paths::INTEROP_HASH, b"hash");
    set_modified(&sandbox, game::ASSEMBLY, 1_700_000_000);
    set_modified(&sandbox, paths::INTEROP_HASH, 1_700_000_100);
    assert!(launcher.refresh().warnings.is_empty());

    // Steam replaces the game's code.
    set_modified(&sandbox, game::ASSEMBLY, 1_700_000_200);
    let updated = launcher.refresh();
    assert_eq!(updated.warnings, [Warning::SlowNextStart]);
    assert_eq!(
        updated.status,
        Status::UpToDate,
        "a warning, not another status"
    );

    // BepInEx prepares the game again on the next start.
    set_modified(&sandbox, paths::INTEROP_HASH, 1_700_000_300);
    assert!(launcher.refresh().warnings.is_empty());
}
