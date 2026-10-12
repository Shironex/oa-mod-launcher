//! What is in the game folder right now.
//!
//! Read fresh from disk every time, and read only: nothing here writes. The
//! state file says what the launcher installed; this module says what is
//! really there, and the two together decide what the launcher may call its
//! own.

use std::path::Path;

use serde::Serialize;

use crate::error::Result;
use crate::files;
use crate::game::paths;
use crate::manifest;
use crate::state::{Owner, State};

/// What kind of BepInEx the game folder holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BepInEx {
    /// None at all.
    Absent,
    /// The build the launcher unpacked.
    Ours {
        /// Its display text.
        label: String,
    },
    /// The build the mod needs, put there by somebody else. The launcher uses
    /// it and does not treat it as its own.
    Adopted {
        /// Its display text.
        label: String,
    },
    /// An IL2CPP build of BepInEx the launcher does not know. The mod is
    /// installed into it with a warning.
    Foreign,
    /// Something the mod cannot be installed into.
    Unsupported {
        /// What was found, as a sentence that names the file.
        reason: String,
    },
}

impl BepInEx {
    /// Whether nothing of BepInEx is there.
    pub fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }
}

/// The facts about one game folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    /// What kind of BepInEx is there.
    pub bepinex: BepInEx,
    /// The SHA-256 of the mod DLL, when there is one.
    pub mod_sha256: Option<String>,
    /// The names in `BepInEx/plugins` that are not the mod, sorted.
    pub foreign_plugins: Vec<String>,
    /// Whether BepInEx has prepared the game: the first start is over.
    pub interop_ready: bool,
    /// Whether the game was updated after BepInEx prepared it, so the next
    /// start prepares it again and takes minutes.
    pub slow_next_start: bool,
}

/// Look at the game folder `root`, which must be canonical.
///
/// `state` says what the launcher installed. `reference` is the BepInEx block
/// of the newest manifest that is known, if any: without it and without a
/// record in the state, a BepInEx on disk cannot be recognised and counts as
/// foreign.
///
/// # Errors
///
/// [`crate::CoreError::UnsafePath`] when one of the places looked at is
/// reached through a link, [`crate::CoreError::Io`] when a file that is there
/// cannot be read.
pub fn inspect(root: &Path, state: &State, reference: Option<&manifest::BepInEx>) -> Result<Facts> {
    let mod_dll = files::inside(root, paths::MOD_DLL)?;
    let mod_sha256 = if mod_dll.is_file() {
        Some(files::sha256_file(&mod_dll)?.1)
    } else {
        None
    };

    let ready = files::inside(root, paths::INTEROP_READY)?;
    let prepared_for = files::inside(root, paths::INTEROP_HASH)?;
    let slow_next_start = match (
        modified(&root.join(crate::game::ASSEMBLY)),
        modified(&prepared_for),
    ) {
        (Some(game), Some(prepared)) => game > prepared,
        _ => false,
    };

    Ok(Facts {
        bepinex: classify(root, state, reference)?,
        mod_sha256,
        foreign_plugins: foreign_plugins(root)?,
        interop_ready: ready.is_file(),
        slow_next_start,
    })
}

fn modified(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

fn classify(root: &Path, state: &State, reference: Option<&manifest::BepInEx>) -> Result<BepInEx> {
    let core = files::inside(root, paths::CORE)?;
    let dotnet = files::inside(root, paths::DOTNET)?;
    let winhttp = files::inside(root, paths::WINHTTP)?;
    let doorstop = files::inside(root, paths::DOORSTOP_CONFIG)?;
    let probe = files::inside(root, paths::PROBE)?;

    if !core.exists() && !dotnet.exists() && !winhttp.exists() && !doorstop.exists() {
        return Ok(BepInEx::Absent);
    }
    let unsupported = |reason: &str| {
        Ok(BepInEx::Unsupported {
            reason: reason.to_owned(),
        })
    };
    if !probe.is_file() {
        return if core.exists() {
            unsupported(
                "BepInEx/core has no BepInEx.Unity.IL2CPP.dll, so it is not the IL2CPP build \
                 of BepInEx 6 the mod needs",
            )
        } else if winhttp.exists() {
            unsupported("winhttp.dll is in the game folder with no BepInEx beside it")
        } else {
            unsupported(
                "dotnet or doorstop_config.ini is in the game folder with no BepInEx beside it",
            )
        };
    }
    if !winhttp.is_file() {
        return unsupported(
            "BepInEx is there but winhttp.dll is missing, so the game does not load it \
             (an antivirus program may have removed the file)",
        );
    }

    let on_disk = files::sha256_file(&probe)?.1;
    if let Some(record) = &state.bepinex
        && record.probe_sha256 == on_disk
    {
        let label = record.label.clone();
        return Ok(match record.owner {
            Owner::Launcher => BepInEx::Ours { label },
            Owner::Adopted => BepInEx::Adopted { label },
        });
    }
    if let Some(reference) = reference
        && reference.probe_sha256 == on_disk
    {
        return Ok(BepInEx::Adopted {
            label: reference.label.clone(),
        });
    }
    Ok(BepInEx::Foreign)
}

/// The names in `BepInEx/plugins` other than the mod's own folder.
fn foreign_plugins(root: &Path) -> Result<Vec<String>> {
    let plugins = files::inside(root, paths::PLUGINS)?;
    let mut names: Vec<String> = match std::fs::read_dir(&plugins) {
        Ok(entries) => entries
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name != paths::MOD_FOLDER_NAME)
            .collect(),
        Err(_) => Vec::new(),
    };
    names.sort();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::state::BepInExRecord;

    const PROBE_BYTES: &[u8] = b"the IL2CPP assembly of the pinned build";

    fn game() -> (tempfile::TempDir, std::path::PathBuf) {
        let temp = tempfile::tempdir().expect("a temporary directory");
        let root = std::fs::canonicalize(temp.path()).expect("a real path");
        std::fs::write(root.join(crate::game::EXE), b"MZ").expect("the exe");
        std::fs::write(root.join(crate::game::ASSEMBLY), b"MZ").expect("the assembly");
        (temp, root)
    }

    fn put(root: &Path, relative: &str, body: &[u8]) {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
        std::fs::write(path, body).expect("a file");
    }

    fn pinned_build(root: &Path) {
        put(root, paths::PROBE, PROBE_BYTES);
        put(root, paths::WINHTTP, b"proxy");
        put(root, paths::DOORSTOP_CONFIG, b"[General]");
        put(root, paths::DOTNET_PROOF, b"clr");
    }

    fn reference() -> manifest::BepInEx {
        manifest::BepInEx {
            label: "6.0.0-be.788".to_owned(),
            url: String::new(),
            size: 1,
            sha256: String::new(),
            probe_sha256: sha256(PROBE_BYTES),
            root_files: BTreeMap::new(),
        }
    }

    fn sha256(bytes: &[u8]) -> String {
        use sha2::Digest;
        crate::net::hex(&sha2::Sha256::digest(bytes))
    }

    fn claimed_by(owner: Owner) -> State {
        State {
            bepinex: Some(BepInExRecord {
                owner,
                label: "recorded".to_owned(),
                probe_sha256: sha256(PROBE_BYTES),
                root_files: BTreeMap::new(),
            }),
            ..State::default()
        }
    }

    fn kind(root: &Path, state: &State, reference: Option<&manifest::BepInEx>) -> BepInEx {
        inspect(root, state, reference)
            .expect("the folder is read")
            .bepinex
    }

    fn reason(kind: BepInEx) -> String {
        match kind {
            BepInEx::Unsupported { reason } => reason,
            other => panic!("expected an unsupported loader, got {other:?}"),
        }
    }

    #[test]
    fn a_vanilla_game_has_no_bepinex_and_no_mod() {
        let (_temp, root) = game();

        let facts = inspect(&root, &State::default(), Some(&reference())).expect("read");

        assert_eq!(
            facts,
            Facts {
                bepinex: BepInEx::Absent,
                mod_sha256: None,
                foreign_plugins: Vec::new(),
                interop_ready: false,
                slow_next_start: false,
            }
        );
    }

    #[test]
    fn the_pinned_build_is_ours_when_the_state_says_so() {
        let (_temp, root) = game();
        pinned_build(&root);

        assert_eq!(
            kind(&root, &claimed_by(Owner::Launcher), None),
            BepInEx::Ours {
                label: "recorded".to_owned()
            }
        );
        assert_eq!(
            kind(&root, &claimed_by(Owner::Adopted), Some(&reference())),
            BepInEx::Adopted {
                label: "recorded".to_owned()
            }
        );
    }

    #[test]
    fn the_pinned_build_without_a_claim_is_adopted() {
        let (_temp, root) = game();
        pinned_build(&root);

        assert_eq!(
            kind(&root, &State::default(), Some(&reference())),
            BepInEx::Adopted {
                label: "6.0.0-be.788".to_owned()
            }
        );
    }

    #[test]
    fn another_il2cpp_build_is_foreign_even_when_the_state_claims_one() {
        let (_temp, root) = game();
        pinned_build(&root);
        put(&root, paths::PROBE, b"some other build");

        assert_eq!(
            kind(&root, &claimed_by(Owner::Launcher), Some(&reference())),
            BepInEx::Foreign
        );
        // Nothing to compare with: unknown is foreign, never ours.
        put(&root, paths::PROBE, PROBE_BYTES);
        assert_eq!(kind(&root, &State::default(), None), BepInEx::Foreign);
    }

    #[test]
    fn what_the_mod_cannot_use_is_unsupported_and_named() {
        // BepInEx 5: a core folder without the IL2CPP assembly.
        let (_temp, root) = game();
        put(&root, "BepInEx/core/BepInEx.dll", b"five");
        put(&root, paths::WINHTTP, b"proxy");
        assert!(
            reason(kind(&root, &State::default(), Some(&reference())))
                .contains("BepInEx.Unity.IL2CPP.dll")
        );

        // A proxy DLL of something else.
        let (_temp, root) = game();
        put(&root, paths::WINHTTP, b"proxy");
        assert!(reason(kind(&root, &State::default(), None)).contains("winhttp.dll"));

        // Leftovers that match no other class.
        let (_temp, root) = game();
        put(&root, paths::DOTNET_PROOF, b"clr");
        assert!(reason(kind(&root, &State::default(), None)).contains("dotnet"));

        // The pinned build with its proxy DLL gone: the game would not load it.
        let (_temp, root) = game();
        pinned_build(&root);
        std::fs::remove_file(root.join(paths::WINHTTP)).expect("remove the proxy");
        assert!(
            reason(kind(
                &root,
                &claimed_by(Owner::Launcher),
                Some(&reference())
            ))
            .contains("winhttp.dll is missing")
        );
    }

    #[test]
    fn the_mod_is_hashed_and_other_plugins_are_listed() {
        let (_temp, root) = game();
        pinned_build(&root);
        put(&root, paths::MOD_DLL, b"the mod");
        put(&root, "BepInEx/plugins/Zeta/Zeta.dll", b"z");
        put(&root, "BepInEx/plugins/alone.dll", b"a");

        let facts = inspect(&root, &State::default(), None).expect("read");

        assert_eq!(facts.mod_sha256, Some(sha256(b"the mod")));
        assert_eq!(facts.foreign_plugins, ["Zeta", "alone.dll"]);
    }

    #[test]
    fn the_first_start_is_over_when_the_interop_assembly_exists() {
        let (_temp, root) = game();
        pinned_build(&root);
        assert!(
            !inspect(&root, &State::default(), None)
                .expect("read")
                .interop_ready
        );

        put(&root, paths::INTEROP_READY, b"generated");
        assert!(
            inspect(&root, &State::default(), None)
                .expect("read")
                .interop_ready
        );
    }

    #[test]
    fn a_game_newer_than_what_bepinex_prepared_means_a_slow_next_start() {
        let (_temp, root) = game();
        pinned_build(&root);
        put(&root, paths::INTEROP_HASH, b"hash");
        let slow = |root: &Path| {
            inspect(root, &State::default(), None)
                .expect("read")
                .slow_next_start
        };
        let set = |relative: &str, seconds: u64| {
            let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds);
            std::fs::File::options()
                .write(true)
                .open(root.join(relative))
                .expect("open")
                .set_modified(time)
                .expect("set the time");
        };

        set(crate::game::ASSEMBLY, 1_000_000);
        set(paths::INTEROP_HASH, 2_000_000);
        assert!(!slow(&root), "prepared after the game was last changed");

        set(crate::game::ASSEMBLY, 3_000_000);
        assert!(slow(&root), "the game was updated since");

        std::fs::remove_file(root.join(paths::INTEROP_HASH)).expect("remove");
        assert!(
            !slow(&root),
            "never prepared is the first start, not a slow one"
        );
    }

    #[test]
    fn a_folder_reached_through_a_link_is_not_read() {
        let (_temp, root) = game();
        let elsewhere = root.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).expect("another folder");
        crate::files::tests::link_dir(&elsewhere, &root.join("BepInEx"));

        assert!(matches!(
            inspect(&root, &State::default(), None),
            Err(crate::CoreError::UnsafePath { .. })
        ));
    }
}
