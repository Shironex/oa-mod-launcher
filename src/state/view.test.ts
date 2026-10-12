import { describe, expect, it } from 'vitest';
import type { Snapshot, Warning } from '../bindings';
import { deriveView, detailsText, UNKNOWN_BUILD, warningNote, type ViewInput } from './view';

const MEGABYTE = 1024 * 1024;

/** The mod installed, prepared and current: the last row of the status table. */
const SETTLED: Snapshot = {
  launcher_version: '0.1.0',
  status: { kind: 'up_to_date' },
  game: { dir: 'D:\\Games\\Golf With Your Friends', build_id: '20236533', steam_busy: false },
  bepinex: { kind: 'ours', label: '6.0.0-be.788' },
  bepinex_wanted: '6.0.0-be.788',
  game_dir_chosen: false,
  mod_installed: '0.2.0',
  foreign_plugins: [],
  remote: { kind: 'up_to_date' },
  warnings: [],
  notes: null,
  check_on_start: true,
};

/** A game folder with nothing in it. */
const EMPTY: Snapshot = {
  ...SETTLED,
  status: { kind: 'not_installed' },
  bepinex: { kind: 'absent' },
  mod_installed: null,
  remote: { kind: 'update_available', version: '0.2.0', size: 33 * MEGABYTE },
};

function view(snapshot: Partial<Snapshot>, rest: Partial<ViewInput> = {}) {
  return deriveView({
    snapshot: { ...SETTLED, ...snapshot },
    progress: null,
    checking: false,
    error: null,
    failed: null,
    fatal: null,
    fresh: false,
    firstRun: false,
    ...rest,
  });
}

describe('the status table, row by row', () => {
  it('1 error: says why, opens the logs, and promises nothing was changed', () => {
    const error = view({ status: { kind: 'error', reason: 'state.json is not valid JSON' } });
    expect(error.sentence).toContain('Nothing was changed');
    expect(error.detail).toBe('state.json is not valid JSON.');
    expect(error.cta).toMatchObject({ word: 'Open logs', action: { type: 'open_logs' } });
    expect(error.links.map(link => link.label)).toEqual(['Look again', 'Copy details']);
  });

  it('2 installing, fresh: four holes, the one in play has its own percentage', () => {
    const installing = view(
      { ...EMPTY, status: { kind: 'installing', phase: 'downloading_bepinex' } },
      {
        fresh: true,
        progress: { phase: 'downloading_bepinex', received: 16 * MEGABYTE, total: 32 * MEGABYTE },
      }
    );
    expect(installing.sentence).toBe('Downloading BepInEx.');
    expect(installing.holes?.map(hole => hole.state)).toEqual(['now', 'next', 'next', 'next']);
    expect(installing.holes?.[0]?.mark).toBe('50%');
    // Half of the first hole, which ends at 40 of the total.
    expect(installing.cta).toMatchObject({ style: 'busy', word: 'Installing', sub: '20%' });
    expect(installing.cta.action).toBeUndefined();
    expect(installing.pill?.text).toBe('step 1 of 4 · 16.0 of 32.0 MB');
    expect(installing.locked).toBe('An install is running.');
  });

  it('2 installing, fresh: the card keeps four holes after BepInEx is in', () => {
    // The poll answers `ours` halfway through. What the install began with decides.
    const later = view(
      { status: { kind: 'installing', phase: 'downloading_mod' }, mod_installed: null },
      { fresh: true }
    );
    expect(later.holes?.map(hole => hole.state)).toEqual(['done', 'done', 'now', 'next']);
    expect(later.cta.sub).toBe('55%');
  });

  it('2 installing, update: two holes, and the word is Updating', () => {
    const updating = view(
      {
        status: { kind: 'installing', phase: 'installing_mod' },
        mod_installed: '0.1.0',
        remote: { kind: 'update_available', version: '0.2.0', size: MEGABYTE },
      },
      // A report of the step before says nothing about this one.
      { progress: { phase: 'downloading_mod', received: MEGABYTE, total: MEGABYTE } }
    );
    expect(updating.holes).toHaveLength(2);
    expect(updating.holes?.[1]).toMatchObject({ state: 'now', mark: 'working' });
    expect(updating.cta).toMatchObject({ word: 'Updating', sub: '85%' });
    expect(updating.sentence).toBe('Putting OA Mod into the game folder.');
  });

  it('2 installing: a progress report is ahead of the polled step, and wins', () => {
    // What the page sets itself when Install is pressed, and the first report.
    const started = view(
      { ...EMPTY, status: { kind: 'installing', phase: 'checking' } },
      {
        fresh: true,
        progress: { phase: 'downloading_bepinex', received: 16 * MEGABYTE, total: 32 * MEGABYTE },
      }
    );
    expect(started.sentence).toBe('Downloading BepInEx.');
    expect(started.cta.sub).toBe('20%');

    // The download is over and the poll has not said so yet: the bar does not fall back.
    const unpacking = view(
      { ...EMPTY, status: { kind: 'installing', phase: 'downloading_bepinex' } },
      { fresh: true, progress: { phase: 'installing_bepinex', received: 0, total: 0 } }
    );
    expect(unpacking.holes?.map(hole => hole.state)).toEqual(['done', 'now', 'next', 'next']);
    expect(unpacking.cta.sub).toBe('40%');
  });

  it('2 installing: a BepInEx step is a fresh install even when nobody said so', () => {
    const installing = view({ status: { kind: 'installing', phase: 'installing_bepinex' } });
    expect(installing.holes).toHaveLength(4);
  });

  it('3 removing: nothing can be pressed', () => {
    const removing = view({ status: { kind: 'removing' } });
    expect(removing.cta).toEqual({ style: 'off', word: 'Removing', sub: 'please wait' });
    expect(removing.locked).toBe('A removal is running.');
  });

  it('4 game not found: the action chooses a folder and the right half explains where it is', () => {
    const missing = view({ ...EMPTY, status: { kind: 'game_not_found' }, game: null });
    expect(missing.cta).toMatchObject({
      word: 'Choose game folder',
      action: { type: 'pick_folder' },
    });
    expect(missing.links).toEqual([{ label: 'Look again', action: { type: 'look_again' } }]);
    expect(missing.right).toEqual({ kind: 'find_game' });
    expect(missing.canRemove).toBe(false);
  });

  it('5 game running: every action is off, Play included, and the reason is shown', () => {
    const running = view({ status: { kind: 'game_running' } });
    expect(running.cta).toEqual({ style: 'off', word: 'Game is running', sub: 'OA Mod 0.2.0' });
    expect(running.links).toEqual([]);
    // A failed check adds its note, and still nothing to press.
    const offline = view({
      status: { kind: 'game_running' },
      remote: { kind: 'offline', reason: 'github.com did not answer' },
    });
    expect(offline.links).toEqual([]);
    expect(offline.notes.at(-1)?.label).toBe('Offline');
    expect(running.locked).toBe('Close the game first.');
    expect(running.notes[0]?.text).toContain('Windows keeps the mod files locked');
  });

  it('5 game running without the mod says so, and the flag is blank', () => {
    const vanilla = view({ ...EMPTY, status: { kind: 'game_running' } });
    expect(vanilla.sentence).toBe('Golf With Your Friends is running without the mod.');
    expect(vanilla.flag).toBe(false);
  });

  it('5 game running after the first Play: says it has not hung', () => {
    const first = view({ status: { kind: 'game_running' } }, { firstRun: true });
    expect(first.sentence).toContain('has not hung');
    expect(first.cta.action).toBeUndefined();
    expect(first.notes[0]?.text).toContain('internet connection');
  });

  it('6 clean: the game is as Steam installed it, and Install is one click away', () => {
    const clean = view({ ...EMPTY, status: { kind: 'clean' } });
    expect(clean.sentence).toBe('All clean. The game is back to how Steam installed it.');
    expect(clean.cta).toMatchObject({ word: 'Install', sub: 'OA Mod 0.2.0' });
    expect(clean.canRemove).toBe(false);
  });

  it('7 not installed: names both downloads and their size', () => {
    const fresh = view(EMPTY);
    expect(fresh.cta).toMatchObject({
      style: 'primary',
      word: 'Install',
      sub: 'OA Mod 0.2.0',
      action: { type: 'install' },
    });
    expect(fresh.notes[0]?.text).toBe(
      'BepInEx 6.0.0-be.788 and OA Mod 0.2.0. Two downloads, 33.0 MB, about a minute.'
    );
    expect(view({ ...EMPTY, bepinex_wanted: null }).notes[0]?.text).toContain(
      'BepInEx and OA Mod 0.2.0.'
    );
    expect(fresh.links[0]).toEqual({ label: 'Play without the mod', action: { type: 'play' } });
    expect(fresh.flag).toBe(false);
  });

  it('7 not installed, before any check: Install still works, it checks by itself', () => {
    const unchecked = view({ ...EMPTY, remote: { kind: 'unknown' } });
    expect(unchecked.cta).toMatchObject({ sub: 'newest release', action: { type: 'install' } });
  });

  it('7 not installed with BepInEx there: only the mod is fetched, and it can be removed', () => {
    const kept = view({ ...EMPTY, bepinex: { kind: 'adopted', label: '6.0.0-be.788' } });
    expect(kept.sentence).toBe('OA Mod is not installed. BepInEx is already in the game folder.');
    expect(kept.notes[0]?.text).toBe('OA Mod 0.2.0 only. One small download.');
    expect(kept.canRemove).toBe(true);
  });

  it('7 not installed with an unsupported mod loader: Install stays, and removal is the way out', () => {
    const blocked = view({
      ...EMPTY,
      bepinex: { kind: 'unsupported', reason: 'winhttp.dll is there without BepInEx beside it' },
    });
    // An interrupted install reads as unsupported too, and the next install clears it.
    expect(blocked.cta).toEqual({
      style: 'primary',
      word: 'Install',
      sub: 'try anyway',
      action: { type: 'install' },
    });
    expect(blocked.detail).toBe('winhttp.dll is there without BepInEx beside it.');
    expect(blocked.notes[0]?.text).toContain('Remove mod');
    expect(blocked.canRemove).toBe(true);
  });

  it('8 first run needed: Play, with the minutes and the internet said plainly', () => {
    const first = view({ status: { kind: 'first_run_needed' }, remote: { kind: 'up_to_date' } });
    expect(first.sentence).toBe('Installed. The first start takes a few minutes.');
    expect(first.cta).toMatchObject({ word: 'Play', sub: 'first start', action: { type: 'play' } });
    expect(first.notes[0]?.text).toContain('2 to 5 minutes');
    expect(first.notes[0]?.text).toContain('internet connection');
  });

  it('9 update available: both versions on the button, and a quiet way to play anyway', () => {
    const update = view({
      status: { kind: 'update_available' },
      mod_installed: '0.1.0',
      remote: { kind: 'update_available', version: '0.2.0', size: MEGABYTE },
      notes: 'Brackets for up to 8 players.',
    });
    expect(update.sentence).toBe('OA Mod 0.2.0 is out. You have 0.1.0.');
    expect(update.cta).toMatchObject({
      word: 'Update',
      sub: '0.1.0 → 0.2.0',
      action: { type: 'install' },
    });
    expect(update.links[0]).toEqual({ label: 'Play 0.1.0 anyway', action: { type: 'play' } });
    expect(update.notes[0]?.text).toContain('Everyone in a match needs the same OA Mod');
    expect(update.right).toEqual({
      kind: 'release',
      title: 'OA Mod 0.2.0',
      tag: 'new',
      notes: 'Brackets for up to 8 players.',
    });
  });

  it('9 update available for an unknown build, before any check: no version is invented', () => {
    const unknown = view({
      status: { kind: 'update_available' },
      mod_installed: UNKNOWN_BUILD,
      remote: { kind: 'unknown' },
    });
    expect(unknown.sentence).toContain('not a build this launcher installed');
    expect(unknown.cta).toMatchObject({ word: 'Update', sub: 'newest release' });
    expect(unknown.links[0]?.label).toBe('Play anyway');
  });

  it('10 up to date: Play carries the mod version', () => {
    const ready = view({ notes: 'Brackets for up to 8 players.' });
    expect(ready.sentence).toBe('OA Mod 0.2.0 is installed and up to date.');
    expect(ready.cta).toMatchObject({
      word: 'Play',
      sub: 'OA Mod 0.2.0',
      action: { type: 'play' },
    });
    expect(ready.pill).toEqual({ tone: 'ok', text: 'BepInEx 6.0.0-be.788 · checked' });
    expect(ready.notes).toEqual([]);
    expect(ready.flag).toBe(true);
    expect(ready.canRemove).toBe(true);
    expect(ready.right).toMatchObject({ kind: 'release', tag: 'installed' });
  });

  it('10 without notes the right half is the help, never an invented release', () => {
    expect(view({}).right).toEqual({ kind: 'guide' });
  });
});

describe('what the update check adds', () => {
  it('not checked yet: installed, without the claim of being current', () => {
    const ready = view({ remote: { kind: 'unknown' } });
    expect(ready.sentence).toBe('OA Mod 0.2.0 is installed.');
    expect(ready.pill?.text).toBe('BepInEx 6.0.0-be.788');
  });

  it('checking: the pill says so', () => {
    const checking = view({ remote: { kind: 'unknown' } }, { checking: true });
    expect(checking.pill).toEqual({ tone: 'wait', text: 'checking for updates' });
  });

  it('offline: Play stays, with the reason and a way to check again', () => {
    const offline = view({ remote: { kind: 'offline', reason: 'github.com did not answer' } });
    expect(offline.cta.action).toEqual({ type: 'play' });
    expect(offline.links).toContainEqual({ label: 'Check again', action: { type: 'check' } });
    expect(offline.notes.at(-1)?.text).toContain('github.com did not answer.');
  });

  it('launcher too old: Install is off and the note names the launcher that is needed', () => {
    const old = view({ ...EMPTY, remote: { kind: 'launcher_too_old', required: '0.3.0' } });
    expect(old.cta).toEqual({ style: 'off', word: 'Install', sub: 'launcher too old' });
    expect(old.notes.at(-1)?.text).toContain('Get launcher 0.3.0 or newer');
  });

  it('launcher too old with the mod installed: Play still works', () => {
    const old = view({ remote: { kind: 'launcher_too_old', required: '0.3.0' } });
    expect(old.cta.action).toEqual({ type: 'play' });
  });
});

describe('Steam busy', () => {
  const busy = { dir: 'D:\\Games\\Golf With Your Friends', build_id: '20236533', steam_busy: true };

  it('Install waits, with the reason on the button and on the footer', () => {
    const waiting = view({ ...EMPTY, game: busy });
    expect(waiting.cta).toEqual({ style: 'off', word: 'Install', sub: 'Steam is busy' });
    expect(waiting.locked).toContain('Steam is busy');
    expect(waiting.notes.at(-1)?.label).toBe('Steam is busy');
  });

  it('Play is not blocked', () => {
    expect(view({ game: busy }).cta.action).toEqual({ type: 'play' });
  });
});

describe('the warnings', () => {
  const all: Warning[] = [
    { kind: 'game_build_untested', build: '20999999' },
    { kind: 'foreign_bepinex' },
    { kind: 'slow_next_start' },
    { kind: 'bepinex_differs', installed: '6.0.0-be.788', wanted: '6.0.0-be.800' },
  ];

  it('each one has a note of its own', () => {
    const texts = all.map(warning => warningNote(warning).text);
    expect(texts[0]).toContain('20999999');
    expect(texts[1]).toContain('never replaces it');
    expect(texts[2]).toContain('takes a few minutes again');
    expect(texts[3]).toContain('names BepInEx 6.0.0-be.800. This game has 6.0.0-be.788');
    expect(new Set(all.map(warning => warningNote(warning).label)).size).toBe(4);
  });

  it('they follow the note of the state and stop nothing', () => {
    const warned = view({ status: { kind: 'first_run_needed' }, warnings: all });
    expect(warned.notes).toHaveLength(5);
    expect(warned.notes[0]?.label).toBe('First start');
    expect(warned.cta.action).toEqual({ type: 'play' });
  });

  it('they stay out of the way while an install runs', () => {
    const installing = view({
      status: { kind: 'installing', phase: 'checking' },
      warnings: all,
    });
    expect(installing.notes).toHaveLength(1);
  });
});

describe('a command that failed', () => {
  const error = 'The download does not match its checksum, so it was not installed';

  it('its sentence is shown next to the status, with Retry and Copy details', () => {
    const failed = view(EMPTY, { error, failed: { type: 'install' } });
    expect(failed.sentence).toBe(`${error}.`);
    expect(failed.fact).toBe('something went wrong');
    expect(failed.cta).toMatchObject({ word: 'Retry', action: { type: 'install' } });
    expect(failed.links.map(link => link.label)).toEqual(['Copy details', 'Dismiss']);
    // The status under it is still what the disk says.
    expect(failed.flag).toBe(false);
  });

  it('while the game runs nothing is retried', () => {
    const failed = view(
      { status: { kind: 'game_running' } },
      { error: 'Golf With Your Friends is running. Close the game first', failed: { type: 'play' } }
    );
    expect(failed.cta.action).toBeUndefined();
    expect(failed.sentence).toBe('Golf With Your Friends is running. Close the game first.');
  });

  it('is not shown over a running install', () => {
    const installing = view(
      { status: { kind: 'installing', phase: 'checking' } },
      { error, failed: { type: 'install' } }
    );
    expect(installing.fact).toBe('installing');
  });
});

describe('without a snapshot', () => {
  const none: ViewInput = {
    snapshot: null,
    progress: null,
    checking: false,
    error: null,
    failed: null,
    fatal: null,
    fresh: false,
    firstRun: false,
  };

  it('starting: nothing can be pressed yet', () => {
    expect(deriveView(none).cta).toEqual({ style: 'off', word: 'Play', sub: 'not ready' });
  });

  it('fatal: says the launcher could not start, with the reason and Retry', () => {
    const fatal = deriveView({ ...none, fatal: 'The per-user application folder was not found' });
    expect(fatal.sentence).toBe('The launcher could not start.');
    expect(fatal.detail).toBe('The per-user application folder was not found.');
    expect(fatal.cta.action).toEqual({ type: 'look_again' });
  });
});

it('Copy details names the sentence, the versions and the folder', () => {
  const text = detailsText({
    snapshot: EMPTY,
    progress: null,
    checking: false,
    error: 'The game folder is not writable (D:\\Games\\Golf With Your Friends\\winhttp.dll)',
    failed: { type: 'install' },
    fatal: null,
    fresh: false,
    firstRun: false,
  });
  expect(text.split('\n')).toEqual([
    'The game folder is not writable (D:\\Games\\Golf With Your Friends\\winhttp.dll).',
    'OA Mod Launcher 0.1.0 on Windows',
    'Status: not_installed',
    'OA Mod: not installed',
    'BepInEx: no BepInEx',
    'Game folder: D:\\Games\\Golf With Your Friends',
    'Game build: 20236533',
    'Update check: update_available',
  ]);
});
