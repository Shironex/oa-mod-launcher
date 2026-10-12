// Run with: pnpm test:scripts
//
// Runs build-manifest.mjs the way the mod's release workflow does: from
// another working directory, with explicit arguments.

import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { ROOT_FILES } from './lib/manifest.mjs';
import { findTauriCli, isBase64 } from './lib/sign.mjs';

const script = join(dirname(fileURLToPath(import.meta.url)), 'build-manifest.mjs');
const hashOf = text => createHash('sha256').update(text).digest('hex');

function workspace() {
  const directory = mkdtempSync(join(tmpdir(), 'oa-build-manifest-'));
  writeFileSync(join(directory, 'OaMod.dll'), 'MZ a built mod');
  writeFileSync(
    join(directory, 'bepinex.json'),
    JSON.stringify({
      label: '6.0.0-be.788',
      url: 'https://github.com/Shironex/oa-mod/releases/download/bepinex-6.0.0-be.788/BepInEx.zip',
      size: 100,
      sha256: hashOf('zip'),
      probe_sha256: hashOf('probe'),
      root_files: Object.fromEntries(ROOT_FILES.map(name => [name, hashOf(name)])),
    })
  );
  writeFileSync(join(directory, 'CHANGELOG.md'), '## 0.2.0\n\nNotes of the release.\n');
  return directory;
}

function run(cwd, ...args) {
  return spawnSync(process.execPath, [script, ...args], { cwd, encoding: 'utf8' });
}

const COMMON = ['--mod', 'OaMod.dll', '--version', '0.2.0', '--bepinex', 'bepinex.json'];
const RELEASE = 'https://github.com/Shironex/oa-mod/releases/download/v0.2.0/';

test('a release manifest is written unsigned with --no-sign', () => {
  const directory = workspace();
  try {
    const result = run(
      directory,
      ...COMMON,
      ...['--base-url', RELEASE, '--game-builds', '20236533\r\n', '--changelog', 'CHANGELOG.md'],
      '--no-sign'
    );
    assert.equal(result.status, 0, result.stderr);

    const manifest = JSON.parse(readFileSync(join(directory, 'manifest.json'), 'utf8'));
    assert.equal(manifest.version, '0.2.0');
    assert.equal(manifest.mod.url, `${RELEASE}OaMod-0.2.0.dll`);
    assert.equal(manifest.mod.sha256, hashOf('MZ a built mod'));
    assert.equal(manifest.notes, 'Notes of the release.');
    assert.deepEqual(manifest.game.builds, ['20236533']);
    assert.ok(!existsSync(join(directory, 'manifest.json.sig')));
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test('a release address is never signed with the development key by default', () => {
  const directory = workspace();
  try {
    const result = run(directory, ...COMMON, '--base-url', RELEASE);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /--sign-key/);
    assert.ok(!existsSync(join(directory, 'manifest.json')));
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test('missing arguments and a bepinex.json without the probe are refused', () => {
  const directory = workspace();
  try {
    assert.equal(run(directory, '--version', '0.2.0').status, 2);

    writeFileSync(join(directory, 'bepinex.json'), JSON.stringify({ label: 'x', size: 1 }));
    const result = run(directory, ...COMMON, '--base-url', RELEASE, '--no-sign');
    assert.equal(result.status, 1);
    assert.match(result.stderr, /bepinex\.json/);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

const skip = findTauriCli() ? false : 'the Tauri CLI is not installed: run `pnpm install`';

test('a local manifest is signed with the development key', { skip }, () => {
  const directory = workspace();
  try {
    const result = run(
      directory,
      ...COMMON,
      ...['--base-url', 'http://127.0.0.1:8123/', '--out', 'feed', '--notes', 'By hand.']
    );
    assert.equal(result.status, 0, result.stderr);

    const manifest = JSON.parse(readFileSync(join(directory, 'feed', 'manifest.json'), 'utf8'));
    assert.equal(manifest.notes, 'By hand.');
    assert.ok(isBase64(readFileSync(join(directory, 'feed', 'manifest.json.sig'), 'utf8')));
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
