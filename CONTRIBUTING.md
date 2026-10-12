# Contributing

Thanks for looking. This is a small project that I make alone, for my friends and me. I want to be able to explain every line in it, so I keep it small and plain. Please read the next section before you spend time on a pull request.

## What is welcome

- **Bug reports**, with the launcher version, what you did and what happened. "Copy details" in the window and `launcher.log` (Settings, About, "Open log folder") have what I need.
- **Pull requests that fix a bug.** Open an issue first, so we agree on the fix before you write it.

I may rewrite a pull request, ask you to shrink it, or decline it. This is not about the quality of your work. I need to understand and keep the code. A small pull request that changes one thing has the best chance.

## What is not open to pull requests

- **The wording of the window.** I write the texts myself. A wrong spelling or a broken sentence is a fine bug report.
- **The signing keys, the update addresses, the manifest format and the release workflow.** The keys and the addresses are compiled into every installed launcher and cannot be changed from outside. If you think something there is wrong, report it (privately, if it is a security problem: see [SECURITY.md](SECURITY.md)). In files, this means `.github/workflows/`, `dev-keys/`, `docs/contract.md`, `crates/core/src/signature.rs`, `src-tauri/src/updater.rs`, the `plugins.updater` part of `src-tauri/tauri.conf.json` and the release scripts that the mod repository uses.

## Set up

You need Rust (`rust-toolchain.toml` pins it), Node 22 and pnpm 10, and the WebView2 runtime on Windows. Then:

```sh
pnpm install
pnpm tauri dev
```

[docs/development.md](docs/development.md) has the rest: how to run against a local test server, a throwaway data folder and a fake game folder (do this, so you do not touch your real game), and what each folder does.

pnpm refuses a dependency version younger than 7 days (`minimumReleaseAge` in `pnpm-workspace.yaml`). Do not add a dependency in a bug fix.

## Before you open the pull request

`.github/workflows/ci.yml` runs these on a Windows runner. Run them yourself first:

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

If you changed a command, a type it returns or an event, regenerate `src/bindings.ts` with `UPDATE_BINDINGS=1 cargo test -p oa-mod-launcher` and commit it. A test fails while the file is out of date.

I do not ask for a `CHANGELOG.md` entry in pull requests. I write it when I make a release.

## Commits and pull requests

Conventional commits, always with a scope, in lowercase: `type(scope): summary`. The types are `feat`, `fix`, `chore`, `docs`, `refactor`, `test`, `ci`, `build` and `perf`. The scopes in use are `core`, `shell`, `window`, `updater`, `scripts`, `release`, `repo` and `deps` (`git log --format=%s` shows them). One logical change per commit. The title of the pull request has the same shape.
