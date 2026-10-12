// Run with: pnpm test:scripts
//
// Only the pure parts: reading licence expressions, picking the linked crates
// out of `cargo metadata` output, grouping equal texts. The script itself is
// run by hand (it starts cargo and reads node_modules).

import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import {
  findPackageDirectory,
  flagged,
  groupByText,
  licenseFiles,
  licenseIds,
  linkedPackages,
  nonPermissiveIds,
  parseLicense,
  renderSection,
  standardTextsFor,
  withoutEmail,
} from './build-launcher-notices.mjs';

test('licence expressions are read, also in the old slash form', () => {
  assert.deepEqual(licenseIds('MIT OR Apache-2.0'), ['MIT', 'Apache-2.0']);
  assert.deepEqual(licenseIds('MIT/Apache-2.0'), ['MIT', 'Apache-2.0']);
  assert.deepEqual(licenseIds('Apache-2.0 / MIT'), ['Apache-2.0', 'MIT']);
  assert.deepEqual(licenseIds('(MIT OR Apache-2.0) AND Unicode-3.0'), [
    'MIT',
    'Apache-2.0',
    'Unicode-3.0',
  ]);
  assert.deepEqual(licenseIds('Apache-2.0 WITH LLVM-exception OR MIT'), ['Apache-2.0', 'MIT']);
  assert.deepEqual(parseLicense('A AND B OR C'), {
    op: 'or',
    items: [{ op: 'and', items: [{ id: 'A' }, { id: 'B' }] }, { id: 'C' }],
  });
  assert.throws(() => parseLicense('MIT OR'));
  assert.throws(() => parseLicense('(MIT'));
});

test('a licence is permissive when permissive terms alone are enough', () => {
  assert.deepEqual(nonPermissiveIds('MIT'), []);
  assert.deepEqual(nonPermissiveIds('MPL-2.0 OR MIT'), []);
  assert.deepEqual(nonPermissiveIds('MIT AND MPL-2.0'), ['MPL-2.0']);
  assert.deepEqual(nonPermissiveIds('GPL-3.0-only OR LGPL-2.1-only'), [
    'GPL-3.0-only',
    'LGPL-2.1-only',
  ]);
  assert.deepEqual(nonPermissiveIds('ISC AND (Apache-2.0 OR ISC)'), []);
  assert.deepEqual(nonPermissiveIds('Something-Custom'), ['Something-Custom']);
});

test('packages without a licence field are flagged as unknown', () => {
  const entries = [
    { name: 'a', license: 'MIT' },
    { name: 'b', license: '' },
    { name: 'c', license: 'AGPL-3.0-only' },
  ];
  assert.deepEqual(
    flagged(entries).map(item => [item.entry.name, item.ids]),
    [
      ['b', ['unknown']],
      ['c', ['AGPL-3.0-only']],
    ]
  );
});

test('only normal dependencies of the workspace are linked', () => {
  const metadata = {
    workspace_members: ['app'],
    packages: ['app', 'lib', 'macro', 'builder', 'tooling', 'deep'].map(id => ({ id, name: id })),
    resolve: {
      nodes: [
        {
          id: 'app',
          deps: [
            { pkg: 'lib', dep_kinds: [{ kind: null }] },
            { pkg: 'macro', dep_kinds: [{ kind: null }] },
            { pkg: 'builder', dep_kinds: [{ kind: 'build' }] },
            { pkg: 'tooling', dep_kinds: [{ kind: 'dev' }] },
          ],
        },
        { id: 'lib', deps: [{ pkg: 'deep', dep_kinds: [{ kind: 'dev' }, { kind: null }] }] },
        { id: 'macro', deps: [] },
        { id: 'builder', deps: [] },
        { id: 'tooling', deps: [] },
        { id: 'deep', deps: [{ pkg: 'app', dep_kinds: [{ kind: null }] }] },
      ],
    },
  };
  assert.deepEqual([...linkedPackages(metadata).keys()].sort(), ['app', 'deep', 'lib', 'macro']);
});

test('equal licence texts are printed once with every package that uses them', () => {
  const a = { name: 'a', version: '1.0.0', license: 'MIT', texts: ['MIT text'] };
  const b = { name: 'b', version: '2.0.0', license: 'MIT', texts: ['MIT text', 'Other text'] };
  const c = { name: 'c', version: '3.0.0', license: 'MIT', texts: [] };
  const { groups, withoutFiles } = groupByText([a, b, c]);
  assert.deepEqual(
    groups.map(group => [group.text, group.entries.map(entry => entry.name)]),
    [
      ['MIT text', ['a', 'b']],
      ['Other text', ['b']],
    ]
  );
  assert.deepEqual(withoutFiles, [c]);
});

test('a package without a licence file gets the standard text, once', () => {
  const entries = [
    { name: 'x', version: '1.0.0', license: 'MIT', authors: ['Ann <ann@example.com>'], texts: [] },
    { name: 'y', version: '1.0.0', license: 'MIT/Apache-2.0', authors: [], texts: [] },
  ];
  const apache = 'Apache License\n Version 2.0, January 2004 ...';
  const texts = standardTextsFor(entries, [apache]);
  assert.deepEqual([...texts.keys()].sort(), ['Apache-2.0', 'MIT']);
  assert.equal(texts.get('Apache-2.0'), apache);

  const section = renderSection('Things', 'Intro.', entries, [apache]);
  assert.equal(section.split('Standard text of MIT').length, 2);
  assert.ok(section.includes('x 1.0.0, MIT, by Ann'));
  assert.ok(!section.includes('ann@example.com'));
});

test('a package with no text at all is an error, not a silent gap', () => {
  const entry = { name: 'odd', version: '1.0.0', license: 'Custom-1.0', authors: [], texts: [] };
  assert.throws(() => standardTextsFor([entry], []), /odd 1.0.0/);
  assert.throws(
    () => standardTextsFor([{ ...entry, license: 'Apache-2.0' }], ['no such text']),
    /odd 1.0.0/
  );
});

test('author names lose their mail address', () => {
  assert.equal(withoutEmail('Ann Lee <ann@example.com>'), 'Ann Lee');
  assert.equal(withoutEmail('Ann Lee (https://example.com)'), 'Ann Lee');
  assert.equal(withoutEmail('Ann Lee'), 'Ann Lee');
});

test('licence files are found by name and read with unix line ends', () => {
  const root = mkdtempSync(join(tmpdir(), 'notices-'));
  try {
    writeFileSync(join(root, 'LICENSE-MIT'), '\uFEFFMIT\r\ntext\r\n');
    writeFileSync(join(root, 'NOTICE'), 'a notice');
    writeFileSync(join(root, 'README.md'), 'not a licence');
    mkdirSync(join(root, 'LICENSES'));
    assert.deepEqual(licenseFiles(root), ['MIT\ntext', 'a notice']);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test('a package is found in the node_modules of a parent folder', () => {
  const root = mkdtempSync(join(tmpdir(), 'notices-'));
  try {
    const inner = join(root, 'node_modules', 'outer', 'lib');
    mkdirSync(join(root, 'node_modules', '@scope', 'pkg'), { recursive: true });
    mkdirSync(inner, { recursive: true });
    writeFileSync(join(root, 'node_modules', '@scope', 'pkg', 'package.json'), '{}');
    assert.ok(findPackageDirectory('@scope/pkg', inner)?.endsWith('pkg'));
    assert.equal(findPackageDirectory('missing', inner), null);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
