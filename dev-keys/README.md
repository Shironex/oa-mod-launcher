# The development key

`dev.key` and `dev.key.pub` are a throwaway minisign key pair made with `pnpm tauri signer generate`. The private half is committed on purpose and has an empty password, so it is public and a signature by it proves nothing.

- It signs local test manifests and the fixture `crates/core/tests/data/signed-by-dev-key.json`. It signs nothing real.
- A debug build of the launcher trusts it next to the release key. In a release build the constant does not exist (`#[cfg(debug_assertions)]` in `crates/core/src/signature.rs`), and `verify_manifest` never accepts it.
- The release keys are not in this repository and never will be.
