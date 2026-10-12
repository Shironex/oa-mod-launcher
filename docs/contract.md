# The contract between the launcher and the mod repository

Two repositories publish files that an installed launcher reads:

| Repository                 | Publishes                                                             | Read by                    |
| -------------------------- | --------------------------------------------------------------------- | -------------------------- |
| `Shironex/oa-mod`          | the mod DLL, `manifest.json`, `manifest.json.sig`, the BepInEx mirror | the launcher's core (Rust) |
| `Shironex/oa-mod-launcher` | the launcher installer and `latest.json`                              | `tauri-plugin-updater`     |

An installed launcher cannot be changed from outside, only replaced by an update it accepts. So everything on this page is frozen into every copy that is installed. A change here needs a new launcher first, and `launcher.min` in the manifest to keep the old ones away from what they cannot read.

The mod's release workflow checks out this repository at a pinned commit and runs its scripts. This page is what that workflow may rely on.

## The address

| What                      | Address                                                                            | Compiled in at                                             |
| ------------------------- | ---------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| The mod's manifest        | `https://github.com/Shironex/oa-mod/releases/latest/download/manifest.json`        | `DEFAULT_FEED` in `crates/core/src/net.rs`                 |
| The launcher's own update | `https://github.com/Shironex/oa-mod-launcher/releases/latest/download/latest.json` | `plugins.updater.endpoints` in `src-tauri/tauri.conf.json` |

`manifest.json.sig` is read from the same address with `.sig` appended.

"Latest" means the newest published release that is not a prerelease. That is why the BepInEx mirror is a prerelease, and why a mod release is created as a draft and published last, when all of its files are in place.

Only https is accepted, also after redirects.

## Launcher release files

A launcher release holds two files: `OaModLauncher-<version>-windows-x64-setup.exe` (the NSIS installer, Windows x64 only) and `latest.json`, which `scripts/build-latest.mjs` writes:

```json
{
  "version": "0.1.0",
  "notes": "The section of this version in CHANGELOG.md.",
  "pub_date": "2026-10-20T18:00:00.000Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "<the content of the installer's .sig file>",
      "url": "https://github.com/Shironex/oa-mod-launcher/releases/download/v0.1.0/OaModLauncher-0.1.0-windows-x64-setup.exe"
    }
  }
}
```

The signature is made with `tauri signer sign --app-version <version>`, by the updater key. `requireSignedVersion` is on, so an installed launcher refuses an installer whose signature does not carry the version `latest.json` names. Only https is accepted, for `latest.json`, for the redirect GitHub answers with and for the installer.

## Mod release files

A mod release holds `OaMod-<version>.dll`, `manifest.json` and `manifest.json.sig`. The DLL is published bare, not in a zip. The launcher writes it to `BepInEx/plugins/OaMod/OaMod.dll` in the game folder; that path is a constant of the launcher and never comes from the manifest.

### `manifest.json`

```json
{
  "schema": 1,
  "version": "0.1.0",
  "published": "2026-10-20T18:00:00Z",
  "notes": "One line.",
  "mod": {
    "url": "https://github.com/Shironex/oa-mod/releases/download/v0.1.0/OaMod-0.1.0.dll",
    "size": 123456,
    "sha256": "64 hex characters"
  },
  "bepinex": {
    "label": "6.0.0-be.788",
    "url": "https://github.com/Shironex/oa-mod/releases/download/bepinex-6.0.0-be.788/BepInEx-Unity.IL2CPP-win-x64-6.0.0-be.788.zip",
    "size": 34336405,
    "sha256": "f4cc496bd098a0df4164b81e3737297707f13a47c2478dba2f60eefab784817a",
    "probe_sha256": "sha256 of BepInEx/core/BepInEx.Unity.IL2CPP.dll inside the zip",
    "root_files": {
      "winhttp.dll": "sha256",
      "doorstop_config.ini": "sha256",
      ".doorstop_version": "sha256",
      "changelog.txt": "sha256"
    }
  },
  "game": { "builds": ["20236533"] },
  "launcher": { "min": "0.1.0" }
}
```

Every block is required. A launcher refuses the whole manifest, and treats the check as failed, when any rule below is broken:

- `schema`: a number the launcher knows. It is `1`.
- `version`: exactly `major.minor.patch`.
- `mod.url`, `bepinex.url`: https, and under `https://github.com/Shironex/oa-mod/releases/download/` (`DOWNLOAD_PREFIX` in `crates/core/src/manifest.rs`). The permanent address of a release, never the moving "latest" one. No query string.
- `size`, `sha256`: checked after the download. A size of 0 is refused. A checksum is 64 hex characters; the scripts write lowercase.
- `bepinex.label`: display text of at most 64 characters on one line. It is never compared as a version.
- `bepinex.probe_sha256`: how a copy of BepInEx on disk is recognised as this build.
- `bepinex.root_files`: a checksum for exactly the four names above. The names are constants of the launcher; a fifth name or a missing one refuses the manifest.
- `game.builds`: Steam build ids (digits) the mod was tried with. Another build shows a warning and never blocks. The list may be empty.
- `launcher.min`: see below.
- `notes`: one line shown in the window. Optional.
- `published`: written for people, not read by the launcher. Optional.

Keys the launcher does not know are ignored, so a later release may add one without locking out older launchers.

### `launcher.min`

A launcher older than `launcher.min` does not install the release and says so. What is installed keeps working. It is the one tool for a change that old launchers cannot read: raise it in the same release that needs the newer launcher. The scripts write `0.1.0` unless `--launcher-min` says otherwise.

### `bepinex.json`, the input of the scripts

The mod repository keeps one file that describes the BepInEx build it needs. `build-manifest.mjs --bepinex` copies it into the `bepinex` block and refuses a file without any of these fields:

```json
{
  "label": "6.0.0-be.788",
  "url": "https://github.com/Shironex/oa-mod/releases/download/bepinex-6.0.0-be.788/BepInEx-Unity.IL2CPP-win-x64-6.0.0-be.788.zip",
  "size": 34336405,
  "sha256": "f4cc496bd098a0df4164b81e3737297707f13a47c2478dba2f60eefab784817a",
  "probe_sha256": "...",
  "root_files": {
    "winhttp.dll": "...",
    "doorstop_config.ini": "...",
    ".doorstop_version": "...",
    "changelog.txt": "..."
  }
}
```

`url` is the mirror in the mod repository, not the origin. Other keys (an `origin` address, for example) are ignored. Checksums are lowercase here.

### The BepInEx zip

The launcher unpacks the zip into its own staging folder and refuses it unless:

- its top level holds nothing but `BepInEx/`, `dotnet/` and the four root files, and all four are there;
- every file in `BepInEx/` is in `BepInEx/core/` (that folder is the only one of BepInEx the launcher moves into the game folder, so anything else would be dropped);
- `BepInEx/core/BepInEx.Unity.IL2CPP.dll` hashes to `probe_sha256` and each root file to its entry in `root_files`;
- no entry is absolute, has a drive letter or a `..` part, or is a symbolic link;
- no part of an entry name is one Windows would rewrite: a trailing dot or space, or a device name (`CON`, `PRN`, `AUX`, `NUL`, `COM0` to `COM9`, `LPT0` to `LPT9`, with or without an extension);
- it holds at most 20 000 entries and unpacks to at most 2 GB, counted both by the sizes it declares and by the bytes that are written.

## The `.sig` format

A `.sig` file is what the Tauri CLI's `signer sign` writes: the base64 of a minisign signature text, four lines.

```text
untrusted comment: signature from tauri secret key
<base64 of the signature: the algorithm "ED", the key id, the Ed25519 signature>
trusted comment: timestamp:1791390998	file:manifest.json
<base64 of the signature over the signature of line 2 and the trusted comment>
```

- The signature is prehashed (the `ED` kind: Ed25519 over the BLAKE2b hash of the file). The older kind that signs the file directly is refused. The Tauri CLI has never written it.
- It is over the exact bytes of the file. Nothing may reformat `manifest.json` after it is signed, and the launcher checks the signature before it parses a byte.
- The fields of the trusted comment are separated by tabs.
- A public key has the same wrapping: the base64 of the two lines of a minisign `.pub` file.

`scripts/sign-file.mjs` makes a `.sig`. It takes the key as a file and the password from the environment variable `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, never as an argument.

A manifest whose signature is missing or wrong is a failed check: nothing is installed from it, and what is installed keeps working.

## The keys

Two release keys, both minisign keys made by `pnpm tauri signer generate`. There are no spare keys: if one is lost, everyone installs a new launcher by hand.

| Key        | Public half is in                                       | Signs                                |
| ---------- | ------------------------------------------------------- | ------------------------------------ |
| `manifest` | `RELEASE_KEYS[0]` in `crates/core/src/signature.rs`     | `manifest.json` of every mod release |
| `updater`  | `plugins.updater.pubkey` in `src-tauri/tauri.conf.json` | every launcher installer             |

**The development key** (`dev-keys/dev.key`, public half `dev-keys/dev.key.pub`) is not one of them. Its private half is committed with an empty password on purpose, so it proves nothing. A debug build of the launcher trusts it in addition to the manifest key, for local test manifests; in a release build the constant does not exist (`#[cfg(debug_assertions)]` in `signature.rs`). The scripts sign with it by default for any address that is not on github.com, and refuse to use it by default for one that is.

**The data root override** (`OA_MOD_LAUNCHER_ROOT`) follows the same rule. A debug build keeps `state.json`, `staging/`, `cache/` and `backups/` in the folder the variable names. A release build does not read the variable and always uses `%LOCALAPPDATA%\OaModLauncher`, so nothing in the environment can redirect what it writes and deletes. Below that root the launcher refuses to write or delete through a junction or a symbolic link: one at `staging/` or inside it stops an install, one at `backups/` stops a full removal, and neither is followed or removed.

## What pins each piece

| Piece                                                     | Pinned by                                                                                                                                       |
| --------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| The bytes the scripts write for a manifest                | `scripts/lib/manifest.test.mjs`: regenerates `scripts/fixtures/contract/manifest.json` from fixed inputs and compares byte for byte             |
| That the launcher reads exactly that file                 | `crates/core/tests/contract.rs`: parses the same fixture with `Manifest::parse` and compares the values                                         |
| Manifest rules (schema, addresses, checksums, root files) | the unit tests in `crates/core/src/manifest.rs`                                                                                                 |
| The `bepinex.json` input and `--game-builds`              | `scripts/lib/manifest.test.mjs`, `scripts/build-manifest.test.mjs`                                                                              |
| The `.sig` format as the real CLI writes it               | `a_signature_written_by_the_tauri_cli_is_accepted` in `signature.rs`, on `crates/core/tests/data/signed-by-dev-key.json` (never edit that file) |
| Signing through the scripts                               | `scripts/lib/sign.test.mjs`, `scripts/build-manifest.test.mjs`                                                                                  |
| The release key and that the dev key is not one           | `release_keys_are_real` and `a_release_build_trusts_the_release_keys_only` in `signature.rs`                                                    |

To change the format on purpose: change the writer, run `UPDATE_FIXTURES=1 pnpm test:scripts` (PowerShell: `$env:UPDATE_FIXTURES = '1'` first), then make `crates/core/tests/contract.rs` and the parser agree, and think about `launcher.min`.

## What the mod's release workflow uses

All of these work from another working directory, with explicit arguments. The usage is in the comment at the top of each file.

| Tool                                                                                                                                           | Use                                                                                                                                                                                                                 |
| ---------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `scripts/build-manifest.mjs`                                                                                                                   | Writes `manifest.json` for one built DLL and signs it. For a github.com address it needs `--sign-key` or `--no-sign`. `--game-builds` takes ids separated by commas or white space, so `"$(cat buildid.txt)"` works |
| `scripts/sign-file.mjs`                                                                                                                        | Signs one file                                                                                                                                                                                                      |
| `cargo run --locked -p oa-mod-launcher-core --example verify_manifest -- <manifest.json> --mod <OaMod-x.y.z.dll>`, started inside the checkout | Checks `manifest.json` against the release key, parses it as the launcher does, and compares the size and the SHA-256 of the DLL with the manifest. The gate before a publish                                       |

The scripts need Node 22 and no `pnpm install`, except signing, which starts the Tauri CLI from this repository's `node_modules`. `rust-toolchain.toml` pins the Rust version for commands started inside this repository, so the example is run with the checkout as the working directory; a `cargo run --manifest-path` from another directory also works, with that directory's toolchain. The exact commands are in [development.md](development.md#what-the-mod-repositorys-release-runs-from-here).
