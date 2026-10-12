//! Removing from end to end, on a fake game folder in a temporary directory.
//!
//! Every test here deletes files, so every test here works in a [`Sandbox`]
//! only: its game folder, its Steam folder and its data root are all in one
//! temporary directory, and the launcher is never asked where the real Steam
//! is.

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use oa_mod_launcher_core::game::paths;
use oa_mod_launcher_core::state::State;
use oa_mod_launcher_core::{BepInEx, CoreError, ForeignKind, Launcher, Progress, Status};
use support::{Release, Sandbox, TestServer, link_dir, listing, put, unpack_bepinex_by_hand};

fn quiet(_progress: Progress) {}

const VANILLA: [&str; 2] = ["GameAssembly.dll", "Golf With Your Friends.exe"];

/// A sandbox with release 0.1.0 installed by the launcher itself, and the
/// first start of the game over.
async fn installed() -> (Sandbox, TestServer, Arc<Launcher>) {
    let sandbox = Sandbox::new();
    let server = TestServer::start().await;
    server.publish(&Release::new("0.1.0"));
    let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");
    launcher.install(&quiet).await.expect("the install works");
    first_start(&sandbox);
    (sandbox, server, launcher)
}

/// What BepInEx leaves in the game folder after the first start.
fn first_start(sandbox: &Sandbox) {
    put(&sandbox.game, paths::INTEROP_READY, b"generated");
    put(&sandbox.game, paths::INTEROP_HASH, b"hash");
    put(&sandbox.game, "BepInEx/config/BepInEx.cfg", b"[Logging]");
    put(&sandbox.game, paths::MOD_CONFIG, b"speed = 2");
    put(&sandbox.game, "BepInEx/LogOutput.log", b"loaded");
    put(&sandbox.game, "BepInEx/cache/chainloader.dat", b"cache");
}

/// Plugins, a patcher and settings that are somebody else's.
fn other_peoples_things(sandbox: &Sandbox) {
    put(
        &sandbox.game,
        "BepInEx/plugins/Other/Other.dll",
        b"MZ other",
    );
    put(
        &sandbox.game,
        "BepInEx/plugins/Other/data/deep.bin",
        b"deep data",
    );
    put(&sandbox.game, "BepInEx/plugins/loose.dll", b"MZ loose");
    put(&sandbox.game, "BepInEx/patchers/Patch.dll", b"MZ patch");
    put(
        &sandbox.game,
        "BepInEx/config/other.plugin.cfg",
        b"theirs = 1",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_preview_of_our_own_install_lists_what_would_go() {
    let (sandbox, _server, launcher) = installed().await;
    let before = listing(&sandbox.game);

    let plan = launcher.removal_preview().expect("a plan");

    assert!(!plan.game_dir.starts_with(r"\\?\"));
    assert!(matches!(plan.bepinex, BepInEx::Ours { .. }));
    assert_eq!(plan.mod_files, [paths::MOD_DLL, paths::MOD_CONFIG]);
    assert_eq!(plan.folders, ["dotnet", "BepInEx"]);
    assert_eq!(
        plan.root_files,
        [
            "winhttp.dll",
            "doorstop_config.ini",
            ".doorstop_version",
            "changelog.txt"
        ]
    );
    assert!(plan.unrecognised_root_files.is_empty());
    assert!(plan.kept.is_empty());
    assert!(plan.foreign.is_empty());
    assert!(!plan.mod_only_recommended);
    assert_eq!(listing(&sandbox.game), before, "a preview changes nothing");
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_only_the_mod_leaves_bepinex_and_every_other_plugin() {
    let (sandbox, _server, launcher) = installed().await;
    other_peoples_things(&sandbox);
    put(&sandbox.game, paths::MOD_PDB, b"symbols");
    let mut expected = listing(&sandbox.game);
    expected.retain(|file| {
        ![paths::MOD_DLL, paths::MOD_PDB, paths::MOD_CONFIG].contains(&file.as_str())
    });

    let snapshot = launcher.remove_mod().expect("the mod is removed");

    assert_eq!(listing(&sandbox.game), expected);
    assert!(!sandbox.game.join(paths::MOD_DIR).exists());
    assert_eq!(snapshot.status, Status::NotInstalled);
    assert_eq!(snapshot.mod_installed, None);
    assert!(matches!(snapshot.bepinex, BepInEx::Ours { .. }));
    let state = sandbox.saved_state();
    assert_eq!(state.mod_file, None);
    assert!(state.bepinex.is_some(), "BepInEx stays on record");
    assert!(!state.opted_out);

    // Again is fine, and changes nothing.
    launcher.remove_mod().expect("a second run is fine");
    assert_eq!(listing(&sandbox.game), expected);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_next_to_the_mod_keeps_the_mod_folder() {
    let (sandbox, _server, launcher) = installed().await;
    put(&sandbox.game, "BepInEx/plugins/OaMod/notes.txt", b"mine");

    launcher.remove_mod().expect("the mod is removed");

    assert!(!sandbox.game.join(paths::MOD_DLL).exists());
    assert_eq!(
        std::fs::read(sandbox.game.join("BepInEx/plugins/OaMod/notes.txt")).expect("kept"),
        b"mine"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_everything_leaves_a_vanilla_game_and_is_remembered() {
    let (sandbox, _server, launcher) = installed().await;

    let outcome = launcher.remove_all(false).expect("everything is removed");

    assert_eq!(listing(&sandbox.game), VANILLA);
    assert!(!sandbox.game.join("BepInEx").exists());
    assert!(!sandbox.game.join("dotnet").exists());
    assert!(outcome.kept.is_empty());
    assert_eq!(outcome.backup_dir, None);
    assert!(!sandbox.data.join("backups").exists());
    assert_eq!(outcome.snapshot.status, Status::Clean);
    assert_eq!(outcome.snapshot.bepinex, BepInEx::Absent);
    let state = sandbox.saved_state();
    assert!(state.opted_out);
    assert_eq!((state.mod_file, state.bepinex), (None, None));

    // Again is fine.
    let again = launcher.remove_all(false).expect("a second run is fine");
    assert_eq!(again.snapshot.status, Status::Clean);
    assert_eq!(listing(&sandbox.game), VANILLA);

    // And installing again ends the opt-out.
    let reinstalled = launcher.install(&quiet).await.expect("the install works");
    assert_eq!(reinstalled.status, Status::FirstRunNeeded);
    assert!(!sandbox.saved_state().opted_out);
}

#[tokio::test(flavor = "multi_thread")]
async fn other_peoples_files_are_named_first_and_backed_up_before_they_go() {
    let (sandbox, _server, launcher) = installed().await;
    other_peoples_things(&sandbox);

    let plan = launcher.removal_preview().expect("a plan");
    assert!(plan.mod_only_recommended);
    let named: Vec<(&str, ForeignKind, u64)> = plan
        .foreign
        .iter()
        .map(|item| (item.path.as_str(), item.kind, item.bytes))
        .collect();
    assert_eq!(
        named,
        [
            ("BepInEx/plugins/Other", ForeignKind::Plugin, 17),
            ("BepInEx/plugins/loose.dll", ForeignKind::Plugin, 8),
            ("BepInEx/patchers/Patch.dll", ForeignKind::Patcher, 8),
            ("BepInEx/config/other.plugin.cfg", ForeignKind::Config, 10),
        ]
    );

    let outcome = launcher.remove_all(false).expect("everything is removed");

    assert_eq!(listing(&sandbox.game), VANILLA);
    let backup = std::path::PathBuf::from(outcome.backup_dir.expect("a backup was made"));
    assert!(
        std::fs::canonicalize(&backup)
            .expect("the backup exists")
            .starts_with(sandbox.data.join("backups")),
        "the backup is in the launcher's own folder"
    );
    assert_eq!(
        listing(&backup),
        [
            "BepInEx/config/other.plugin.cfg",
            "BepInEx/patchers/Patch.dll",
            "BepInEx/plugins/Other/Other.dll",
            "BepInEx/plugins/Other/data/deep.bin",
            "BepInEx/plugins/loose.dll",
        ],
        "the mod's and BepInEx's own files are not backed up"
    );
    assert_eq!(
        std::fs::read(backup.join("BepInEx/plugins/Other/data/deep.bin")).expect("copied"),
        b"deep data"
    );
    assert_eq!(outcome.snapshot.status, Status::Clean);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_adopted_bepinex_makes_mod_only_the_default() {
    let sandbox = Sandbox::new();
    unpack_bepinex_by_hand(&sandbox.game);
    let server = TestServer::start().await;
    server.publish(&Release::new("0.1.0"));
    let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");
    launcher.install(&quiet).await.expect("the install works");

    let plan = launcher.removal_preview().expect("a plan");

    assert!(matches!(plan.bepinex, BepInEx::Adopted { .. }));
    assert!(plan.foreign.is_empty());
    assert!(plan.mod_only_recommended);
    // Recognised by the checksums of the release, so a full removal that was
    // asked for twice would take them.
    assert_eq!(plan.root_files.len(), 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn root_files_the_launcher_does_not_recognise_are_kept_unless_asked() {
    let (sandbox, _server, launcher) = installed().await;
    put(
        &sandbox.game,
        "winhttp.dll",
        b"MZ a proxy of something else",
    );
    put(&sandbox.game, "changelog.txt", b"edited by hand");

    let plan = launcher.removal_preview().expect("a plan");
    assert_eq!(
        plan.root_files,
        ["doorstop_config.ini", ".doorstop_version"]
    );
    assert_eq!(
        plan.unrecognised_root_files,
        ["winhttp.dll", "changelog.txt"]
    );

    let outcome = launcher.remove_all(false).expect("the rest is removed");

    assert_eq!(outcome.kept, ["winhttp.dll", "changelog.txt"]);
    let mut expected = VANILLA.map(str::to_owned).to_vec();
    expected.extend(["changelog.txt".to_owned(), "winhttp.dll".to_owned()]);
    expected.sort();
    assert_eq!(listing(&sandbox.game), expected);
    assert_eq!(
        std::fs::read(sandbox.game.join("winhttp.dll")).expect("kept"),
        b"MZ a proxy of something else"
    );
    // Something is still there, so this is not the clean state.
    assert!(matches!(
        outcome.snapshot.bepinex,
        BepInEx::Unsupported { .. }
    ));
    assert!(!sandbox.saved_state().opted_out);

    let outcome = launcher.remove_all(true).expect("now they go too");
    assert!(outcome.kept.is_empty());
    assert_eq!(listing(&sandbox.game), VANILLA);
    assert_eq!(outcome.snapshot.status, Status::Clean);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dotnet_folder_without_the_runtime_is_not_ours_to_delete() {
    let (sandbox, _server, launcher) = installed().await;
    std::fs::remove_file(sandbox.game.join(paths::DOTNET_PROOF)).expect("remove the proof");

    let outcome = launcher.remove_all(true).expect("the rest is removed");

    assert_eq!(outcome.kept, ["dotnet"]);
    assert!(
        sandbox
            .game
            .join("dotnet/shared/System.Runtime.dll")
            .is_file()
    );
    assert!(!sandbox.game.join("BepInEx").exists());
    assert!(!sandbox.game.join("winhttp.dll").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_anywhere_in_what_would_be_deleted_refuses_the_whole_removal() {
    for place in [
        "BepInEx/plugins/Linked",
        "BepInEx/core/deep/Linked",
        "dotnet/shared/Linked",
    ] {
        let (sandbox, _server, launcher) = installed().await;
        // The target is outside the game folder and holds a file that must
        // survive whatever the launcher does.
        let outside = sandbox.steam.join("outside");
        put(&outside, "precious.txt", b"keep me");
        std::fs::create_dir_all(sandbox.game.join(place).parent().expect("a parent"))
            .expect("folders");
        link_dir(&outside, &sandbox.game.join(place));
        let before = listing(&sandbox.game);

        for attempt in [launcher.remove_all(false), launcher.remove_all(true)] {
            let error = attempt.expect_err("the removal is refused");
            assert!(
                matches!(error, CoreError::UnsafePath { .. }),
                "{place}: {error}"
            );
        }
        assert!(matches!(
            launcher.removal_preview(),
            Err(CoreError::UnsafePath { .. })
        ));

        assert_eq!(
            listing(&sandbox.game),
            before,
            "{place}: nothing was deleted"
        );
        assert!(sandbox.game.join("winhttp.dll").is_file(), "{place}");
        assert_eq!(
            std::fs::read(outside.join("precious.txt")).expect("the target survives"),
            b"keep me",
            "{place}"
        );
        assert!(!sandbox.data.join("backups").exists(), "{place}");
        assert!(sandbox.saved_state().bepinex.is_some(), "{place}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_where_a_named_file_or_folder_belongs_is_refused_and_its_target_survives() {
    // The mod's own folder is a link: neither removal deletes through it.
    let (sandbox, _server, launcher) = installed().await;
    let outside = sandbox.steam.join("outside");
    put(
        &outside,
        "OaMod.dll",
        b"not the mod, and not in the game folder",
    );
    std::fs::remove_file(sandbox.game.join(paths::MOD_DLL)).expect("remove the mod");
    std::fs::remove_dir(sandbox.game.join(paths::MOD_DIR)).expect("remove its folder");
    link_dir(&outside, &sandbox.game.join(paths::MOD_DIR));

    assert!(matches!(
        launcher.remove_mod(),
        Err(CoreError::UnsafePath { .. })
    ));
    assert!(matches!(
        launcher.remove_all(true),
        Err(CoreError::UnsafePath { .. })
    ));
    assert_eq!(
        std::fs::read(outside.join("OaMod.dll")).expect("the target survives"),
        b"not the mod, and not in the game folder"
    );
    assert!(sandbox.game.join("winhttp.dll").is_file());

    // `dotnet` and a root file are links.
    for name in ["dotnet", "winhttp.dll"] {
        let (sandbox, _server, launcher) = installed().await;
        let outside = sandbox.steam.join("outside");
        put(
            &outside,
            "coreclr.dll",
            b"a runtime that is not in the game folder",
        );
        let named = sandbox.game.join(name);
        if named.is_dir() {
            std::fs::remove_dir_all(&named).expect("remove the folder");
        } else {
            std::fs::remove_file(&named).expect("remove the file");
        }
        link_dir(&outside, &named);

        assert!(
            matches!(launcher.remove_all(true), Err(CoreError::UnsafePath { .. })),
            "{name}"
        );
        assert_eq!(
            std::fs::read(outside.join("coreclr.dll")).expect("the target survives"),
            b"a runtime that is not in the game folder",
            "{name}"
        );
        assert!(sandbox.game.join("BepInEx/core").is_dir(), "{name}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_is_removed_while_the_game_runs_or_steam_is_busy() {
    let (sandbox, _server, launcher) = installed().await;
    other_peoples_things(&sandbox);
    let before = listing(&sandbox.game);

    sandbox.running.store(true, Ordering::SeqCst);
    assert!(matches!(launcher.remove_mod(), Err(CoreError::GameRunning)));
    assert!(matches!(
        launcher.remove_all(true),
        Err(CoreError::GameRunning)
    ));
    sandbox.running.store(false, Ordering::SeqCst);

    support::write_app_manifest_in_state(&sandbox.steam, support::INSTALL_DIR, 1026);
    assert!(matches!(launcher.remove_mod(), Err(CoreError::SteamBusy)));
    assert!(matches!(
        launcher.remove_all(true),
        Err(CoreError::SteamBusy)
    ));
    // Reading is never blocked.
    assert!(launcher.removal_preview().is_ok());

    assert_eq!(listing(&sandbox.game), before);
    assert!(!sandbox.data.join("backups").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_game_started_after_the_backup_gets_nothing_deleted() {
    let sandbox = Sandbox::new();
    let server = TestServer::start().await;
    server.publish(&Release::new("0.1.0"));
    // Once armed, the game is not running the first time the launcher asks
    // and is running from the second time on: the question that is asked
    // right before the first delete.
    let armed = Arc::new(AtomicBool::new(false));
    let asked = Arc::new(AtomicUsize::new(0));
    let mut config = sandbox.config(&server);
    let (switch, counter) = (Arc::clone(&armed), Arc::clone(&asked));
    config.running = Arc::new(move |_| {
        switch.load(Ordering::SeqCst) && counter.fetch_add(1, Ordering::SeqCst) >= 1
    });
    let launcher = Launcher::open(config).expect("the launcher opens");
    launcher.install(&quiet).await.expect("the install works");
    other_peoples_things(&sandbox);
    let before = listing(&sandbox.game);

    armed.store(true, Ordering::SeqCst);
    let error = launcher.remove_all(true).expect_err("the game runs by now");
    armed.store(false, Ordering::SeqCst);

    assert!(matches!(error, CoreError::GameRunning), "{error}");
    assert_eq!(
        asked.load(Ordering::SeqCst),
        2,
        "asked at the start and again before deleting"
    );
    assert_eq!(listing(&sandbox.game), before, "nothing was deleted");
    assert!(
        sandbox.data.join("backups").is_dir(),
        "the backup was made before the question, and it is only a copy"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_removal_that_was_interrupted_is_finished_by_running_it_again() {
    let (sandbox, _server, launcher) = installed().await;
    // What an interrupted run leaves: the root files and dotnet are gone,
    // BepInEx is half deleted.
    for gone in ["winhttp.dll", "doorstop_config.ini", ".doorstop_version"] {
        std::fs::remove_file(sandbox.game.join(gone)).expect("remove");
    }
    std::fs::remove_dir_all(sandbox.game.join("dotnet")).expect("remove");
    std::fs::remove_dir_all(sandbox.game.join("BepInEx/core")).expect("remove");
    assert!(sandbox.game.join("BepInEx/plugins").is_dir());

    let outcome = launcher.remove_all(false).expect("the removal is finished");

    assert_eq!(listing(&sandbox.game), VANILLA);
    assert!(!sandbox.game.join("BepInEx").exists());
    assert_eq!(outcome.snapshot.status, Status::Clean);
    assert!(sandbox.saved_state().opted_out);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_changed_state_file_cannot_point_a_removal_at_another_folder() {
    let (sandbox, server, launcher) = installed().await;
    drop(launcher);
    // A folder that looks like an install of BepInEx and is not the game.
    let documents = sandbox.steam.parent().expect("the base").join("Documents");
    unpack_bepinex_by_hand(&documents);
    put(&documents, "BepInEx/plugins/OaMod/OaMod.dll", b"MZ");
    put(&documents, "thesis.txt", b"years of work");
    let before = listing(&documents);
    let drive = documents
        .ancestors()
        .last()
        .expect("every path has a root")
        .to_path_buf();

    for chosen in [documents.clone(), drive, documents.join("..").join("..")] {
        let mut state = sandbox.saved_state();
        state.game_dir_override = Some(chosen);
        state
            .save(&sandbox.data.join("state.json"))
            .expect("the state is changed by hand");
        let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");

        // The folder is not the game, so it is never the root of anything:
        // the removal works on the game Steam names.
        assert_eq!(launcher.game_dir(), Some(sandbox.game.clone()));
        launcher.remove_all(true).expect("the game is cleaned");

        assert_eq!(listing(&documents), before);
        assert_eq!(listing(&sandbox.game), VANILLA);
        // Put the install back for the next round.
        launcher.install(&quiet).await.expect("the install works");
    }
}

#[test]
fn without_a_game_folder_nothing_is_removed() {
    let sandbox = Sandbox::new();
    std::fs::remove_file(sandbox.game.join(oa_mod_launcher_core::game::EXE)).expect("remove");
    unpack_bepinex_by_hand(&sandbox.game);
    let before = listing(&sandbox.game);
    let launcher = Launcher::open(sandbox.config_offline()).expect("the launcher opens");

    assert!(launcher.remove_mod().is_err());
    assert!(launcher.remove_all(true).is_err());
    assert!(launcher.removal_preview().is_err());
    assert_eq!(listing(&sandbox.game), before);
    assert_eq!(sandbox.saved_state(), State::default());
}
