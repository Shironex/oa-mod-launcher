// Run with: pnpm test:scripts

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { buildLatest, changelogSection, installerAssetName, installerUrl } from './latest.mjs';

const SIGNATURE = 'dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZQ==';

test('the notes of a version are its changelog section and nothing else', () => {
  const changelog = [
    '# Changelog',
    '',
    '## 0.1.10 (2026-11-01)',
    '',
    'Ten.',
    '',
    '## 0.1.1 (2026-10-07) Update test',
    '',
    'First line.',
    '',
    '### Fixes',
    '',
    '- A fix',
    '',
    '## 0.1.0',
    '',
    '## 0.0.9',
    'Old.',
  ].join('\r\n');

  assert.equal(changelogSection(changelog, '0.1.1'), 'First line.\n\n### Fixes\n\n- A fix');
  assert.equal(changelogSection(changelog, '0.1.10'), 'Ten.');
  assert.equal(changelogSection(changelog, '0.0.9'), 'Old.');
  assert.throws(() => changelogSection(changelog, '0.1.2'), /no section for version 0\.1\.2/);
  assert.throws(() => changelogSection(changelog, '0.1.0'), /empty/);
});

test('latest.json has the shape the updater reads, with only the Windows platform', () => {
  const latest = buildLatest({
    version: '0.1.0',
    repo: 'Shironex/oa-mod-launcher',
    signature: SIGNATURE,
    notes: 'First launcher.',
    now: new Date('2026-12-05T18:00:00Z'),
  });
  assert.deepEqual(latest, {
    version: '0.1.0',
    notes: 'First launcher.',
    pub_date: '2026-12-05T18:00:00.000Z',
    platforms: {
      'windows-x86_64': {
        signature: SIGNATURE,
        url: 'https://github.com/Shironex/oa-mod-launcher/releases/download/v0.1.0/OaModLauncher-0.1.0-windows-x64-setup.exe',
      },
    },
  });
});

test('the signature is taken verbatim, only trimmed', () => {
  const latest = buildLatest({
    version: '0.1.0',
    repo: 'a/b',
    signature: `  ${SIGNATURE}\r\n`,
  });
  assert.equal(latest.platforms['windows-x86_64'].signature, SIGNATURE);
  assert.throws(() => buildLatest({ version: '0.1.0', repo: 'a/b', signature: ' \n' }), /empty/);
});

test('the publication date parses as an RFC 3339 UTC time', () => {
  const { pub_date } = buildLatest({ version: '1.2.3', repo: 'a/b', signature: SIGNATURE });
  assert.match(pub_date, /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?Z$/);
  assert.ok(!Number.isNaN(Date.parse(pub_date)));
});

test('the installer name has no spaces and the version is checked', () => {
  assert.equal(installerAssetName('0.1.0'), 'OaModLauncher-0.1.0-windows-x64-setup.exe');
  assert.ok(!/\s/.test(installerAssetName('10.20.30')));
  for (const bad of ['1.0', 'v1.0.0', '1.0.0-beta', '', 'a b']) {
    assert.throws(() => installerAssetName(bad), /not a version/);
    assert.throws(
      () => buildLatest({ version: bad, repo: 'a/b', signature: SIGNATURE }),
      /not a version/
    );
  }
});

test('the repository must be owner/name', () => {
  assert.equal(
    installerUrl('Shironex/oa-mod-launcher', '0.1.0'),
    'https://github.com/Shironex/oa-mod-launcher/releases/download/v0.1.0/OaModLauncher-0.1.0-windows-x64-setup.exe'
  );
  for (const bad of ['oa-mod-launcher', 'a/b/c', 'a b/c', '']) {
    assert.throws(() => installerUrl(bad, '0.1.0'), /not a repository/);
  }
});
