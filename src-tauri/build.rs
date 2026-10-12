fn main() {
    let attributes = tauri_build::Attributes::new()
        .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
    tauri_build::try_build(attributes).expect("tauri-build");

    embed_app_manifest();
}

/// Link `app.manifest` into every target of this crate on Windows.
///
/// `tauri-build` embeds its manifest into the application binary only, so the
/// test binaries would get none. The window code imports a function that only
/// version 6 of the common controls library exports, and without a manifest a
/// test binary binds the old version and stops at load time with
/// `STATUS_ENTRYPOINT_NOT_FOUND`, before a single test runs.
///
/// The file holds what `tauri-build` would have embedded. Its own copy is
/// switched off above, so the two cannot collide as a duplicate resource.
fn embed_app_manifest() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let Ok(directory) = std::env::var("CARGO_MANIFEST_DIR") else {
        return;
    };
    let manifest = std::path::Path::new(&directory).join("app.manifest");

    println!("cargo::rerun-if-changed=app.manifest");
    println!("cargo::rustc-link-arg=/MANIFEST:EMBED");
    println!(
        "cargo::rustc-link-arg=/MANIFESTINPUT:{}",
        manifest.display()
    );
}
