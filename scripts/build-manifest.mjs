// Writes manifest.json for one mod release.
//
//   node scripts/build-manifest.mjs --mod <OaMod.dll> --version 0.1.0 \
//     --base-url https://github.com/Shironex/oa-mod/releases/download/v0.1.0/ \
//     --bepinex bepinex.json [--game-builds "20236533 20240000"] \
//     [--notes "One line."] [--changelog CHANGELOG.md] [--launcher-min 0.1.0] \
//     [--out <folder>] [--sign-key <key>] [--no-sign]
//
// The manifest is written to `--out`, which defaults to the working directory,
// and signed (manifest.json.sig). For a github.com address `--sign-key` is
// required (the password comes from the environment, see sign-file.mjs) unless
// `--no-sign`; any other address is signed with the development key by default.
//
// `--bepinex` is a JSON file with label, url, size, sha256, probe_sha256 and
// root_files, see docs/contract.md. Without `--notes`, the first line under the
// version's heading in `--changelog` becomes the notes.

import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import {
  argument,
  buildManifest,
  flag,
  manifestText,
  notesFromChangelog,
  parseGameBuilds,
} from './lib/manifest.mjs';
import { manifestSigningKey, signFile } from './lib/sign.mjs';

const mod = argument('mod');
const version = argument('version');
const baseUrl = argument('base-url');
const bepinexPath = argument('bepinex');
if (!mod || !version || !baseUrl || !bepinexPath) {
  console.error(
    'usage: build-manifest.mjs --mod <dll> --version <x.y.z> --base-url <url> --bepinex <bepinex.json>'
  );
  process.exit(2);
}

try {
  const signingKey = manifestSigningKey({
    baseUrl,
    signKey: argument('sign-key'),
    noSign: flag('no-sign'),
  });

  const changelogPath = argument('changelog');
  const changelog =
    changelogPath && existsSync(changelogPath) ? readFileSync(changelogPath, 'utf8') : '';
  const manifest = buildManifest({
    mod: resolve(mod),
    version,
    baseUrl,
    bepinex: JSON.parse(readFileSync(bepinexPath, 'utf8')),
    gameBuilds: parseGameBuilds(argument('game-builds')),
    notes: argument('notes') ?? notesFromChangelog(changelog, version),
    launcherMin: argument('launcher-min', '0.1.0'),
  });

  const out = resolve(argument('out', '.'));
  mkdirSync(out, { recursive: true });
  const file = join(out, 'manifest.json');
  writeFileSync(file, manifestText(manifest));
  console.log(`manifest.json: version ${version}, ${manifest.mod.size} bytes of mod`);
  if (signingKey) console.log(signFile({ file, key: signingKey }));
} catch (error) {
  console.error(error.message);
  process.exit(1);
}
