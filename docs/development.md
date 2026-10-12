# OA Mod Launcher: development

How to run the launcher from source, what the checks are, how a release is made, and what the mod repository's release runs from this repository. The formats both repositories share are in [contract.md](contract.md).

## What it does

- Finds _Golf With Your Friends_ through Steam (registry, `libraryfolders.vdf`, `appmanifest_431240.acf`), or takes a folder chosen by hand.
- Reads the signed `manifest.json` of the newest OA Mod release, downloads BepInEx and the mod, checks size and SHA-256 of both, and puts them into the game folder.
- Starts the game through Steam (`steam://rungameid/431240`).
- Removes only the mod, or everything BepInEx brought, after copying other people's plugins to a backup folder.
- Updates itself through `tauri-plugin-updater`, from a signed installer.

Two crates: `crates/core` holds every rule and knows nothing about Tauri, so it is tested without a window. `src-tauri` is the shell that turns it into commands and events. The page in `src/` shows a snapshot and calls a command; it decides nothing and makes no network request.

| Folder        | What is in it                                                                                   |
| ------------- | ----------------------------------------------------------------------------------------------- |
| `crates/core` | manifest, signature check, download, unpacking, Steam discovery, install, removal, the snapshot |
| `src-tauri`   | the commands, the events, the poll, the launcher's own update (`updater.rs`), the bundle config |
| `src`         | the page: `state/view.ts` turns a snapshot into words, `state/store.ts` calls the commands      |
| `scripts`     | writing and signing a manifest, `latest.json`, the release notes, a local file server           |
| `dev-keys`    | the development key, public on purpose (see its README)                                         |
| `docs`        | this file and the contract                                                                      |

On disk the launcher keeps its own files in `%LOCALAPPDATA%\OaModLauncher`: `state.json`, `staging/`, `cache/manifest.json`, `backups/` and `launcher.log`. Everything else it writes is inside the game folder, under names that are constants of the code.

## Run it in development

Rust (the version in `rust-toolchain.toml`), Node 22, pnpm 10 and the WebView2 runtime.

```sh
pnpm install
pnpm tauri dev
```

A development build never updates itself: `check_launcher_update` answers "none" without a request.

Do not point a development build at your real game folder for install or removal tests. Use a throwaway data root and a fake game folder, as below.

### Against a local test server

A debug build reads two environment variables (a release build reads neither):

- `OA_MOD_LAUNCHER_FEED`: the address of a `manifest.json`, or of the folder it is in. A debug build that is pointed at another manifest also accepts the files next to it, and trusts the development key.
- `OA_MOD_LAUNCHER_ROOT`: the folder for `state.json`, `staging/`, `cache/` and `backups/`, in place of the real per-user folder.

1. Make a feed folder with a mod file, the BepInEx zip and a copy of the mod repository's `bepinex.json` whose `url` names the zip on the local server, for example `http://127.0.0.1:8123/BepInEx.zip`. Plain http is accepted for this computer only.

2. Write and sign the manifest. For an address that is not on github.com the development key is used by default:

   ```sh
   node scripts/build-manifest.mjs --mod feed/OaMod-0.1.0.dll --version 0.1.0 \
     --base-url http://127.0.0.1:8123/ --bepinex feed/bepinex.json \
     --game-builds 20236533 --notes "A local test." --out feed
   ```

3. Serve it, slowed down so the progress can be watched:

   ```sh
   pnpm serve --root feed --throttle 2000
   ```

4. Make a fake game folder: any folder with two files named `Golf With Your Friends.exe` and `GameAssembly.dll` in it. Start the launcher against the server and choose that folder in the window:

   ```powershell
   $env:OA_MOD_LAUNCHER_FEED = 'http://127.0.0.1:8123/'
   $env:OA_MOD_LAUNCHER_ROOT = "$env:TEMP\oa-launcher-root"
   pnpm tauri dev
   ```

### The window without a game

`pnpm dev` and `http://localhost:15190/?preview=<name>` show a fixed state without the Rust side. The names are the keys of `PREVIEWS` in `src/dev/preview.ts`: one per row of the status table, the dialogs, and the launcher's own update (`launcher-update`, `about-update`).

## Checks

The nine checks CI runs (`.github/workflows/ci.yml`), in the order that works on a fresh checkout:

```sh
pnpm build                 # first: the Rust shell embeds dist/ and does not compile without it
pnpm lint
pnpm format:check
pnpm typecheck
pnpm test
pnpm test:scripts
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
```

After a change to a command, a type it returns or an event, regenerate the bindings and commit them (PowerShell: `$env:UPDATE_BINDINGS = '1'` first):

```sh
UPDATE_BINDINGS=1 cargo test -p oa-mod-launcher
```

After a change to `Cargo.lock` or `pnpm-lock.yaml`, run `pnpm notices` and commit `THIRD-PARTY-NOTICES.txt`. The installer carries that file, so a build needs it to exist.

Building the installer:

```sh
# No key at all: no .sig is made. This is what the release workflow's build job runs.
pnpm tauri build --bundles nsis --ci --config src-tauri/tauri.unsigned.conf.json

# The development key: the CLI warns that it does not match the key in the config.
# Expected for a trial, and such an installer must never be published.
TAURI_SIGNING_PRIVATE_KEY="$(tr -d '\r\n' < dev-keys/dev.key)" TAURI_SIGNING_PRIVATE_KEY_PASSWORD='' \
  pnpm tauri build --bundles nsis --ci
```

The installer lands in `target/release/bundle/nsis/`.

The real BepInEx zip is not in the repository. One ignored test runs it through the real unpacking, the check against the release values and the placement, into a temporary folder:

```sh
OA_MOD_BEPINEX_ZIP=<path to the zip> cargo test -p oa-mod-launcher-core --lib -- --ignored the_real_bepinex_zip
```

## Releasing

Two things are released, from two repositories:

| What                     | Repository                 | Who builds it                                   |
| ------------------------ | -------------------------- | ----------------------------------------------- |
| The launcher             | `Shironex/oa-mod-launcher` | GitHub Actions, `.github/workflows/release.yml` |
| The mod and its manifest | `Shironex/oa-mod`          | the mod repository's own workflow, see below    |

Launcher installers must not be released from the mod repository, and mod files not from this one: "latest" in each repository has to stay what the launcher reads there.

### Before the first release

In the settings of `Shironex/oa-mod-launcher`:

1. Create the environment `release`. Add yourself as a required reviewer, and under "Deployment branches and tags" allow only the tag pattern `v*`.
2. Add two secrets to that environment (not to the repository):
   - `UPDATER_PRIVATE_KEY`: the content of the updater key file (`updater.key`), one line of base64.
   - `UPDATER_PRIVATE_KEY_PASSWORD`: its password.

No repository variable is needed. The workflow's own token writes the release (`contents: write` on the publish job only).

### Release the launcher

A pushed tag starts the release. I approve it once, and the runners build, sign, verify and publish.

1. Set the new version in two places: `[workspace.package]` in `Cargo.toml` and `version` in `package.json`. The workflow stops when they differ. Run `cargo check` so `Cargo.lock` follows.
2. Add a section for the version at the top of `CHANGELOG.md`: `## 0.1.1 (2026-11-02)`, then the text. It becomes the notes of the GitHub release and the `notes` of `latest.json`, which the launcher shows as plain text, so write plain sentences. A version without a section is not released.
3. Run `pnpm notices` if a lockfile changed beyond the version, make the [checks](#checks) green, commit and push `main`. Wait for the CI run of that commit.
4. Tag that commit and push the tag:

   ```sh
   git tag v0.1.1
   git push origin v0.1.1
   ```

5. Open the run of the `release` workflow. When `build` is green, `sign` waits: click "Review deployments", tick `release`, approve. The summary of `build` shows the SHA-256 of the installer that is about to be signed.
6. Wait for `publish`. Its last step reads `https://github.com/Shironex/oa-mod-launcher/releases/latest/download/latest.json` and compares it with what was built. Then open an installed launcher and update it.

| Job       | Runner  | Has                                                 | Does                                                                                                                                                                                                                                                                                         |
| --------- | ------- | --------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `build`   | Windows | no secret                                           | Checks that the tag is `v` plus the version, that `package.json` agrees, that the changelog has the section and that no release of the tag is published. Runs the tests. Builds the NSIS installer without a signature and uploads it as `OaModLauncher-<version>-windows-x64-setup.exe`     |
| `sign`    | Windows | the updater key and its password, after my approval | Installs the Tauri CLI without running install scripts, downloads the installer and signs it with `tauri signer sign --app-version <version>`. Uploads the `.sig`                                                                                                                            |
| `publish` | Ubuntu  | write access to releases, no signing secret         | Checks the `.sig` against the public key in `tauri.conf.json` and that it carries the version (`verify_file`). Writes `latest.json` and the notes. Removes a draft a failed run left behind, refuses when the release is already published. Creates a draft, compares, publishes, reads back |

**If it fails**

- _`build` fails because of the code._ Nothing was signed or published. Fix it on `main`, then move the tag to the fixed commit (`git tag -f v0.1.1`, `git push -f origin v0.1.1`). Moving a tag is fine as long as its release was never published.
- _A job fails for a reason outside the code._ Use "Re-run failed jobs". The installer of the first attempt is kept for a week and is used again; `sign` asks for approval again.
- _`publish` fails before its "Publish" step._ Nobody saw anything: a draft is never "latest". Re-run the failed job; it removes the draft and starts over.
- _The last step fails._ The release **is** published. Do not re-run: look at the release page and at the address by hand.
- _A published release is bad._ Do not replace its files and do not reuse its version. Release a higher version.

**Rehearsal.** A manual run from a branch (`gh workflow run release.yml --ref main`) runs `build` completely and uploads the unsigned installer as an artifact. `sign` and `publish` are skipped, no approval is asked for and nothing is published. Do one after a change to the workflow.

On every push to `main`, CI also builds the installer signed with the development key and runs the same signature check and `build-latest.mjs` on it (job `dev-installer`). That artifact is named `DEV-KEY-NOT-A-RELEASE-launcher-installer` and no installed launcher accepts it.

### What the mod repository's release runs from here

The mod repository (`Shironex/oa-mod`) checks this repository out at a pinned commit into a subfolder named `launcher/` and uses three things from it: `scripts/build-manifest.mjs`, `scripts/sign-file.mjs` and the example `verify_manifest`. I ran the commands below with this repository cloned into `launcher/` of an empty folder, from that outer folder. Signed with the development key, the last step refuses the manifest with "not a signature ... by the release key", which is what it has to do for any key but the real one.

In its `build` job (no secret, no install needed, Node 22 only):

```sh
node launcher/scripts/build-manifest.mjs \
  --mod "OaMod-$VERSION.dll" \
  --version "$VERSION" \
  --base-url "https://github.com/Shironex/oa-mod/releases/download/v$VERSION/" \
  --bepinex bepinex.json \
  --game-builds "$(cat refs/buildid.txt)" \
  --changelog CHANGELOG.md \
  --no-sign
```

It writes `manifest.json` into the working directory (`--out <folder>` for another place). `bepinex.json` may carry keys the manifest does not use, such as `origin`.

In its `sign` job (environment `release`, the only job with the manifest secrets). The key reaches the script as a file, the password through the environment and never as an argument:

```sh
pnpm --dir launcher install --frozen-lockfile --ignore-scripts
printf '%s' "$MANIFEST_PRIVATE_KEY" | tr -d '\r\n' > "$RUNNER_TEMP/manifest.key"
node launcher/scripts/sign-file.mjs --file manifest.json \
  --key "$RUNNER_TEMP/manifest.key" --password-env MANIFEST_PRIVATE_KEY_PASSWORD
rm -f "$RUNNER_TEMP/manifest.key"
test -s manifest.json.sig
```

with this on the step, and nowhere else:

```yaml
env:
  MANIFEST_PRIVATE_KEY: ${{ secrets.MANIFEST_PRIVATE_KEY }}
  MANIFEST_PRIVATE_KEY_PASSWORD: ${{ secrets.MANIFEST_PRIVATE_KEY_PASSWORD }}
```

The install is needed for the Tauri CLI only, which does the signing. `corepack enable` first, so the pnpm version of `launcher/package.json` is the one that runs.

In its `publish` job, before anything is uploaded. Run it with `working-directory: launcher`, so `rust-toolchain.toml` of this repository picks the Rust version (a `cargo run --manifest-path launcher/Cargo.toml` from the outer folder also works, but builds with whatever toolchain the outer folder gets):

```sh
rustup toolchain install --no-self-update
cargo run --locked -p oa-mod-launcher-core --example verify_manifest -- \
  ../manifest.json --mod "../OaMod-$VERSION.dll"
```

It checks `manifest.json.sig` next to the manifest against the release key compiled into the launcher, parses the manifest as the launcher does, and compares the size and the SHA-256 of the DLL with it. It exits with 1 and one line starting with `NOT VERIFIED:` otherwise.

What the mod repository needs in its settings for that workflow:

| Kind        | Name                            | What                                                                                   |
| ----------- | ------------------------------- | -------------------------------------------------------------------------------------- |
| Environment | `release`                       | required reviewer, only `v*` tags                                                      |
| Secret      | `MANIFEST_PRIVATE_KEY`          | in the environment `release`: the content of the manifest key file                     |
| Secret      | `MANIFEST_PRIVATE_KEY_PASSWORD` | in the environment `release`: its password                                             |
| Secret      | `REFS_DEPLOY_KEY`               | repository secret: the read-only deploy key of the private build references repository |
| Variable    | `REFS_SHA`                      | repository variable: the commit of the build references to build against               |

When one of the three tools changes here, the pin in the mod repository has to be raised for the change to arrive there.

## Keys

Two release keys, both minisign keys made by `pnpm tauri signer generate`, each with its password. There is no spare of either: if one is lost, everyone installs a new launcher by hand.

| Key        | Signs                      | Public half is in                                       | Private half is                                                |
| ---------- | -------------------------- | ------------------------------------------------------- | -------------------------------------------------------------- |
| `manifest` | `manifest.json` of the mod | `RELEASE_KEYS[0]` in `crates/core/src/signature.rs`     | the secret `MANIFEST_PRIVATE_KEY` of `Shironex/oa-mod`         |
| `updater`  | the launcher installer     | `plugins.updater.pubkey` in `src-tauri/tauri.conf.json` | the secret `UPDATER_PRIVATE_KEY` of `Shironex/oa-mod-launcher` |

Both public halves are frozen into every installed launcher. Tests pin them: `release_keys_are_real` in `signature.rs`, and `the_updater_key_is_the_release_key_and_none_of_the_others` in `updater.rs`, which also checks that the three keys differ.

The development key in `dev-keys/` is none of them. Only a debug build trusts it, and only for manifests.

## Not done

- No macOS or Linux build. The code compiles there, and Steam discovery and the running check are Windows only.
- The installer is not signed with a code signing certificate, so SmartScreen warns.
- An open launcher asks for its own update once, when it starts, and when "Check for updates" is pressed under About. It does not ask again while it stays open.
- A changed BepInEx build in a newer manifest is reported and not installed.

## Third-party material in the launcher

`THIRD-PARTY-NOTICES.txt` lists the Rust crates linked into the launcher and the npm packages bundled into its window, the three fonts among them, each with its licence text. `scripts/build-launcher-notices.mjs` writes it (`pnpm notices`) and the installer carries it. The texts are copied from the packages as they are.

The launcher bundles neither BepInEx nor the mod: it downloads both when you press Install.
