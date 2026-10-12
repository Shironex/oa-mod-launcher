// Assembles THIRD-PARTY-NOTICES.txt for the launcher installer: the Rust crates
// linked into the launcher, the npm packages bundled into its window (the
// fonts are among them) and the licence text of each.
//
//   node scripts/build-launcher-notices.mjs [--out THIRD-PARTY-NOTICES.txt] \
//     [--targets x86_64-pc-windows-msvc]
//
// Run from anywhere; paths are relative to the repository root. Rust: `cargo metadata`
// is asked once per target, and only crates reached through normal
// dependencies of the two workspace crates are listed. Build dependencies,
// development dependencies and procedural macros run on the build machine and
// are not in the installer. The result is the union over the targets, so the
// file is the same whichever machine writes it. npm: the `dependencies` of
// package.json and, below them, the `dependencies` of each package, read from
// node_modules (run `pnpm install` first). Licence texts are copied from the
// source of each package, never typed by hand, except the short standard texts
// used for a package whose source has no licence file. Equal texts are printed
// once, with the packages that use them.
//
// The file is committed: tauri.conf.json lists it under bundle.resources, so a
// build needs it to exist. Run this again after Cargo.lock or pnpm-lock.yaml
// changes. Packages whose licence is not clearly permissive are reported on
// the error output.

import { spawnSync } from 'node:child_process';
import {
  existsSync,
  readdirSync,
  readFileSync,
  realpathSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { argument } from './lib/manifest.mjs';

const launcherRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');

/** The systems the installer is built for: Windows on x64. */
export const DEFAULT_TARGETS = ['x86_64-pc-windows-msvc'];

/** Licences that ask for nothing beyond keeping the notice. */
const PERMISSIVE = new Set([
  '0BSD',
  'Apache-2.0',
  'BSD-2-Clause',
  'BSD-3-Clause',
  'BSL-1.0',
  'CC0-1.0',
  'ISC',
  'LLVM-exception',
  'MIT',
  'MIT-0',
  'OFL-1.1',
  'Unicode-3.0',
  'Unicode-DFS-2016',
  'Unlicense',
  'Zlib',
]);

const MIT_TEXT = `Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.`;

const DISCLAIMER = `THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.`;

/**
 * Standard texts, for a package whose source has no licence file. The copyright
 * line is the package's own and is listed with the package, not here.
 */
const STANDARD_TEXTS = {
  MIT: `MIT License\n\nCopyright (c) <year> <copyright holders>\n\n${MIT_TEXT}`,
  'BSD-2-Clause': `BSD 2-Clause License\n\nCopyright (c) <year>, <copyright holders>\n\nRedistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:\n\n1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.\n\n2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.\n\n${DISCLAIMER}`,
  'BSD-3-Clause': `BSD 3-Clause License\n\nCopyright (c) <year>, <copyright holders>\n\nRedistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:\n\n1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.\n\n2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.\n\n3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote products derived from this software without specific prior written permission.\n\n${DISCLAIMER}`,
  ISC: `ISC License\n\nCopyright (c) <year>, <copyright holders>\n\nPermission to use, copy, modify, and/or distribute this software for any purpose with or without fee is hereby granted, provided that the above copyright notice and this permission notice appear in all copies.\n\nTHE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.`,
  Zlib: `zlib License\n\nCopyright (c) <year> <copyright holders>\n\nThis software is provided 'as-is', without any express or implied warranty. In no event will the authors be held liable for any damages arising from the use of this software.\n\nPermission is granted to anyone to use this software for any purpose, including commercial applications, and to alter it and redistribute it freely, subject to the following restrictions:\n\n1. The origin of this software must not be misrepresented; you must not claim that you wrote the original software. If you use this software in a product, an acknowledgment in the product documentation would be appreciated but is not required.\n2. Altered source versions must be plainly marked as such, and must not be misrepresented as being the original software.\n3. This notice may not be removed or altered from any source distribution.`,
};

/**
 * Long standard texts are not typed here. They are taken from a licence file
 * found in the packages themselves, recognised by their opening words.
 */
const HARVESTED_TEXTS = {
  'Apache-2.0': /Apache License\s+Version 2\.0, January 2004/,
  'MPL-2.0': /Mozilla Public License,? Version 2\.0/,
};

// ---------------------------------------------------------------------------
// Licence expressions

/**
 * Parses an SPDX expression (`MIT OR Apache-2.0`, `A AND (B OR C)`, older
 * `MIT/Apache-2.0`) into `{ op: 'or' | 'and', items }` and `{ id }` nodes.
 * `X WITH exception` is kept as the id `X`.
 */
export function parseLicense(expression) {
  const tokens = expression
    .replace(/\//g, ' OR ')
    .replace(/([()])/g, ' $1 ')
    .split(/\s+/)
    .filter(Boolean);
  let position = 0;

  function atom() {
    const token = tokens[position++];
    if (token === '(') {
      const inner = either();
      if (tokens[position++] !== ')') throw new Error(`unbalanced parentheses in "${expression}"`);
      return inner;
    }
    if (!token || token === ')' || token === 'OR' || token === 'AND') {
      throw new Error(`cannot read the licence expression "${expression}"`);
    }
    if (tokens[position] === 'WITH') position += 2;
    return { id: token };
  }

  function both() {
    const items = [atom()];
    while (tokens[position] === 'AND') {
      position++;
      items.push(atom());
    }
    return items.length === 1 ? items[0] : { op: 'and', items };
  }

  function either() {
    const items = [both()];
    while (tokens[position] === 'OR') {
      position++;
      items.push(both());
    }
    return items.length === 1 ? items[0] : { op: 'or', items };
  }

  const tree = either();
  if (position !== tokens.length)
    throw new Error(`cannot read the licence expression "${expression}"`);
  return tree;
}

/** Every licence id that appears in `expression`, in order, once each. */
export function licenseIds(expression) {
  const found = [];
  const walk = node => {
    if (node.id) {
      if (!found.includes(node.id)) found.push(node.id);
    } else node.items.forEach(walk);
  };
  walk(parseLicense(expression));
  return found;
}

/**
 * The ids that stand in the way of calling `expression` permissive: none when
 * the package may be used under permissive terms alone (any OR branch that is
 * permissive is enough, every AND branch is needed).
 */
export function nonPermissiveIds(expression) {
  const check = node => {
    if (node.id) return PERMISSIVE.has(node.id) ? [] : [node.id];
    const parts = node.items.map(check);
    if (node.op === 'or')
      return parts.some(part => part.length === 0) ? [] : [...new Set(parts.flat())];
    return [...new Set(parts.flat())];
  };
  return check(parseLicense(expression));
}

// ---------------------------------------------------------------------------
// Rust

/**
 * The packages `cargo metadata` output (one target) links into the program:
 * everything reached from the workspace members through normal dependencies.
 * Returns a Map from package id to package.
 */
export function linkedPackages(metadata) {
  const packages = new Map(metadata.packages.map(item => [item.id, item]));
  const nodes = new Map(metadata.resolve.nodes.map(node => [node.id, node]));
  const seen = new Set();
  const pending = [...metadata.workspace_members];
  while (pending.length > 0) {
    const id = pending.pop();
    if (seen.has(id)) continue;
    seen.add(id);
    for (const dependency of nodes.get(id).deps) {
      if (dependency.dep_kinds.some(kind => kind.kind === null)) pending.push(dependency.pkg);
    }
  }
  return new Map([...seen].map(id => [id, packages.get(id)]));
}

function cargoMetadata(target) {
  const result = spawnSync(
    'cargo',
    ['metadata', '--format-version', '1', '--locked', '--filter-platform', target],
    { cwd: launcherRoot, encoding: 'utf8', maxBuffer: 1 << 28 }
  );
  if (result.error) throw new Error(`could not run cargo: ${result.error.message}`);
  if (result.status !== 0)
    throw new Error(`cargo metadata failed for ${target}:\n${result.stderr}`);
  return JSON.parse(result.stdout);
}

/** Entries for the crates linked on any of `targets`, without the workspace's own. */
function rustEntries(targets) {
  const entries = new Map();
  for (const target of targets) {
    const metadata = cargoMetadata(target);
    const members = new Set(metadata.workspace_members);
    for (const [id, item] of linkedPackages(metadata)) {
      if (members.has(id) || entries.has(id)) continue;
      entries.set(id, {
        name: item.name,
        version: item.version,
        license: item.license ?? '',
        repository: item.repository ?? item.homepage ?? '',
        authors: item.authors ?? [],
        directory: dirname(item.manifest_path),
      });
    }
  }
  return [...entries.values()];
}

// ---------------------------------------------------------------------------
// npm

/** The directory of package `name` as Node would find it from `from`, or null. */
export function findPackageDirectory(name, from) {
  let directory = from;
  for (;;) {
    const candidate = join(directory, 'node_modules', name);
    if (existsSync(join(candidate, 'package.json'))) return realpathSync(candidate);
    const parent = dirname(directory);
    if (parent === directory) return null;
    directory = parent;
  }
}

function repositoryOf(manifest) {
  const repository = manifest.repository;
  const text = typeof repository === 'string' ? repository : (repository?.url ?? '');
  return (text || manifest.homepage || '').replace(/^git\+/, '').replace(/\.git$/, '');
}

function readJson(path) {
  return JSON.parse(readFileSync(path, 'utf8'));
}

/** Entries for the runtime dependencies of package.json and what they depend on. */
function npmEntries() {
  const root = readJson(join(launcherRoot, 'package.json'));
  const entries = new Map();
  const pending = Object.keys(root.dependencies ?? {}).map(name => ({ name, from: launcherRoot }));
  while (pending.length > 0) {
    const { name, from } = pending.pop();
    const directory = findPackageDirectory(name, from);
    if (!directory) throw new Error(`${name} is not installed: run \`pnpm install\` first.`);
    if (entries.has(directory)) continue;
    const manifest = readJson(join(directory, 'package.json'));
    entries.set(directory, {
      name: manifest.name,
      version: manifest.version,
      license:
        typeof manifest.license === 'string' ? manifest.license : (manifest.license?.type ?? ''),
      repository: repositoryOf(manifest),
      authors: typeof manifest.author === 'string' ? [manifest.author] : [],
      directory,
    });
    for (const dependency of Object.keys(manifest.dependencies ?? {})) {
      pending.push({ name: dependency, from: directory });
    }
  }
  return [...entries.values()];
}

// ---------------------------------------------------------------------------
// Texts and layout

/** `Name <mail>` becomes `Name`. */
export function withoutEmail(author) {
  return author
    .replace(/\s*<[^>]*>/g, '')
    .replace(/\s*\([^)]*\)\s*$/, '')
    .trim();
}

function clean(text) {
  return text
    .replace(/^\uFEFF/, '')
    .replace(/\r\n?/g, '\n')
    .trim();
}

/** The licence, copying and notice files in the top folder of a package, as texts. */
export function licenseFiles(directory) {
  return readdirSync(directory)
    .filter(name => /^(licen[cs]e|copying|notice|unlicen[cs]e)/i.test(name))
    .filter(name => statSync(join(directory, name)).isFile())
    .sort()
    .map(name => clean(readFileSync(join(directory, name), 'utf8')))
    .filter(Boolean);
}

/**
 * Groups entries by identical licence text. Returns
 * `{ groups: [{ text, entries }], withoutFiles: [entry] }`, both in a stable order.
 */
export function groupByText(entries) {
  const byText = new Map();
  const withoutFiles = [];
  for (const entry of entries) {
    const texts = entry.texts ?? [];
    if (texts.length === 0) withoutFiles.push(entry);
    for (const text of new Set(texts)) {
      if (!byText.has(text)) byText.set(text, []);
      byText.get(text).push(entry);
    }
  }
  const groups = [...byText].map(([text, members]) => ({ text, entries: members }));
  groups.sort((a, b) => b.entries.length - a.entries.length || a.text.localeCompare(b.text));
  return { groups, withoutFiles };
}

/**
 * The standard texts to print for the packages without a licence file: one per
 * licence id in their expressions, found in STANDARD_TEXTS or among `harvest`
 * (all texts seen). Throws when a package has no id with a text at all.
 */
export function standardTextsFor(withoutFiles, harvest) {
  const texts = new Map();
  for (const entry of withoutFiles) {
    const ids = entry.license ? licenseIds(entry.license) : [];
    let found = false;
    for (const id of ids) {
      let text = STANDARD_TEXTS[id];
      if (!text && HARVESTED_TEXTS[id])
        text = harvest.find(candidate => HARVESTED_TEXTS[id].test(candidate));
      if (!text) continue;
      texts.set(id, text);
      found = true;
    }
    if (!found) {
      throw new Error(
        `${entry.name} ${entry.version} has no licence file and no standard text for "${entry.license}".`
      );
    }
  }
  return texts;
}

const RULE = '='.repeat(78);
const THIN = '-'.repeat(78);

function describe(entry) {
  const parts = [`${entry.name} ${entry.version}`];
  if (entry.license) parts.push(entry.license);
  if (entry.repository) parts.push(entry.repository);
  return parts.join(', ');
}

function sortEntries(entries) {
  return [...entries].sort(
    (a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version)
  );
}

/** One part of the file: an index of the packages, then the licence texts. */
export function renderSection(title, intro, entries, harvest) {
  const sorted = sortEntries(entries);
  const { groups, withoutFiles } = groupByText(sorted);
  const lines = [RULE, `${title} (${sorted.length})`, RULE, '', intro, ''];
  lines.push(...sorted.map(entry => `  ${describe(entry)}`), '');

  groups.forEach((group, index) => {
    lines.push(THIN, `Licence text ${index + 1} of ${groups.length}, used by:`, '');
    lines.push(
      ...group.entries.map(
        entry => `  ${entry.name} ${entry.version} (${entry.license || 'no licence field'})`
      )
    );
    lines.push('', group.text, '');
  });

  if (withoutFiles.length > 0) {
    lines.push(
      THIN,
      'No licence file in the source of these packages. They state the licence',
      'below in their metadata; the standard text follows.',
      ''
    );
    for (const entry of withoutFiles) {
      const authors = entry.authors.map(withoutEmail).filter(Boolean).join(', ');
      lines.push(
        `  ${entry.name} ${entry.version}, ${entry.license}${authors ? `, by ${authors}` : ''}`
      );
    }
    lines.push('');
    for (const [id, text] of standardTextsFor(withoutFiles, harvest)) {
      lines.push(THIN, `Standard text of ${id}`, '', text, '');
    }
  }
  return lines.join('\n');
}

/** Packages that are not clearly permissive, as `{ entry, ids }`. */
export function flagged(entries) {
  return entries
    .map(entry => ({ entry, ids: entry.license ? nonPermissiveIds(entry.license) : ['unknown'] }))
    .filter(item => item.ids.length > 0);
}

function build(targets) {
  const rust = rustEntries(targets);
  const npm = npmEntries();
  for (const entry of [...rust, ...npm]) entry.texts = licenseFiles(entry.directory);
  const harvest = [...rust, ...npm].flatMap(entry => entry.texts);

  const body = [
    'OA Mod Launcher: third-party notices',
    '',
    'OA Mod Launcher contains the following third-party software and',
    'material. Each is used under the licence printed below it. The launcher itself',
    'is under the MIT licence, see LICENSE in its repository. Written by',
    'scripts/build-launcher-notices.mjs; do not edit by hand.',
    '',
    renderSection(
      'Rust crates',
      `Linked into the launcher program (${targets.join(', ')}).`,
      rust,
      harvest
    ),
    renderSection(
      'npm packages',
      'Bundled into the launcher window. Atkinson Hyperlegible Next, Atkinson Hyperlegible\nMono and Bagel Fat One are the @fontsource packages; their licence is the SIL\nOpen Font License 1.1 and the copyright line of each font is in its text.',
      npm,
      harvest
    ),
  ].join('\n');
  return { text: body.replace(/\n{3,}/g, '\n\n').trimEnd() + '\n', rust, npm };
}

function main() {
  const out = resolve(argument('out', join(launcherRoot, 'THIRD-PARTY-NOTICES.txt')));
  const targets = argument('targets', DEFAULT_TARGETS.join(',')).split(',').filter(Boolean);
  const { text, rust, npm } = build(targets);
  writeFileSync(out, text);
  console.log(
    `wrote ${out}: ${rust.length} crates, ${npm.length} npm packages, ${Buffer.byteLength(text)} bytes`
  );
  for (const { entry, ids } of flagged([...rust, ...npm])) {
    console.error(
      `NOT PERMISSIVE: ${entry.name} ${entry.version}, ${entry.license || 'no licence'} (${ids.join(', ')})`
    );
  }
}

const invoked = process.argv[1] ? resolve(process.argv[1]) : '';
const here = fileURLToPath(import.meta.url);
if (
  process.platform === 'win32' ? invoked.toLowerCase() === here.toLowerCase() : invoked === here
) {
  try {
    main();
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
