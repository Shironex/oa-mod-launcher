// Fixed states for looking at the window without a game, Steam or a download.
//
// Development only: the store imports this file behind `import.meta.env.DEV`,
// so it is not part of a release build. Open the dev page with `?preview=`
// followed by one of the names below, for example `?preview=installing`.
//
// Every version, path and sentence in this file is an example.

import type { Progress, RemovalOutcome, RemovalPlan, Snapshot } from '../bindings';
import type { Dialog } from '../state/store';
import { UNKNOWN_BUILD, type Action } from '../state/view';

export interface Preview {
  snapshot: Snapshot | null;
  progress: Progress | null;
  checking: boolean;
  error: string | null;
  failed: Action | null;
  fatal: string | null;
  fresh: boolean;
  firstRun: boolean;
  /** The dialog that is open from the start. */
  dialog?: Dialog;
  /** What the remove dialog shows. */
  plan?: RemovalPlan;
  /** What a full removal answers with in this preview. */
  outcome?: RemovalOutcome;
}

const MEGABYTE = 1024 * 1024;
const NEWEST = '0.2.0';
const BEFORE = '0.1.0';
const LABEL = '6.0.0-be.788';
const GAME_DIR = 'C:\\Program Files (x86)\\Steam\\steamapps\\common\\Golf With Your Friends';
const NOTES = 'Brackets for up to 8 players, and the shot clock pauses while someone reconnects.';
/** The BepInEx zip and the mod together. */
const FIRST_SIZE = 34_336_405 + 412_160;

const QUIET = {
  progress: null,
  checking: false,
  error: null,
  failed: null,
  fatal: null,
  fresh: false,
  firstRun: false,
};

/** The mod installed, prepared and current. */
const READY: Snapshot = {
  launcher_version: '0.1.0',
  status: { kind: 'up_to_date' },
  game: { dir: GAME_DIR, build_id: '20236533', steam_busy: false },
  bepinex: { kind: 'ours', label: LABEL },
  mod_installed: NEWEST,
  foreign_plugins: [],
  remote: { kind: 'up_to_date' },
  warnings: [],
  notes: NOTES,
  check_on_start: true,
};

/** A game folder with nothing in it, and a release that was found. */
const EMPTY: Snapshot = {
  ...READY,
  status: { kind: 'not_installed' },
  bepinex: { kind: 'absent' },
  mod_installed: null,
  remote: { kind: 'update_available', version: NEWEST, size: FIRST_SIZE },
};

const BEHIND: Snapshot = {
  ...READY,
  status: { kind: 'update_available' },
  mod_installed: BEFORE,
  remote: { kind: 'update_available', version: NEWEST, size: 412_160 },
};

const WITH_GHOST_CADDY: Snapshot = { ...READY, foreign_plugins: ['GhostCaddy'] };

/** BepInEx is the launcher's own and nobody else's files are there. */
const PLAN_OURS: RemovalPlan = {
  game_dir: GAME_DIR,
  bepinex: { kind: 'ours', label: LABEL },
  mod_files: ['BepInEx/plugins/OaMod/OaMod.dll', 'BepInEx/config/com.shirone.oamod.cfg'],
  folders: ['dotnet', 'BepInEx'],
  root_files: ['winhttp.dll', 'doorstop_config.ini', '.doorstop_version', 'changelog.txt'],
  unrecognised_root_files: [],
  kept_with_proxy: [],
  kept: [],
  foreign: [],
  mod_only_recommended: false,
};

/** Another plugin and its settings live in the same BepInEx. */
const PLAN_FOREIGN: RemovalPlan = {
  ...PLAN_OURS,
  foreign: [
    { path: 'BepInEx/plugins/GhostCaddy', kind: 'plugin', bytes: 1_482_752 },
    { path: 'BepInEx/config/com.shironex.ghostcaddy.cfg', kind: 'config', bytes: 9_216 },
  ],
  mod_only_recommended: true,
};

/**
 * A BepInEx somebody else installed: the launcher does not know its
 * `winhttp.dll`, so the two files that one reads stay with it.
 */
const PLAN_UNRECOGNISED: RemovalPlan = {
  ...PLAN_FOREIGN,
  bepinex: { kind: 'foreign' },
  root_files: ['changelog.txt'],
  unrecognised_root_files: ['winhttp.dll', 'doorstop_config.ini', '.doorstop_version'],
  kept_with_proxy: ['doorstop_config.ini', '.doorstop_version'],
};

const PREVIEWS: Record<string, Preview> = {
  // The rows of the status table.
  error: {
    ...QUIET,
    snapshot: {
      ...READY,
      status: {
        kind: 'error',
        reason:
          "The launcher's settings could not be opened: state.json is not valid JSON at line 3",
      },
      notes: null,
    },
  },
  installing: {
    ...QUIET,
    fresh: true,
    snapshot: { ...EMPTY, status: { kind: 'installing', phase: 'downloading_bepinex' } },
    progress: { phase: 'downloading_bepinex', received: 21 * MEGABYTE, total: 34_336_405 },
  },
  'installing-unpack': {
    ...QUIET,
    fresh: true,
    snapshot: { ...EMPTY, status: { kind: 'installing', phase: 'installing_bepinex' } },
    progress: { phase: 'installing_bepinex', received: 0, total: 0 },
  },
  updating: {
    ...QUIET,
    snapshot: { ...BEHIND, status: { kind: 'installing', phase: 'downloading_mod' } },
    progress: { phase: 'downloading_mod', received: 247_296, total: 412_160 },
  },
  removing: { ...QUIET, snapshot: { ...READY, status: { kind: 'removing' } } },
  'game-not-found': {
    ...QUIET,
    snapshot: { ...EMPTY, status: { kind: 'game_not_found' }, game: null, notes: null },
  },
  running: { ...QUIET, snapshot: { ...READY, status: { kind: 'game_running' } } },
  'running-first-start': {
    ...QUIET,
    firstRun: true,
    snapshot: { ...READY, status: { kind: 'game_running' } },
  },
  'running-vanilla': { ...QUIET, snapshot: { ...EMPTY, status: { kind: 'game_running' } } },
  clean: { ...QUIET, snapshot: { ...EMPTY, status: { kind: 'clean' } } },
  'not-installed': { ...QUIET, snapshot: EMPTY },
  // After "Remove only OA Mod": BepInEx and the other plugin stayed.
  'not-installed-bepinex': {
    ...QUIET,
    snapshot: {
      ...EMPTY,
      bepinex: { kind: 'ours', label: LABEL },
      foreign_plugins: ['GhostCaddy'],
      remote: { kind: 'update_available', version: NEWEST, size: 412_160 },
    },
  },
  unsupported: {
    ...QUIET,
    snapshot: {
      ...EMPTY,
      bepinex: {
        kind: 'unsupported',
        reason: 'winhttp.dll is in the game folder without BepInEx beside it',
      },
    },
    plan: {
      ...PLAN_OURS,
      bepinex: {
        kind: 'unsupported',
        reason: 'winhttp.dll is in the game folder without BepInEx beside it',
      },
      mod_files: [],
      folders: [],
      root_files: [],
      unrecognised_root_files: ['winhttp.dll'],
    },
  },
  'first-run': { ...QUIET, snapshot: { ...READY, status: { kind: 'first_run_needed' } } },
  update: { ...QUIET, snapshot: BEHIND },
  'unknown-build': {
    ...QUIET,
    snapshot: {
      ...READY,
      status: { kind: 'update_available' },
      mod_installed: UNKNOWN_BUILD,
      remote: { kind: 'unknown' },
      notes: null,
    },
  },
  'up-to-date': { ...QUIET, snapshot: READY },

  // What the update check adds.
  checking: {
    ...QUIET,
    checking: true,
    snapshot: { ...READY, remote: { kind: 'unknown' }, notes: null },
  },
  offline: {
    ...QUIET,
    snapshot: {
      ...READY,
      remote: {
        kind: 'offline',
        reason: 'Could not check for updates: github.com did not answer within 5 s',
      },
      notes: null,
    },
  },
  'launcher-too-old': {
    ...QUIET,
    snapshot: { ...EMPTY, remote: { kind: 'launcher_too_old', required: '0.3.0' } },
  },

  // Warnings, Steam, and commands that failed.
  warnings: {
    ...QUIET,
    snapshot: {
      ...READY,
      bepinex: { kind: 'foreign' },
      warnings: [
        { kind: 'game_build_untested', build: '20999999' },
        { kind: 'foreign_bep_in_ex' },
        { kind: 'slow_next_start' },
      ],
    },
  },
  'bepinex-differs': {
    ...QUIET,
    snapshot: {
      ...READY,
      warnings: [{ kind: 'bep_in_ex_differs', installed: LABEL, wanted: '6.0.0-be.800' }],
    },
  },
  'steam-busy': {
    ...QUIET,
    snapshot: { ...EMPTY, game: { dir: GAME_DIR, build_id: '20236533', steam_busy: true } },
  },
  failed: {
    ...QUIET,
    error: 'The download does not match its checksum, so it was not installed',
    failed: { type: 'install' },
    snapshot: EMPTY,
  },
  'failed-folder': {
    ...QUIET,
    error: `The game folder is not writable (${GAME_DIR}\\winhttp.dll)`,
    failed: { type: 'install' },
    snapshot: EMPTY,
  },
  'failed-pick': {
    ...QUIET,
    error:
      'This is not a Golf With Your Friends folder (D:\\Games): Golf With Your Friends.exe is not in it',
    failed: { type: 'pick_folder' },
    snapshot: { ...EMPTY, status: { kind: 'game_not_found' }, game: null, notes: null },
  },
  starting: { ...QUIET, snapshot: null },
  fatal: {
    ...QUIET,
    snapshot: null,
    fatal: 'The per-user application folder could not be found',
  },

  // The dialogs.
  settings: { ...QUIET, snapshot: READY, dialog: 'settings', plan: PLAN_OURS },
  about: { ...QUIET, snapshot: READY, dialog: 'about' },
  remove: { ...QUIET, snapshot: READY, dialog: 'remove', plan: PLAN_OURS },
  'remove-foreign': {
    ...QUIET,
    snapshot: WITH_GHOST_CADDY,
    dialog: 'remove',
    plan: PLAN_FOREIGN,
    outcome: {
      snapshot: { ...EMPTY, status: { kind: 'clean' } },
      kept: [],
      backup_dir: 'C:\\Users\\friend\\AppData\\Local\\OaModLauncher\\backups\\2026-10-12_18-30-14',
    },
  },
  // Click "Delete BepInEx and OA Mod" without the tick to see what the kept
  // files mean afterwards.
  'remove-unrecognised': {
    ...QUIET,
    snapshot: {
      ...WITH_GHOST_CADDY,
      bepinex: { kind: 'foreign' },
      warnings: [{ kind: 'foreign_bep_in_ex' }],
    },
    dialog: 'remove',
    plan: PLAN_UNRECOGNISED,
    outcome: {
      snapshot: {
        ...EMPTY,
        bepinex: {
          kind: 'unsupported',
          reason: 'winhttp.dll is in the game folder without BepInEx beside it',
        },
      },
      kept: ['winhttp.dll', 'doorstop_config.ini', '.doorstop_version'],
      backup_dir: 'C:\\Users\\friend\\AppData\\Local\\OaModLauncher\\backups\\2026-10-12_18-30-14',
    },
  },
};

/** The names of all previews, for the test that renders each of them. */
export const PREVIEW_NAMES = Object.keys(PREVIEWS);

/** One preview by its name. */
export function previewNamed(name: string): Preview | null {
  return PREVIEWS[name] ?? null;
}

/** The preview named in the address of the page, if any. */
export function previewFromLocation(): Preview | null {
  const name = new URLSearchParams(window.location.search).get('preview');
  return name ? previewNamed(name) : null;
}
