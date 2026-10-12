//! The command and event list, and the TypeScript generated from it.
//!
//! `src/bindings.ts` is generated, committed, and checked by a test: the test
//! fails when the file no longer matches the Rust side, and
//! `UPDATE_BINDINGS=1 cargo test -p oa-mod-launcher` rewrites it.

use tauri_specta::{Builder, ErrorHandlingMode, collect_commands, collect_events};

use crate::commands;

/// The one builder the handler, the events and the bindings all come from.
///
/// Errors reach the page as a rejected promise (`Throw`), so a call site uses
/// plain `try` and `catch`.
pub(crate) fn builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .error_handling(ErrorHandlingMode::Throw)
        .commands(collect_commands![
            commands::snapshot,
            commands::refresh,
            commands::check_for_updates,
            commands::install,
            commands::start_game,
            commands::removal_preview,
            commands::remove_mod,
            commands::remove_all,
            commands::pick_game_folder,
            commands::reset_game_folder,
            commands::set_check_on_start,
            commands::open_folder,
        ])
        .events(collect_events![
            commands::InstallProgress,
            commands::SnapshotChanged
        ])
}

#[cfg(test)]
mod tests {
    use specta_typescript::Typescript;

    use super::*;

    /// The exporter settings. The exporter refuses `u64`, so the byte counts
    /// in the core types are declared as numbers for it: a download is far
    /// below 2^53 bytes.
    fn exporter() -> Typescript {
        Typescript::default()
            .header("// Generated from the Rust commands. Do not edit by hand.\n// Regenerate: UPDATE_BINDINGS=1 cargo test -p oa-mod-launcher")
    }

    /// Where the generated file lives. A compile time path is fine here: it is
    /// only used by this test, never by the shipped program.
    const GENERATED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/bindings.ts");

    #[test]
    fn the_committed_bindings_match_the_commands() {
        let fresh = std::env::temp_dir().join(format!("oa-bindings-{}.ts", std::process::id()));
        builder()
            .export(exporter(), &fresh)
            .expect("the bindings are generated");
        let generated = std::fs::read_to_string(&fresh).expect("the generated file is readable");
        let _ = std::fs::remove_file(&fresh);
        let generated = generated.replace("\r\n", "\n");

        if std::env::var_os("UPDATE_BINDINGS").is_some() {
            std::fs::write(GENERATED, &generated).expect("the bindings file is written");
            return;
        }

        let committed = std::fs::read_to_string(GENERATED)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert!(
            committed == generated,
            "src/bindings.ts is out of date. Run: UPDATE_BINDINGS=1 cargo test -p oa-mod-launcher"
        );
    }
}
