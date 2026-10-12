//! Installing and updating from end to end: a loopback server publishes a
//! signed release, and a fake game folder in a temporary directory receives
//! it. Nothing here touches a real Steam, a real game or the network.

mod support;

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::Ordering;

use oa_mod_launcher_core::game::paths;
use oa_mod_launcher_core::state::Owner;
use oa_mod_launcher_core::{
    BepInEx, CoreError, Launcher, Phase, Progress, Remote, Snapshot, Status, UNKNOWN_BUILD, Warning,
};
use support::{
    BEPINEX_FILES, Release, Reply, Sandbox, Signer, TestServer, bepinex_file, link_dir, listing,
    put, sha256, unpack_bepinex_by_hand, zip_of,
};

fn quiet(_progress: Progress) {}

/// The two files that make the fake folder the game, which is all a vanilla
/// game folder lists.
const VANILLA: [&str; 2] = ["GameAssembly.dll", "Golf With Your Friends.exe"];

async fn install(launcher: &Launcher) -> Result<Snapshot, CoreError> {
    launcher.install(&quiet).await
}

fn file_requests(server: &TestServer) -> Vec<String> {
    server
        .requests()
        .into_iter()
        .filter(|path| path.starts_with("/files/"))
        .collect()
}

fn read(sandbox: &Sandbox, relative: &str) -> Vec<u8> {
    std::fs::read(sandbox.game.join(relative)).expect("the file is in the game folder")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fresh_install_puts_bepinex_and_the_mod_into_the_game_folder() {
    let sandbox = Sandbox::new();
    let server = TestServer::start().await;
    let release = Release::new("0.1.0");
    server.publish(&release);
    let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");

    let phases = Mutex::new(Vec::new());
    let installed = launcher
        .install(&|progress: Progress| {
            assert!(progress.received <= progress.total || progress.total == 0);
            let mut phases = phases.lock().expect("the phase list");
            if phases.last() != Some(&progress.phase) {
                phases.push(progress.phase);
            }
        })
        .await
        .expect("the install works");

    assert_eq!(
        phases.into_inner().expect("the phase list"),
        [
            Phase::Checking,
            Phase::DownloadingBepInEx,
            Phase::InstallingBepInEx,
            Phase::DownloadingMod,
            Phase::InstallingMod,
        ]
    );

    // Exactly the files of the zip and the mod, next to the game's own.
    let mut expected: Vec<String> = BEPINEX_FILES
        .iter()
        .map(|file| file.0.to_owned())
        .chain(VANILLA.map(str::to_owned))
        .chain([paths::MOD_DLL.to_owned()])
        .collect();
    expected.sort();
    assert_eq!(listing(&sandbox.game), expected);
    for (name, body) in BEPINEX_FILES {
        assert_eq!(read(&sandbox, name), body, "{name}");
    }
    assert_eq!(read(&sandbox, paths::MOD_DLL), release.mod_bytes);

    assert_eq!(installed.status, Status::FirstRunNeeded);
    assert_eq!(
        installed.bepinex,
        BepInEx::Ours {
            label: "6.0.0-be.788".to_owned()
        }
    );
    assert_eq!(installed.mod_installed.as_deref(), Some("0.1.0"));
    assert_eq!(installed.remote, Remote::UpToDate);
    assert!(installed.warnings.is_empty());

    let state = sandbox.saved_state();
    let bepinex = state.bepinex.expect("BepInEx is on record");
    assert_eq!(bepinex.owner, Owner::Launcher);
    assert_eq!(
        bepinex.root_files["winhttp.dll"],
        sha256(bepinex_file("winhttp.dll"))
    );
    let recorded = state.mod_file.expect("the mod is on record");
    assert_eq!(recorded.version, "0.1.0");
    assert_eq!(recorded.sha256, sha256(&release.mod_bytes));
    assert!(!state.opted_out);

    assert!(listing(&sandbox.data.join("staging")).is_empty());

    // BepInEx prepares the game on the first start, and the status follows.
    put(&sandbox.game, paths::INTEROP_READY, b"generated");
    assert_eq!(launcher.refresh().status, Status::UpToDate);
}

/// The real zip names three folders without content: `BepInEx/`, and the
/// empty `BepInEx/plugins/` and `BepInEx/patchers/`. They are no file outside
/// `BepInEx/core`, so they stop nothing, and none of them is carried over.
#[tokio::test(flavor = "multi_thread")]
async fn the_empty_folder_entries_of_the_real_zip_do_not_stop_an_install() {
    let sandbox = Sandbox::new();
    let server = TestServer::start().await;
    let mut release = Release::new("0.1.0");
    let mut entries: Vec<(&str, &[u8])> = vec![
        ("BepInEx/", b""),
        ("BepInEx/plugins/", b""),
        ("BepInEx/patchers/", b""),
    ];
    entries.extend(BEPINEX_FILES);
    release.zip = zip_of(&entries);
    server.publish(&release);
    let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");

    let installed = install(&launcher).await.expect("the install works");

    assert_eq!(installed.status, Status::FirstRunNeeded);
    let mut expected: Vec<String> = BEPINEX_FILES
        .iter()
        .map(|file| file.0.to_owned())
        .chain(VANILLA.map(str::to_owned))
        .chain([paths::MOD_DLL.to_owned()])
        .collect();
    expected.sort();
    assert_eq!(listing(&sandbox.game), expected);
    assert!(!sandbox.game.join(paths::PATCHERS).exists());
    assert!(listing(&sandbox.data.join("staging")).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_update_replaces_the_mod_and_leaves_bepinex_alone() {
    let sandbox = Sandbox::new();
    let server = TestServer::start().await;
    server.publish(&Release::new("0.1.0"));
    let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");
    install(&launcher).await.expect("the install works");
    put(&sandbox.game, paths::INTEROP_READY, b"generated");
    put(&sandbox.game, paths::MOD_CONFIG, b"speed = 2");

    let newer = Release::new("0.1.1");
    server.publish(&newer);
    let checked = launcher.check().await;
    assert_eq!(checked.status, Status::UpdateAvailable);
    assert_eq!(
        checked.remote,
        Remote::UpdateAvailable {
            version: "0.1.1".to_owned(),
            size: newer.mod_bytes.len() as u64,
        }
    );
    assert_eq!(checked.mod_installed.as_deref(), Some("0.1.0"));

    let updated = install(&launcher).await.expect("the update works");

    assert_eq!(updated.status, Status::UpToDate);
    assert_eq!(updated.mod_installed.as_deref(), Some("0.1.1"));
    assert_eq!(read(&sandbox, paths::MOD_DLL), newer.mod_bytes);
    assert_eq!(read(&sandbox, paths::MOD_CONFIG), b"speed = 2");
    assert_eq!(
        file_requests(&server),
        [
            "/files/BepInEx.zip",
            "/files/OaMod-0.1.0.dll",
            "/files/OaMod-0.1.1.dll"
        ],
        "the zip is fetched once, for the first install"
    );

    // Nothing new: the manifest is read, and no file is fetched or written.
    let again = install(&launcher).await.expect("nothing to do is fine");
    assert_eq!(again.status, Status::UpToDate);
    assert_eq!(file_requests(&server).len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_local_build_is_replaced_only_by_an_update() {
    let sandbox = Sandbox::new();
    let server = TestServer::start().await;
    let release = Release::new("0.1.0");
    server.publish(&release);
    let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");
    install(&launcher).await.expect("the install works");
    put(&sandbox.game, paths::INTEROP_READY, b"generated");
    put(&sandbox.game, paths::MOD_DLL, b"MZ built on this machine");

    let snapshot = launcher.check().await;
    assert_eq!(snapshot.mod_installed.as_deref(), Some(UNKNOWN_BUILD));
    assert_eq!(snapshot.status, Status::UpdateAvailable);
    assert_eq!(read(&sandbox, paths::MOD_DLL), b"MZ built on this machine");

    let updated = install(&launcher).await.expect("the update works");
    assert_eq!(updated.mod_installed.as_deref(), Some("0.1.0"));
    assert_eq!(read(&sandbox, paths::MOD_DLL), release.mod_bytes);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_pinned_bepinex_that_is_already_there_is_adopted_not_replaced() {
    let sandbox = Sandbox::new();
    unpack_bepinex_by_hand(&sandbox.game);
    put(
        &sandbox.game,
        "BepInEx/plugins/Other/Other.dll",
        b"MZ other",
    );
    let before = listing(&sandbox.game);
    let server = TestServer::start().await;
    server.publish(&Release::new("0.1.0"));
    let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");

    let installed = install(&launcher).await.expect("the install works");

    assert_eq!(
        installed.bepinex,
        BepInEx::Adopted {
            label: "6.0.0-be.788".to_owned()
        }
    );
    assert_eq!(installed.foreign_plugins, ["Other"]);
    assert_eq!(
        sandbox.saved_state().bepinex.expect("on record").owner,
        Owner::Adopted
    );
    assert_eq!(file_requests(&server), ["/files/OaMod-0.1.0.dll"]);
    let mut expected = before;
    expected.push(paths::MOD_DLL.to_owned());
    expected.sort();
    assert_eq!(listing(&sandbox.game), expected, "only the mod was added");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_foreign_bepinex_gets_the_mod_and_a_warning_and_is_never_claimed() {
    let sandbox = Sandbox::new();
    unpack_bepinex_by_hand(&sandbox.game);
    put(&sandbox.game, paths::PROBE, b"MZ il2cpp of another build");
    let server = TestServer::start().await;
    let release = Release::new("0.1.0");
    server.publish(&release);
    let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");

    let installed = install(&launcher).await.expect("the install works");

    assert_eq!(installed.bepinex, BepInEx::Foreign);
    assert_eq!(installed.warnings, [Warning::ForeignBepInEx]);
    assert_eq!(installed.mod_installed.as_deref(), Some("0.1.0"));
    assert_eq!(read(&sandbox, paths::MOD_DLL), release.mod_bytes);
    assert_eq!(read(&sandbox, paths::PROBE), b"MZ il2cpp of another build");
    assert_eq!(sandbox.saved_state().bepinex, None);
    assert_eq!(file_requests(&server), ["/files/OaMod-0.1.0.dll"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_loader_the_mod_cannot_use_stops_the_install_with_its_name() {
    let sandbox = Sandbox::new();
    put(&sandbox.game, "BepInEx/core/BepInEx.dll", b"MZ five");
    put(&sandbox.game, paths::WINHTTP, b"MZ proxy of five");
    let before = listing(&sandbox.game);
    let server = TestServer::start().await;
    server.publish(&Release::new("0.1.0"));
    let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");

    let error = install(&launcher).await.expect_err("the install stops");

    assert!(
        matches!(&error, CoreError::UnsupportedBepInEx(reason) if reason.contains("BepInEx.Unity.IL2CPP.dll")),
        "{error}"
    );
    assert_eq!(listing(&sandbox.game), before);
    assert!(file_requests(&server).is_empty());
    assert_eq!(launcher.snapshot().status, Status::NotInstalled);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_in_the_way_stops_the_install_before_anything_is_written() {
    let sandbox = Sandbox::new();
    // Not a sign of BepInEx by itself, but a name the zip would write to.
    put(&sandbox.game, "changelog.txt", b"the game's own notes");
    let server = TestServer::start().await;
    server.publish(&Release::new("0.1.0"));
    let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");

    let error = install(&launcher).await.expect_err("the install stops");

    assert!(
        matches!(&error, CoreError::Conflict { path } if path.ends_with("changelog.txt")),
        "{error}"
    );
    let mut expected = VANILLA.map(str::to_owned).to_vec();
    expected.insert(0, "changelog.txt".to_owned());
    expected.sort();
    assert_eq!(listing(&sandbox.game), expected);
    assert_eq!(read(&sandbox, "changelog.txt"), b"the game's own notes");
    assert_eq!(sandbox.saved_state().bepinex, None);
    assert!(listing(&sandbox.data.join("staging")).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hostile_or_wrong_zip_is_refused_whole() {
    let mut escaping = BEPINEX_FILES.to_vec();
    escaping.push(("../../escaped.txt", b"out"));
    let mut extra = BEPINEX_FILES.to_vec();
    extra.push(("version.dll", b"MZ another proxy"));
    let mut swapped = BEPINEX_FILES.to_vec();
    swapped.retain(|file| file.0 != "winhttp.dll");
    swapped.push(("winhttp.dll", b"MZ not the proxy the release names"));
    let mut incomplete = BEPINEX_FILES.to_vec();
    incomplete.retain(|file| file.0 != ".doorstop_version");

    for (what, entries) in [
        ("an entry that leaves the folder", escaping),
        ("a file that is not part of BepInEx", extra),
        ("a root file with other content", swapped),
        ("a missing root file", incomplete),
    ] {
        let sandbox = Sandbox::new();
        let server = TestServer::start().await;
        let mut release = Release::new("0.1.0");
        // The manifest names the size and the checksum of this very zip, so
        // only its content can stop it.
        release.zip = zip_of(&entries);
        server.publish(&release);
        let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");

        let error = install(&launcher).await.expect_err("the zip is refused");

        assert!(matches!(error, CoreError::Archive(_)), "{what}: {error}");
        assert_eq!(listing(&sandbox.game), VANILLA, "{what}");
        assert!(!sandbox.data.join("escaped.txt").exists(), "{what}");
        assert!(
            !sandbox
                .data
                .parent()
                .expect("the base")
                .join("escaped.txt")
                .exists(),
            "{what}"
        );
        assert!(listing(&sandbox.data.join("staging")).is_empty(), "{what}");
        assert_eq!(sandbox.saved_state().bepinex, None, "{what}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_download_that_does_not_match_the_manifest_is_not_installed() {
    let sandbox = Sandbox::new();
    let server = TestServer::start().await;
    let release = Release::new("0.1.0");
    server.publish(&release);
    let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");

    // The zip with one byte changed, the same length.
    let mut changed = release.zip.clone();
    let last = changed.len() - 1;
    changed[last] ^= 1;
    server.route("/files/BepInEx.zip", Reply::Body(changed));
    let error = install(&launcher).await.expect_err("the zip is refused");
    assert!(matches!(error, CoreError::HashMismatch { .. }), "{error}");
    assert_eq!(listing(&sandbox.game), VANILLA);

    // A zip that ends early.
    server.route(
        "/files/BepInEx.zip",
        Reply::Body(release.zip[..release.zip.len() / 2].to_vec()),
    );
    let error = install(&launcher).await.expect_err("the zip is refused");
    assert!(
        matches!(
            error,
            CoreError::SizeMismatch { .. } | CoreError::Http { .. }
        ),
        "{error}"
    );
    assert_eq!(listing(&sandbox.game), VANILLA);

    // The right zip, and a mod with other bytes of the same length.
    server.route("/files/BepInEx.zip", Reply::Body(release.zip.clone()));
    let mut other = release.mod_bytes.clone();
    other[3] ^= 1;
    server.route("/files/OaMod-0.1.0.dll", Reply::Body(other));
    let error = install(&launcher).await.expect_err("the mod is refused");
    assert!(matches!(error, CoreError::HashMismatch { .. }), "{error}");

    // BepInEx was installed completely and stays, on record as ours. The mod
    // is not there, and the next install only fetches the mod.
    assert!(!sandbox.game.join(paths::MOD_DLL).exists());
    assert!(!sandbox.game.join(paths::MOD_DLL_NEW).exists());
    let snapshot = launcher.snapshot();
    assert_eq!(snapshot.status, Status::NotInstalled);
    assert!(matches!(snapshot.bepinex, BepInEx::Ours { .. }));
    assert_eq!(sandbox.saved_state().mod_file, None);
    assert!(listing(&sandbox.data.join("staging")).is_empty());

    server.route(
        "/files/OaMod-0.1.0.dll",
        Reply::Body(release.mod_bytes.clone()),
    );
    let installed = install(&launcher).await.expect("the install works");
    assert_eq!(installed.status, Status::FirstRunNeeded);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_manifest_that_is_unsigned_or_wrongly_signed_installs_nothing() {
    let sandbox = Sandbox::new();
    let server = TestServer::start().await;
    let manifest = server.publish(&Release::new("0.1.0"));
    let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");
    let stranger = Signer::new();
    let mut tampered = manifest.clone();
    let version = String::from_utf8_lossy(&tampered)
        .find("0.1.0")
        .expect("the version");
    tampered[version + 4] = b'9';

    let cases: [(&str, Reply, Reply); 4] = [
        (
            "no signature file",
            Reply::Body(manifest.clone()),
            Reply::Status(404),
        ),
        (
            "a signature by another key",
            Reply::Body(manifest.clone()),
            Reply::Body(stranger.sign(&manifest).into_bytes()),
        ),
        (
            "a signature that is not one",
            Reply::Body(manifest.clone()),
            Reply::Body(b"not a signature".to_vec()),
        ),
        (
            "a manifest changed after it was signed",
            Reply::Body(tampered),
            Reply::Body(server.sign(&manifest).into_bytes()),
        ),
    ];
    for (what, body, signature) in cases {
        server.route("/manifest.json", body);
        server.route("/manifest.json.sig", signature);

        let error = install(&launcher).await.expect_err("nothing is installed");

        assert!(matches!(error, CoreError::BadSignature), "{what}: {error}");
        assert_eq!(listing(&sandbox.game), VANILLA, "{what}");
        assert!(file_requests(&server).is_empty(), "{what}");
        let snapshot = launcher.snapshot();
        assert!(
            matches!(snapshot.remote, Remote::Offline { .. }),
            "{what}: the check counts as failed"
        );
        assert_eq!(snapshot.status, Status::NotInstalled, "{what}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_signed_manifest_that_names_a_file_elsewhere_is_refused() {
    let sandbox = Sandbox::new();
    let server = TestServer::start().await;
    let other = TestServer::start().await;
    // Correctly signed by the trusted key, with the files on another host.
    let manifest = serde_json::to_vec(&Release::new("0.1.0").manifest(&other)).expect("JSON");
    server.route_signed("/manifest.json", &manifest);
    let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");

    let error = install(&launcher).await.expect_err("nothing is installed");

    assert!(matches!(error, CoreError::BadManifest(_)), "{error}");
    assert!(other.requests().is_empty());
    assert_eq!(listing(&sandbox.game), VANILLA);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_release_that_needs_a_newer_launcher_is_not_installed() {
    let sandbox = Sandbox::new();
    let server = TestServer::start().await;
    let mut release = Release::new("0.2.0");
    release.launcher_min = "9.0.0".to_owned();
    server.publish(&release);
    let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");

    let error = install(&launcher).await.expect_err("nothing is installed");

    assert!(
        matches!(&error, CoreError::LauncherTooOld { required } if required == "9.0.0"),
        "{error}"
    );
    assert_eq!(
        launcher.snapshot().remote,
        Remote::LauncherTooOld {
            required: "9.0.0".to_owned()
        }
    );
    assert_eq!(listing(&sandbox.game), VANILLA);
    assert!(file_requests(&server).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_is_installed_while_the_game_runs_or_steam_is_busy() {
    let sandbox = Sandbox::new();
    let server = TestServer::start().await;
    server.publish(&Release::new("0.1.0"));
    let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");

    sandbox.running.store(true, Ordering::SeqCst);
    let error = install(&launcher).await.expect_err("the game runs");
    assert!(matches!(error, CoreError::GameRunning), "{error}");
    assert_eq!(launcher.snapshot().status, Status::GameRunning);
    sandbox.running.store(false, Ordering::SeqCst);

    support::write_app_manifest_in_state(&sandbox.steam, support::INSTALL_DIR, 1026);
    let error = install(&launcher).await.expect_err("Steam is busy");
    assert!(matches!(error, CoreError::SteamBusy), "{error}");

    assert_eq!(listing(&sandbox.game), VANILLA);
    assert!(file_requests(&server).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_game_started_during_the_download_gets_nothing_written() {
    let sandbox = Sandbox::new();
    let server = TestServer::start().await;
    server.publish(&Release::new("0.1.0"));
    let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");

    // The game starts after the zip has arrived and before the first write.
    let error = launcher
        .install(&|progress: Progress| {
            if progress.phase == Phase::InstallingBepInEx {
                sandbox.running.store(true, Ordering::SeqCst);
            }
        })
        .await
        .expect_err("the game runs by now");

    assert!(matches!(error, CoreError::GameRunning), "{error}");
    assert_eq!(listing(&sandbox.game), VANILLA);
    assert_eq!(sandbox.saved_state().bepinex, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_install_at_the_same_time_is_refused_as_busy() {
    let sandbox = Sandbox::new();
    let server = TestServer::start().await;
    server.publish(&Release::new("0.1.0"));
    let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");

    // The first one is polled first and is waiting for the server when the
    // second one asks.
    let (first, second) = tokio::join!(install(&launcher), install(&launcher));

    assert!(first.is_ok());
    assert!(matches!(second, Err(CoreError::Busy)));
    assert_eq!(launcher.snapshot().status, Status::FirstRunNeeded);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_launcher_that_lost_its_state_adopts_what_it_installed_before() {
    let sandbox = Sandbox::new();
    let server = TestServer::start().await;
    server.publish(&Release::new("0.1.0"));
    install(&Launcher::open(sandbox.config(&server)).expect("the launcher opens"))
        .await
        .expect("the install works");
    std::fs::remove_file(sandbox.data.join("state.json")).expect("the state is lost");
    let requests = file_requests(&server).len();

    let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");
    let installed = install(&launcher).await.expect("the install works");

    assert_eq!(file_requests(&server).len(), requests, "nothing is fetched");
    assert_eq!(installed.mod_installed.as_deref(), Some("0.1.0"));
    // Without its record the launcher cannot prove the loader is its own, so
    // it treats it as somebody else's from now on.
    assert_eq!(
        sandbox.saved_state().bepinex.expect("on record").owner,
        Owner::Adopted
    );
}

/// The shell shares one launcher between its commands and awaits these steps
/// from async commands, which may move between threads. Compiling is the
/// test: the futures are made and dropped without being run.
#[tokio::test(flavor = "multi_thread")]
async fn a_link_in_the_launchers_own_staging_folder_is_never_followed() {
    // The staging folder itself is a link, and then one entry of it is.
    for linked in ["staging", "staging/bepinex"] {
        let sandbox = Sandbox::new();
        let server = TestServer::start().await;
        server.publish(&Release::new("0.1.0"));
        // A folder that is not the launcher's, with a file that must survive
        // and must stay alone.
        let outside = sandbox.steam.join("outside");
        put(&outside, "precious.txt", b"keep me");
        let link = sandbox.data.join(linked);
        std::fs::create_dir_all(link.parent().expect("a parent")).expect("the data root");
        link_dir(&outside, &link);

        // Opening clears the staging folder, and must not clear through.
        let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");
        assert_eq!(listing(&outside), ["precious.txt"], "{linked}: after open");

        let error = install(&launcher)
            .await
            .expect_err("nothing is staged through a link");

        assert!(
            matches!(error, CoreError::UnsafePath { .. }),
            "{linked}: {error}"
        );
        assert_eq!(
            listing(&outside),
            ["precious.txt"],
            "{linked}: nothing was downloaded or unpacked through the link"
        );
        assert_eq!(
            std::fs::read(outside.join("precious.txt")).expect("the file survives"),
            b"keep me"
        );
        assert_eq!(listing(&sandbox.game), VANILLA, "{linked}");
        assert!(
            file_requests(&server).is_empty(),
            "{linked}: refused before a download"
        );
    }
}

/// What an install that died left in the game folder, told by what is there.
fn where_it_died(game: &Path) -> &'static str {
    let there = |relative: &str| game.join(relative).exists();
    if there("winhttp.dll") {
        "the proxy DLL is in place"
    } else if there("doorstop_config.ini") {
        "after the small root files, before winhttp.dll"
    } else if there("BepInEx/core") {
        "after both folders, before the root files"
    } else if there("dotnet") {
        "after dotnet, before BepInEx"
    } else if there(paths::INSTALL_READY) {
        "everything staged, nothing in place"
    } else if game.join(paths::INSTALL_PART).join("BepInEx").exists() {
        "after both folders were copied"
    } else if game
        .join(paths::INSTALL_PART)
        .join("dotnet/shared")
        .exists()
    {
        "after all of dotnet was copied"
    } else if game
        .join(paths::INSTALL_PART)
        .join(paths::DOTNET_PROOF)
        .exists()
    {
        "after a part of dotnet"
    } else {
        "before the first write"
    }
}

/// The launcher is killed at every step of putting BepInEx into the game
/// folder, one step per round. The kill is a panic in the question "is the
/// game running", which is asked before every step: it unwinds through the
/// placement without running any of its cleanup, which is what the process
/// dying does.
#[tokio::test(flavor = "multi_thread")]
async fn an_install_killed_at_any_step_is_undone_by_the_next_start() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    let mut complete: Vec<String> = BEPINEX_FILES
        .iter()
        .map(|file| file.0.to_owned())
        .chain(VANILLA.map(str::to_owned))
        .chain([paths::MOD_DLL.to_owned()])
        .collect();
    complete.sort();

    for then_remove in [false, true] {
        let mut died_at = Vec::new();
        for kill_at in 1.. {
            let sandbox = Sandbox::new();
            let server = TestServer::start().await;
            server.publish(&Release::new("0.1.0"));
            let armed = Arc::new(AtomicBool::new(false));
            let asked = Arc::new(AtomicUsize::new(0));
            let mut config = sandbox.config(&server);
            let (switch, counter) = (Arc::clone(&armed), Arc::clone(&asked));
            config.running = Arc::new(move |_| {
                if switch.load(Ordering::SeqCst)
                    && counter.fetch_add(1, Ordering::SeqCst) + 1 == kill_at
                {
                    // Not `panic!`: this is not a failure of the test, and
                    // it should not print like one.
                    std::panic::resume_unwind(Box::new("the launcher is killed here"));
                }
                false
            });
            let launcher = Launcher::open(config).expect("the launcher opens");

            // Armed for exactly the step that writes BepInEx.
            let killed = launcher
                .install(&|progress: Progress| {
                    armed.store(progress.phase == Phase::InstallingBepInEx, Ordering::SeqCst);
                })
                .await;
            armed.store(false, Ordering::SeqCst);
            if killed.is_ok() {
                // The question was asked fewer than `kill_at` times: every
                // step has had its round.
                break;
            }
            let stage = where_it_died(&sandbox.game);
            died_at.push(stage);
            assert!(
                !sandbox.game.join("winhttp.dll").exists(),
                "{stage}: the game would load half an install"
            );
            assert_eq!(sandbox.saved_state().bepinex, None, "{stage}");
            drop(launcher);

            // The next start of the launcher.
            let launcher = Launcher::open(sandbox.config(&server)).expect("the launcher opens");
            if then_remove {
                let outcome = launcher
                    .remove_all(false)
                    .unwrap_or_else(|error| panic!("{stage}: the removal fails: {error}"));
                assert!(outcome.kept.is_empty(), "{stage}: {:?}", outcome.kept);
                assert_eq!(listing(&sandbox.game), VANILLA, "{stage}");
                assert_eq!(
                    std::fs::read_dir(&sandbox.game).expect("listed").count(),
                    VANILLA.len(),
                    "{stage}: not even an empty folder is left"
                );
            } else {
                assert_eq!(
                    listing(&sandbox.game),
                    VANILLA,
                    "{stage}: opening the launcher took the half install back out"
                );
                let installed = install(&launcher)
                    .await
                    .unwrap_or_else(|error| panic!("{stage}: the next install fails: {error}"));
                assert_eq!(installed.status, Status::FirstRunNeeded, "{stage}");
                assert_eq!(listing(&sandbox.game), complete, "{stage}");
                assert_eq!(
                    sandbox.saved_state().bepinex.map(|record| record.owner),
                    Some(Owner::Launcher),
                    "{stage}"
                );
                assert!(!sandbox.game.join(paths::INSTALL_PART).exists(), "{stage}");
                assert!(!sandbox.game.join(paths::INSTALL_READY).exists(), "{stage}");
            }
        }

        for stage in [
            "after a part of dotnet",
            "after dotnet, before BepInEx",
            "after both folders, before the root files",
            "after the small root files, before winhttp.dll",
        ] {
            assert!(
                died_at.contains(&stage),
                "no round died {stage}: {died_at:?}"
            );
        }
    }
}

#[test]
fn the_launcher_is_shared_and_its_async_steps_move_between_threads() {
    fn is_send<T: Send>(_: T) {}
    fn is_shared<T: Send + Sync>() {}

    let sandbox = Sandbox::new();
    let launcher = Launcher::open(sandbox.config_offline()).expect("the launcher opens");

    is_shared::<Launcher>();
    is_send(launcher.install(&quiet));
    is_send(launcher.check());
}
