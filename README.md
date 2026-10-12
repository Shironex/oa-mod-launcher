# OA Mod Launcher

A small Windows program that installs [OA Mod](https://github.com/Shironex/oa-mod) for _Golf With Your Friends_, keeps it up to date, and removes it again. I made it so my friends do not have to copy files into the game folder by hand before a tournament.

## Please read

- You need your own copy of _Golf With Your Friends_ on Steam. The launcher contains no game code and distributes none.
- It is not affiliated with or endorsed by the game's developers or publisher.
- It comes with no warranty, and I do not promise how long I will keep it working. A game update can break the mod.
- The installer is not signed with a paid certificate, so Windows SmartScreen warns about it. See [Install](#install).

## What it does

- Finds the game through Steam, or you choose the folder.
- **Install** downloads BepInEx (the mod loader) and OA Mod and puts them into the game folder. **Update** replaces only the mod.
- **Play** starts the game through Steam. The first start after an install takes a few minutes while BepInEx prepares the game.
- **Remove mod** puts the game back to how Steam installed it.
- It updates itself.

## Install

1. Download `OaModLauncher-<version>-windows-x64-setup.exe` from the [latest release](https://github.com/Shironex/oa-mod-launcher/releases/latest) and run it.
2. Windows says "Windows protected your PC". Click **More info**, then **Run anyway**.
3. Open the launcher and press **Install**.

Windows 10 or 11, 64 bit. No admin rights needed.

## What it changes on your PC, and how to undo it

| Where                                        | What                                                                                                                            | How to undo it                                            |
| -------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------- |
| The game folder                              | The folders `BepInEx` and `dotnet`, and the files `winhttp.dll`, `doorstop_config.ini`, `.doorstop_version` and `changelog.txt` | **Remove mod** in the launcher, then "Remove everything"  |
| The game folder, by BepInEx itself           | On the first start BepInEx writes its own files into the `BepInEx` folder (interop, cache, config, logs)                        | The same: they are inside the folder that is removed      |
| `%LOCALAPPDATA%\OaModLauncher`               | The launcher's settings, its log, downloads in progress, and backups of other plugins made before a full removal                | Delete the folder after uninstalling                      |
| The launcher itself                          | Installed for your user only                                                                                                    | "Add or remove programs" in Windows                       |
| `%LOCALAPPDATA%\com.shironex.oamod.launcher` | The data of the window (WebView2 keeps a profile per program)                                                                   | The uninstaller offers to delete it, or delete the folder |
| Your temporary folder                        | The new installer, while the launcher updates itself                                                                            | Safe to delete afterwards                                 |

It changes no game file that Steam installed and does not touch your saves. The uninstaller removes the program; the `OaModLauncher` folder stays until you delete it.

A few things worth knowing about the removal:

- "Remove only OA Mod" keeps BepInEx and every other plugin.
- "Remove everything" first copies other people's plugins and their settings to the backups folder, and only then deletes.
- A file that has a BepInEx name but that the launcher does not recognise stays, unless you tick the box that includes it.
- Steam's "Verify integrity of game files" restores game files but does not delete extra ones, so it is no substitute.

Nothing in the game folder is changed while the game runs or while Steam is updating it.

## How it stays safe

- The list of what to download is signed, and the launcher checks the signature before it reads the list. Each download is then checked against the size and SHA-256 the list names.
- The launcher's own update is a signed installer, checked against a key compiled into the launcher.
- The window makes no network request. Everything goes through the Rust side, over https only.

[SECURITY.md](SECURITY.md) says what counts as a vulnerability and where to report one.

## Development

```sh
pnpm install
pnpm tauri dev
```

[docs/development.md](docs/development.md) has the rest: a local test feed, the checks, how a release is made. [docs/contract.md](docs/contract.md) describes the files the launcher and the mod repository share. Changes are listed in [CHANGELOG.md](CHANGELOG.md).

## Licence

MIT, see [LICENSE](LICENSE). Third-party crates, packages and fonts keep their own licences, listed in `THIRD-PARTY-NOTICES.txt`. BepInEx is downloaded at install time and is not part of this repository.
