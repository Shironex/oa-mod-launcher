// Builds latest.json, the file the Tauri updater plugin reads to learn about a
// new launcher installer. Node builtins only.
//
// Only the Windows platform is written: the updater validates every platform
// entry of the file, so a platform that is not shipped must be absent.

import { isVersion, VERSION_HEADING } from './manifest.mjs';

export const PLATFORM = 'windows-x86_64';

/** `OaModLauncher-0.1.0-windows-x64-setup.exe`. No spaces: GitHub rewrites them in asset names. */
export function installerAssetName(version) {
  if (!isVersion(version)) throw new Error(`not a version: ${version}`);
  return `OaModLauncher-${version}-windows-x64-setup.exe`;
}

/** Where GitHub serves the installer of release `v<version>`. */
export function installerUrl(repo, version) {
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repo)) {
    throw new Error(`not a repository, expected owner/name: ${repo}`);
  }
  return `https://github.com/${repo}/releases/download/v${version}/${installerAssetName(version)}`;
}

/**
 * The notes of `version`: the text of its section in the launcher's
 * CHANGELOG.md, from below its `## <version>` heading to the next version
 * heading. The headings are the ones `VERSION_HEADING` matches. Throws
 * when the section is missing or empty, so a release cannot go out without
 * notes.
 */
export function changelogSection(markdown, version) {
  const lines = markdown.replace(/\r\n/g, '\n').split('\n');
  const start = lines.findIndex(line => VERSION_HEADING.exec(line.trim())?.[1] === version);
  if (start === -1) throw new Error(`the changelog has no section for version ${version}.`);
  const rest = lines.slice(start + 1);
  const end = rest.findIndex(line => VERSION_HEADING.test(line.trim()));
  const text = rest
    .slice(0, end === -1 ? rest.length : end)
    .join('\n')
    .trim();
  if (!text) throw new Error(`the changelog section for version ${version} is empty.`);
  return text;
}

/**
 * The content of latest.json.
 *
 * @param {{ version: string, repo: string, signature: string, notes?: string, now?: Date }} options
 *   `signature` is the content of the `.sig` file, taken as it is apart from
 *   surrounding whitespace.
 */
export function buildLatest({ version, repo, signature, notes = '', now = new Date() }) {
  const trimmed = (signature ?? '').trim();
  if (!trimmed) throw new Error('the signature is empty.');
  return {
    version,
    notes,
    pub_date: now.toISOString(),
    platforms: {
      [PLATFORM]: { signature: trimmed, url: installerUrl(repo, version) },
    },
  };
}
