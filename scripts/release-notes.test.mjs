// Run with: pnpm test:scripts
//
// Runs the script on the real CHANGELOG.md and the real footer, from another
// working directory, the way the release workflow depends on it.

import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const script = join(root, 'scripts', 'release-notes.mjs');
const footer = join(root, '.github', 'release-footer.md');

function releaseNotes(...extra) {
  return spawnSync(process.execPath, [script, ...extra], { cwd: tmpdir(), encoding: 'utf8' });
}

test('the notes of a released version are its changelog section', () => {
  const result = releaseNotes('--version', '0.1.0');
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /^The first release: install OA Mod for Golf With Your Friends/);
  assert.ok(!result.stdout.includes('##'), 'no heading and no other version');
});

test('the footer follows the section and names the installer of that version', () => {
  const result = releaseNotes('--version', '0.1.0', '--footer', footer);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /^The first release[^]*\n- [^\n]*\n\n## How to install\n/);
  assert.ok(result.stdout.includes('`OaModLauncher-0.1.0-windows-x64-setup.exe`'));
  assert.ok(!result.stdout.includes('{version}'));
});

test('a version without a changelog section fails', () => {
  const result = releaseNotes('--version', '99.0.0');
  assert.equal(result.status, 1);
  assert.match(result.stderr, /no section for version 99\.0\.0/);
  assert.equal(result.stdout, '');
});
