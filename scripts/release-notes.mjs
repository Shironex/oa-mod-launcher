// Prints the notes of one launcher release: the section of that version in
// CHANGELOG.md, and after it the text of `--footer` when one is given.
//
//   node scripts/release-notes.mjs --version 0.1.2 [--changelog CHANGELOG.md] \
//     [--footer .github/release-footer.md]
//
// The release workflow calls it twice: without the footer for the `notes` of
// latest.json, which the launcher shows in its settings, and with it for the
// GitHub release page. `{version}` in the footer becomes the version. A
// version without a section in the changelog is an error, so a release cannot
// go out without notes.

import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { argument } from './lib/manifest.mjs';
import { changelogSection } from './lib/latest.mjs';

const version = argument('version');
if (!version) {
  console.error(
    'usage: release-notes.mjs --version <x.y.z> [--changelog <file>] [--footer <file>]'
  );
  process.exit(2);
}
// The launcher's own changelog by default, wherever the script is called from.
const changelog = argument(
  'changelog',
  join(dirname(fileURLToPath(import.meta.url)), '..', 'CHANGELOG.md')
);
const footer = argument('footer');

try {
  let notes = changelogSection(readFileSync(changelog, 'utf8'), version);
  if (footer) {
    notes += `\n\n${readFileSync(footer, 'utf8').replaceAll('{version}', version).trim()}`;
  }
  console.log(notes);
} catch (error) {
  console.error(error.message);
  process.exit(1);
}
