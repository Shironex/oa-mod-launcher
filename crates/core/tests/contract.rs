//! The contract with the writer. `scripts/fixtures/contract/manifest.json` is
//! what the release scripts write for fixed inputs: a Node test
//! (`scripts/lib/manifest.test.mjs`) regenerates it and compares byte for
//! byte. Here the launcher's own parser reads the same file, so the writer and
//! the reader cannot drift apart without one of the two tests failing.
//!
//! Values are compared, not only "it parses", so a key renamed by the writer
//! is seen here.

use oa_mod_launcher_core::manifest::ROOT_FILES;
use oa_mod_launcher_core::{Manifest, Version};

const MANIFEST: &str = include_str!("../../../scripts/fixtures/contract/manifest.json");

#[test]
fn the_launcher_reads_the_manifest_the_scripts_write() {
    let manifest = Manifest::parse(MANIFEST).expect("the fixture is a manifest");

    assert_eq!(manifest.schema, 1);
    assert_eq!(manifest.version, "0.1.0");
    assert_eq!(manifest.published.as_deref(), Some("2026-10-20T18:00:00Z"));
    assert_eq!(manifest.notes.as_deref(), Some("The first public build."));
    assert_eq!(
        manifest.minimum_launcher(),
        Version {
            major: 0,
            minor: 1,
            patch: 0
        }
    );

    assert_eq!(
        manifest.mod_file.url,
        "https://github.com/Shironex/oa-mod/releases/download/v0.1.0/OaMod-0.1.0.dll"
    );
    assert_eq!(manifest.mod_file.size, 14);
    assert_eq!(
        manifest.mod_file.sha256,
        "57d23bf90439afc8e9222eaa512a0b02d8616857703d5d357016742c01a6adb3"
    );

    let bepinex = &manifest.bepinex;
    assert_eq!(bepinex.label, "6.0.0-be.788");
    assert_eq!(
        bepinex.url,
        "https://github.com/Shironex/oa-mod/releases/download/bepinex-6.0.0-be.788/BepInEx-Unity.IL2CPP-win-x64-6.0.0-be.788.zip"
    );
    assert_eq!(bepinex.size, 34_336_405);
    assert_eq!(
        bepinex.sha256,
        "f4cc496bd098a0df4164b81e3737297707f13a47c2478dba2f60eefab784817a"
    );
    assert_eq!(
        bepinex.probe_sha256,
        "ba9c736f19e7f60b7f6764adb0b7908c0a2b394e09b6c09863528c7f2bc86095"
    );
    let mut names: Vec<&str> = bepinex.root_files.keys().map(String::as_str).collect();
    let mut expected = ROOT_FILES.to_vec();
    names.sort_unstable();
    expected.sort_unstable();
    assert_eq!(names, expected);
    assert_eq!(
        bepinex.root_files["winhttp.dll"],
        "bf5d99b5c9ae7b43146898d0fc30f8bba698803a48b759e80ff61e22574178f7"
    );

    assert_eq!(manifest.game.builds, ["20236533", "20240000"]);
}
