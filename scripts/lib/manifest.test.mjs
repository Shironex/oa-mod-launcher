// Run with: pnpm test:scripts

import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  ROOT_FILES,
  bepinexBlock,
  buildManifest,
  manifestText,
  modName,
  notesFromChangelog,
  parseGameBuilds,
} from './manifest.mjs';

const fixtures = join(dirname(fileURLToPath(import.meta.url)), '..', 'fixtures', 'contract');

const hashOf = text => createHash('sha256').update(text).digest('hex');

/** A complete bepinex.json, as the mod repository keeps it. */
const BEPINEX = {
  label: '6.0.0-be.788',
  url: 'https://github.com/Shironex/oa-mod/releases/download/bepinex-6.0.0-be.788/BepInEx-Unity.IL2CPP-win-x64-6.0.0-be.788.zip',
  size: 34336405,
  sha256: 'f4cc496bd098a0df4164b81e3737297707f13a47c2478dba2f60eefab784817a',
  probe_sha256: hashOf('probe'),
  root_files: Object.fromEntries(ROOT_FILES.map(name => [name, hashOf(name)])),
};

const CHANGELOG = `# Changelog

Text before the first version is ignored.

## 0.1.1 (2026-10-27)

- Fixes the scoreboard after a late join

## 0.1.0 (2026-10-20) First release

The first public build.

More text.
`;

function withModFile(run) {
  const directory = mkdtempSync(join(tmpdir(), 'oa-manifest-'));
  try {
    const mod = join(directory, 'OaMod.dll');
    writeFileSync(mod, 'MZ fixture mod');
    return run(mod);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

/** The manifest the contract fixture holds: fixed inputs, so fixed bytes. */
function contractManifest(mod) {
  return buildManifest({
    mod,
    version: '0.1.0',
    baseUrl: 'https://github.com/Shironex/oa-mod/releases/download/v0.1.0/',
    bepinex: { ...BEPINEX, origin: 'ignored extra field' },
    gameBuilds: ['20236533', '20240000'],
    notes: notesFromChangelog(CHANGELOG, '0.1.0'),
    launcherMin: '0.1.0',
    published: '2026-10-20T18:00:00Z',
  });
}

test('the manifest names the mod by its release name, size and checksum', () => {
  withModFile(mod => {
    const manifest = contractManifest(mod);
    assert.equal(manifest.schema, 1);
    assert.equal(
      manifest.mod.url,
      'https://github.com/Shironex/oa-mod/releases/download/v0.1.0/OaMod-0.1.0.dll'
    );
    assert.equal(manifest.mod.size, 14);
    assert.equal(manifest.mod.sha256, hashOf('MZ fixture mod'));
    assert.equal(manifest.notes, 'The first public build.');
    assert.deepEqual(manifest.game, { builds: ['20236533', '20240000'] });
    assert.deepEqual(manifest.launcher, { min: '0.1.0' });
    assert.deepEqual(Object.keys(manifest.bepinex), [
      'label',
      'url',
      'size',
      'sha256',
      'probe_sha256',
      'root_files',
    ]);
    assert.equal(modName('1.2.3'), 'OaMod-1.2.3.dll');
  });
});

test('a base url without a trailing slash gives the same address', () => {
  withModFile(mod => {
    const manifest = buildManifest({
      mod,
      version: '0.1.0',
      baseUrl: 'http://127.0.0.1:8123/files',
      bepinex: BEPINEX,
    });
    assert.equal(manifest.mod.url, 'http://127.0.0.1:8123/files/OaMod-0.1.0.dll');
    assert.deepEqual(manifest.game.builds, []);
    assert.ok(!('notes' in manifest));
    assert.match(manifest.published, /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$/);
  });
});

test('bad versions and an empty mod file are refused', () => {
  withModFile(mod => {
    const options = { mod, version: '0.1.0', baseUrl: 'http://127.0.0.1/', bepinex: BEPINEX };
    assert.throws(() => buildManifest({ ...options, version: 'v0.1.0' }), /not a version/);
    assert.throws(() => buildManifest({ ...options, launcherMin: '1' }), /launcher version/);
    writeFileSync(mod, '');
    assert.throws(() => buildManifest(options), /empty/);
  });
});

test('a bepinex.json with a missing or malformed field is refused', () => {
  assert.deepEqual(bepinexBlock(BEPINEX), BEPINEX);
  for (const [field, value] of [
    ['label', ''],
    ['label', 'two\nlines'],
    ['url', 'http://example.com/BepInEx.zip'],
    ['url', 'http://127.0.0.1.example.com/BepInEx.zip'],
    ['url', undefined],
    ['size', 0],
    ['size', '34336405'],
    ['sha256', 'abc'],
    ['sha256', BEPINEX.sha256.toUpperCase()],
    ['probe_sha256', undefined],
    ['root_files', undefined],
    ['root_files', { ...BEPINEX.root_files, 'version.dll': hashOf('x') }],
    ['root_files', { ...BEPINEX.root_files, 'winhttp.dll': 'abc' }],
  ]) {
    assert.throws(
      () => bepinexBlock({ ...BEPINEX, [field]: value }),
      new RegExp(`"${field}"`),
      `${field} = ${JSON.stringify(value)}`
    );
  }
  assert.throws(() => bepinexBlock(undefined), /label/);
});

test('a zip on this computer may be named over plain http, for a local test feed', () => {
  for (const url of ['http://127.0.0.1:8123/BepInEx.zip', 'http://localhost/BepInEx.zip']) {
    assert.equal(bepinexBlock({ ...BEPINEX, url }).url, url);
  }
});

test('game builds are split on commas and white space, a stray CR included', () => {
  assert.deepEqual(parseGameBuilds('20236533\r\n'), ['20236533']);
  assert.deepEqual(parseGameBuilds('1, 2  3'), ['1', '2', '3']);
  assert.deepEqual(parseGameBuilds(undefined), []);
  assert.deepEqual(parseGameBuilds(''), []);
  assert.throws(() => parseGameBuilds('latest'), /build id/);
});

test('the notes are the first line under the heading of the version', () => {
  assert.equal(notesFromChangelog(CHANGELOG, '0.1.0'), 'The first public build.');
  assert.equal(notesFromChangelog(CHANGELOG, '0.1.1'), 'Fixes the scoreboard after a late join');
  assert.equal(notesFromChangelog(CHANGELOG, '0.2.0'), undefined);
  assert.equal(notesFromChangelog('## 0.3.0\n\n## 0.2.0\n\nOlder\n', '0.3.0'), undefined);
  assert.equal(notesFromChangelog('', '0.1.0'), undefined);
});

test('the text ends with one line end and reads back to the same value', () => {
  const text = manifestText({ schema: 1, game: { builds: [] } });
  assert.ok(text.endsWith('}\n') && !text.endsWith('\n\n'));
  assert.ok(!text.includes('\r'));
  assert.deepEqual(JSON.parse(text), { schema: 1, game: { builds: [] } });
});

// The contract with the reader: crates/core/tests/contract.rs parses the same
// file with the launcher's own parser. UPDATE_FIXTURES=1 rewrites it after a
// change made on purpose.
test('the contract fixture is what the writer writes today', () => {
  withModFile(mod => {
    const text = manifestText(contractManifest(mod));
    const path = join(fixtures, 'manifest.json');
    if (process.env.UPDATE_FIXTURES === '1' || !existsSync(path)) {
      mkdirSync(fixtures, { recursive: true });
      writeFileSync(path, text);
    }
    assert.equal(
      readFileSync(path, 'utf8'),
      text,
      'the manifest format changed: see docs/contract.md before running with UPDATE_FIXTURES=1'
    );
  });
});
