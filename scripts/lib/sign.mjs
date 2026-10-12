// Signs a file with the Tauri CLI's `signer sign`, which writes `<file>.sig`:
// base64 of a minisign signature over the exact bytes of the file. Used by
// sign-file.mjs and by build-manifest.mjs, so a local test manifest and a
// release are signed by the same code. Node builtins only.
//
// The CLI is run through node, not through pnpm: `pnpm.cmd` cannot be started
// without a shell on Windows, and the shell would be one more place for the
// password to leak.

import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync, realpathSync, rmSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const launcherRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');

/** The development key: empty password, committed on purpose, local files only. */
export const DEV_KEY = join(launcherRoot, 'dev-keys', 'dev.key');

/** The variable the installed CLI (2.12) reads the key password from. */
export const PASSWORD_VARIABLE = 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD';

/** Variables that would make the CLI load a different key than the one asked for. */
const KEY_VARIABLES = ['TAURI_SIGNING_PRIVATE_KEY', 'TAURI_SIGNING_PRIVATE_KEY_PATH'];

function samePath(a, b) {
  const left = realpathSync(a);
  const right = realpathSync(b);
  return process.platform === 'win32' ? left.toLowerCase() === right.toLowerCase() : left === right;
}

/** Whether `keyPath` is the development key inside the repository. */
export function isDevKey(keyPath) {
  return existsSync(keyPath) && existsSync(DEV_KEY) && samePath(keyPath, DEV_KEY);
}

/** Standard or URL-safe base64 with nothing but its alphabet and padding. */
export function isBase64(text) {
  const trimmed = text.trim();
  return trimmed.length > 0 && trimmed.length % 4 === 0 && /^[A-Za-z0-9+/_-]+={0,2}$/.test(trimmed);
}

/** The path of the Tauri CLI entry point, or null when it is not installed. */
export function findTauriCli() {
  try {
    const require = createRequire(join(launcherRoot, 'package.json'));
    return join(dirname(require.resolve('@tauri-apps/cli/package.json')), 'tauri.js');
  } catch {
    return null;
  }
}

/**
 * Signs `file` with the key in `key` and returns the path of the `.sig` file.
 * Throws an Error with a plain sentence on any failure.
 *
 * @param {{ file: string, key: string, passwordEnv?: string }} options
 *   The password is read from the environment variable `passwordEnv` (default
 *   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`) and reaches the CLI only through the
 *   environment of the child process. It may be empty or missing only for the
 *   development key.
 */
export function signFile({ file, key, passwordEnv = PASSWORD_VARIABLE }) {
  const target = resolve(file);
  const keyPath = resolve(key);
  if (!existsSync(target)) throw new Error(`the file to sign does not exist: ${target}`);
  if (!existsSync(keyPath)) throw new Error(`the key does not exist: ${keyPath}`);

  const password = process.env[passwordEnv] ?? '';
  if (!password && !isDevKey(keyPath)) {
    throw new Error(
      `the key ${keyPath} needs its password: set the environment variable ${passwordEnv}.`
    );
  }

  const cli = findTauriCli();
  if (!cli) throw new Error('the Tauri CLI is not installed: run `pnpm install` first.');
  const env = { ...process.env };
  for (const name of KEY_VARIABLES) delete env[name];
  env[PASSWORD_VARIABLE] = password;

  const signature = `${target}.sig`;
  rmSync(signature, { force: true });
  const result = spawnSync(process.execPath, [cli, 'signer', 'sign', '-f', keyPath, target], {
    env,
    encoding: 'utf8',
    // No terminal: a CLI that decided to ask for a password would wait forever.
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  if (result.error) throw new Error(`could not run the Tauri CLI: ${result.error.message}`);
  if (result.status !== 0) {
    const detail = `${result.stderr ?? ''}${result.stdout ?? ''}`.trim();
    throw new Error(`signing failed (exit ${result.status}): ${detail || 'no output'}`);
  }

  if (!existsSync(signature)) throw new Error(`the CLI did not write ${signature}.`);
  const text = readFileSync(signature, 'utf8');
  if (!text.trim()) throw new Error(`the signature file is empty: ${signature}`);
  if (!isBase64(text)) throw new Error(`the signature file is not base64: ${signature}`);
  return signature;
}

/** Whether `baseUrl` points at a GitHub release, that is, at a real release. */
export function isGithubUrl(baseUrl) {
  try {
    const host = new URL(baseUrl).hostname.toLowerCase();
    return host === 'github.com' || host.endsWith('.github.com');
  } catch {
    return false;
  }
}

/**
 * The key a manifest written for `baseUrl` is signed with, or null for no
 * signing. The development key is the default for local use only: for a
 * github.com address a key must be given with `--sign-key`, or signing
 * switched off with `--no-sign`.
 *
 * @param {{ baseUrl: string, signKey?: string, noSign?: boolean }} options
 */
export function manifestSigningKey({ baseUrl, signKey, noSign }) {
  if (noSign) return null;
  if (signKey) return signKey;
  if (isGithubUrl(baseUrl)) {
    throw new Error(
      `${baseUrl} is a GitHub release: pass --sign-key <your key> (the password goes in the environment), or --no-sign.`
    );
  }
  return DEV_KEY;
}
