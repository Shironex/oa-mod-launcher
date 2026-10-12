// Builds manifest.json, the one file a mod release publishes for the launcher:
// which version, where the DLL and the BepInEx zip are, how large they are and
// what they hash to. Node builtins only.
//
// Used by the mod's release workflow and by local runs, so a local test of the
// launcher reads a file made by the same code as a real release.

import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';

export const SCHEMA = 1;

/** The files BepInEx puts into the root of the game folder. Exactly these. */
export const ROOT_FILES = [
  'winhttp.dll',
  'doorstop_config.ini',
  '.doorstop_version',
  'changelog.txt',
];

/** `## 0.9.0 (2026-12-05) Some tag`. The date and the tag are optional. */
export const VERSION_HEADING =
  /^##\s+\[?v?(\d+\.\d+\.\d+)\]?\s*(?:-\s*)?(?:\(?(\d{4}-\d{2}-\d{2})\)?)?\s*(.*)$/;

export function isVersion(text) {
  return /^\d+\.\d+\.\d+$/.test(text);
}

function isHash(text) {
  return typeof text === 'string' && /^[0-9a-f]{64}$/.test(text);
}

/** `OaMod-0.1.0.dll`, the name of the mod in its release. */
export function modName(version) {
  return `OaMod-${version}.dll`;
}

/**
 * The first line of text under the heading of `version` in a changelog, or
 * undefined when the changelog has no such section or the section no text.
 */
export function notesFromChangelog(markdown, version) {
  let inside = false;
  for (const raw of markdown.replace(/\r\n/g, '\n').split('\n')) {
    const line = raw.trim();
    const heading = VERSION_HEADING.exec(line);
    if (heading) {
      if (inside) return undefined;
      inside = heading[1] === version;
    } else if (inside && line && !line.startsWith('#')) {
      return line.replace(/^[-*]\s+/, '');
    }
  }
  return undefined;
}

/** Steam build ids from one argument: separated by commas or white space. */
export function parseGameBuilds(text) {
  const builds = (text ?? '').split(/[\s,]+/).filter(Boolean);
  for (const build of builds) {
    if (!/^\d{1,20}$/.test(build)) throw new Error(`not a Steam build id: ${build}`);
  }
  return builds;
}

/**
 * The `bepinex` block of the manifest, from the content of the mod
 * repository's bepinex.json. Every field is required, so a file without the
 * mirror address or the probe hash stops the release here and not in an
 * installed launcher.
 */
export function bepinexBlock(input) {
  const { label, url, size, sha256, probe_sha256, root_files } = input ?? {};
  if (
    typeof label !== 'string' ||
    !label ||
    label.length > 64 ||
    [...label].some(character => character.charCodeAt(0) < 32)
  ) {
    throw new Error('bepinex.json: "label" must be one short line');
  }
  if (typeof url !== 'string' || !url.startsWith('https://')) {
    throw new Error('bepinex.json: "url" must be the https address of the mirrored zip');
  }
  if (!Number.isSafeInteger(size) || size <= 0) {
    throw new Error('bepinex.json: "size" must be the length of the zip in bytes');
  }
  if (!isHash(sha256))
    throw new Error('bepinex.json: "sha256" must be 64 lowercase hex characters');
  if (!isHash(probe_sha256)) {
    throw new Error(
      'bepinex.json: "probe_sha256" must be the sha256 of BepInEx/core/BepInEx.Unity.IL2CPP.dll'
    );
  }
  const names = Object.keys(root_files ?? {});
  if (names.length !== ROOT_FILES.length || !ROOT_FILES.every(name => isHash(root_files[name]))) {
    throw new Error(
      `bepinex.json: "root_files" must hold a sha256 for exactly: ${ROOT_FILES.join(', ')}`
    );
  }
  return {
    label,
    url,
    size,
    sha256,
    probe_sha256,
    root_files: Object.fromEntries(ROOT_FILES.map(name => [name, root_files[name]])),
  };
}

/**
 * The manifest of `version`.
 *
 * @param {{ mod: string, version: string, baseUrl: string, bepinex: object,
 *           gameBuilds?: string[], notes?: string, launcherMin?: string,
 *           published?: string }} options
 *   `mod` is the path of the built DLL. `baseUrl` is where the files of this
 *   release will be downloadable; the DLL is published there as
 *   `OaMod-<version>.dll` whatever the local file is called.
 */
export function buildManifest(options) {
  const { mod, version, baseUrl, bepinex, gameBuilds = [], notes, launcherMin = '0.1.0' } = options;
  if (!isVersion(version)) throw new Error(`not a version: ${version}`);
  if (!isVersion(launcherMin)) throw new Error(`not a launcher version: ${launcherMin}`);
  const base = baseUrl.endsWith('/') ? baseUrl : `${baseUrl}/`;

  const bytes = readFileSync(mod);
  if (bytes.length === 0) throw new Error(`the mod file is empty: ${mod}`);

  return {
    schema: SCHEMA,
    version,
    published: options.published ?? new Date().toISOString().replace(/\.\d+Z$/, 'Z'),
    ...(notes ? { notes } : {}),
    mod: {
      url: new URL(modName(version), base).href,
      size: bytes.length,
      sha256: createHash('sha256').update(bytes).digest('hex'),
    },
    bepinex: bepinexBlock(bepinex),
    game: { builds: gameBuilds },
    launcher: { min: launcherMin },
  };
}

/**
 * The exact text of manifest.json. These are the bytes that get signed and
 * that the launcher parses, so every writer goes through here.
 */
export function manifestText(value) {
  return `${JSON.stringify(value, null, 2)}\n`;
}

/** The value of `--name` in the arguments of the running script. */
export function argument(name, fallback) {
  const index = process.argv.indexOf(`--${name}`);
  return index === -1 ? fallback : process.argv[index + 1];
}

export function flag(name) {
  return process.argv.includes(`--${name}`);
}
