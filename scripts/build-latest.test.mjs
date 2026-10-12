// Run with: pnpm test:scripts
//
// Runs the script the way the publish job of the release workflow does: on a
// folder that holds the downloaded installer and the signature made for it.

import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const script = join(dirname(fileURLToPath(import.meta.url)), 'build-latest.mjs');
const INSTALLER = 'OaModLauncher-0.1.2-windows-x64-setup.exe';
const SIGNATURE = 'dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZQ==';

function buildLatest(directory) {
  return spawnSync(
    process.execPath,
    [script, '--bundle', 'release', '--version', '0.1.2', '--repo', 'a/b', '--out', 'publish'],
    { cwd: directory, encoding: 'utf8' }
  );
}

test('one installer and its signature become the two assets of a release', () => {
  const directory = mkdtempSync(join(tmpdir(), 'oa-latest-'));
  try {
    mkdirSync(join(directory, 'release'));
    writeFileSync(join(directory, 'release', INSTALLER), 'fake installer');
    writeFileSync(join(directory, 'release', `${INSTALLER}.sig`), `${SIGNATURE}\n`);

    const result = buildLatest(directory);
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(readdirSync(join(directory, 'publish')).sort(), [INSTALLER, 'latest.json']);
    assert.equal(readFileSync(join(directory, 'publish', INSTALLER), 'utf8'), 'fake installer');
    const latest = JSON.parse(readFileSync(join(directory, 'publish', 'latest.json'), 'utf8'));
    assert.equal(latest.version, '0.1.2');
    assert.deepEqual(latest.platforms, {
      'windows-x86_64': {
        signature: SIGNATURE,
        url: `https://github.com/a/b/releases/download/v0.1.2/${INSTALLER}`,
      },
    });
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test('a missing signature or a second installer stops it', () => {
  const directory = mkdtempSync(join(tmpdir(), 'oa-latest-'));
  try {
    mkdirSync(join(directory, 'release'));
    writeFileSync(join(directory, 'release', INSTALLER), 'fake installer');
    assert.match(buildLatest(directory).stderr, /no signature file/);

    writeFileSync(join(directory, 'release', `${INSTALLER}.sig`), SIGNATURE);
    writeFileSync(join(directory, 'release', 'Other_0.1.2_x64-setup.exe'), 'second');
    const result = buildLatest(directory);
    assert.equal(result.status, 1);
    assert.match(result.stderr, /more than one/);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
