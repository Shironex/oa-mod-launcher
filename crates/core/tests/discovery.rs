//! Finding the game on disk: a fake Steam installation in a temporary folder,
//! read through the same two functions the launcher will call.

mod support;

use oa_mod_launcher_core::{CoreError, game, steam};

#[test]
fn the_game_is_found_in_the_library_that_lists_it() {
    let temp = tempfile::tempdir().expect("a temporary directory");
    let root = temp.path().join("Steam");
    let second = temp.path().join("Second Drive").join("SteamLibrary");
    support::write_library_list(
        &root,
        &[(&root, &["228980", "730"]), (&second, &[steam::APP_ID])],
    );
    let dir = support::fake_game_folder(&second);

    let found = steam::find_game(&root).expect("the game is found");

    assert_eq!(found.dir, dir);
    assert_eq!(found.build_id, "20236533");
    assert_eq!(found.state_flags, 4);
    assert_eq!(
        game::validate(&found.dir).expect("the folder is the game"),
        std::fs::canonicalize(&dir).expect("a real path")
    );
}

#[test]
fn a_library_is_probed_even_when_the_list_does_not_name_the_game() {
    let temp = tempfile::tempdir().expect("a temporary directory");
    let root = temp.path().join("Steam");
    let second = temp.path().join("SteamLibrary");
    support::write_library_list(&root, &[(&root, &[]), (&second, &["730"])]);
    let dir = support::fake_game_folder(&second);

    assert_eq!(steam::find_game(&root).expect("found by probing").dir, dir);
}

#[test]
fn without_a_library_list_the_steam_folder_itself_is_a_library() {
    let temp = tempfile::tempdir().expect("a temporary directory");
    let root = temp.path().join("Steam");
    let dir = support::fake_game_folder(&root);

    assert_eq!(steam::find_game(&root).expect("found in the root").dir, dir);

    std::fs::write(
        root.join("steamapps").join("libraryfolders.vdf"),
        "not a library list {",
    )
    .expect("a damaged list");
    assert_eq!(
        steam::find_game(&root)
            .expect("a damaged list is ignored")
            .dir,
        dir
    );
}

#[test]
fn a_steam_without_the_game_says_so() {
    let temp = tempfile::tempdir().expect("a temporary directory");
    let root = temp.path().join("Steam");
    let second = temp.path().join("SteamLibrary");
    support::write_library_list(&root, &[(&root, &["228980"]), (&second, &["730"])]);

    assert!(matches!(
        steam::find_game(&root),
        Err(CoreError::GameNotFound)
    ));
    assert!(matches!(
        steam::find_game(&temp.path().join("nowhere")),
        Err(CoreError::GameNotFound)
    ));
}

#[test]
fn a_manifest_that_points_outside_the_library_is_refused() {
    let temp = tempfile::tempdir().expect("a temporary directory");
    let root = temp.path().join("Steam");
    support::write_library_list(&root, &[(&root, &[steam::APP_ID])]);
    // The folder exists, so only the name check can refuse it.
    support::fake_game_folder(&root);
    support::write_app_manifest(&root, "..\\\\..\\\\..\\\\Steam");

    assert!(matches!(
        steam::find_game(&root),
        Err(CoreError::BadSteamFile { .. })
    ));
}
