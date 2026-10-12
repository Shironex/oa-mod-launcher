// Writes latest.json and the installer copy for the launcher's own update.
//
//   node scripts/build-latest.mjs --bundle <folder with the NSIS output of tauri build> \
//     --version 0.1.0 --repo Shironex/oa-mod-launcher --out <folder> [--notes <text or @file>]
//
// `--bundle` holds exactly one `*-setup.exe` and its `.sig`. The installer is
// copied into `--out` as OaModLauncher-<version>-windows-x64-setup.exe and
// latest.json is written next to it. See docs/development.md.

import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  writeFileSync,
} from 'node:fs';
import { join, resolve } from 'node:path';
import { argument, isVersion } from './lib/manifest.mjs';
import { buildLatest, installerAssetName } from './lib/latest.mjs';

const bundle = argument('bundle');
const version = argument('version');
const repo = argument('repo');
const out = argument('out');
if (!bundle || !version || !repo || !out) {
  console.error(
    'usage: build-latest.mjs --bundle <folder> --version <x.y.z> --repo <owner/name> --out <folder>'
  );
  process.exit(2);
}
if (!isVersion(version)) {
  console.error(`not a version: ${version}`);
  process.exit(2);
}

function fail(message) {
  console.error(message);
  process.exit(1);
}

const bundleDirectory = resolve(bundle);
if (!existsSync(bundleDirectory)) fail(`the bundle folder does not exist: ${bundleDirectory}`);
const installers = readdirSync(bundleDirectory).filter(name => name.endsWith('-setup.exe'));
if (installers.length === 0) fail(`no *-setup.exe in ${bundleDirectory}`);
if (installers.length > 1) {
  fail(`more than one *-setup.exe in ${bundleDirectory}: ${installers.join(', ')}`);
}
const installer = join(bundleDirectory, installers[0]);
const signaturePath = `${installer}.sig`;
if (!existsSync(signaturePath)) fail(`the installer has no signature file: ${signaturePath}`);

let notes = argument('notes', '');
if (notes.startsWith('@')) {
  const notesPath = resolve(notes.slice(1));
  if (!existsSync(notesPath)) fail(`the notes file does not exist: ${notesPath}`);
  notes = readFileSync(notesPath, 'utf8').trim();
}

let latest;
try {
  latest = buildLatest({ version, repo, notes, signature: readFileSync(signaturePath, 'utf8') });
} catch (error) {
  fail(error.message);
}

const outDirectory = resolve(out);
mkdirSync(outDirectory, { recursive: true });
copyFileSync(installer, join(outDirectory, installerAssetName(version)));
writeFileSync(join(outDirectory, 'latest.json'), `${JSON.stringify(latest, null, 2)}\n`);
console.log(`${installerAssetName(version)} and latest.json written to ${outDirectory}`);
