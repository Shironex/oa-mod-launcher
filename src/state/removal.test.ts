import { describe, expect, it } from 'vitest';
import type { RemovalPlan } from '../bindings';
import {
  defaultMode,
  foreignNames,
  hasAnything,
  leftUnrecognised,
  needsSecondConfirmation,
  removalRows,
} from './removal';

/** BepInEx is the launcher's own and nobody else's files are there. */
const OURS: RemovalPlan = {
  game_dir: 'D:\\Games\\Golf With Your Friends',
  bepinex: { kind: 'ours', label: '6.0.0-be.788' },
  mod_files: ['BepInEx/plugins/OaMod/OaMod.dll', 'BepInEx/config/com.shirone.oamod.cfg'],
  folders: ['dotnet', 'BepInEx'],
  root_files: ['winhttp.dll', 'doorstop_config.ini', '.doorstop_version', 'changelog.txt'],
  unrecognised_root_files: [],
  kept: [],
  foreign: [],
  mod_only_recommended: false,
};

const FOREIGN: RemovalPlan = {
  ...OURS,
  foreign: [
    { path: 'BepInEx/plugins/GhostCaddy', kind: 'plugin', bytes: 1_482_752 },
    { path: 'BepInEx/config/com.shironex.ghostcaddy.cfg', kind: 'config', bytes: 9_216 },
  ],
  mod_only_recommended: true,
};

describe('the choice that is selected first', () => {
  it('follows the plan: everything when the launcher owns it all', () => {
    expect(defaultMode(OURS)).toBe('all');
    expect(needsSecondConfirmation(OURS)).toBe(false);
  });

  it('is "only OA Mod" when the plan recommends it', () => {
    expect(defaultMode(FOREIGN)).toBe('mod');
  });

  it('is "everything" when there is no mod to remove alone', () => {
    expect(defaultMode({ ...FOREIGN, mod_files: [] })).toBe('all');
  });
});

describe('the second confirmation of a full removal', () => {
  it('is asked for when other plugins would go, and names them', () => {
    expect(needsSecondConfirmation(FOREIGN)).toBe(true);
    // The settings file is in the list of the dialog, not among the names.
    expect(foreignNames(FOREIGN)).toEqual(['GhostCaddy']);
  });

  it('is asked for when BepInEx is not the launcher own, also without plugins', () => {
    expect(needsSecondConfirmation({ ...OURS, mod_only_recommended: true })).toBe(true);
  });
});

describe('the list', () => {
  it('everything: folders, root files, and other plugins with where they are copied', () => {
    const rows = removalRows(FOREIGN, 'all', false);
    expect(rows.map(row => row.name)).toEqual([
      'dotnet\\',
      'BepInEx\\',
      'winhttp.dll',
      'doorstop_config.ini',
      '.doorstop_version',
      'changelog.txt',
      'BepInEx\\plugins\\GhostCaddy',
      'BepInEx\\config\\com.shironex.ghostcaddy.cfg',
    ]);
    expect(rows[6]?.what).toBe('another plugin, 1.4 MB: copied to the backups first, then deleted');
    expect(rows.some(row => row.keep)).toBe(false);
  });

  it('only OA Mod: its files, and everything else is kept', () => {
    const rows = removalRows(FOREIGN, 'mod', false);
    expect(rows).toEqual([
      { name: 'BepInEx\\plugins\\OaMod\\OaMod.dll', what: 'deleted' },
      { name: 'BepInEx\\config\\com.shirone.oamod.cfg', what: 'deleted' },
      { name: 'BepInEx and other plugins', what: 'kept, with their settings', keep: true },
    ]);
  });

  it('unrecognised root files are kept unless they are included', () => {
    const plan = { ...OURS, root_files: [], unrecognised_root_files: ['winhttp.dll'] };
    expect(removalRows(plan, 'all', false).at(-1)).toMatchObject({
      name: 'winhttp.dll',
      keep: true,
    });
    expect(removalRows(plan, 'all', true).at(-1)?.keep).toBeUndefined();
  });

  it('what is not what its name says is always kept', () => {
    const rows = removalRows({ ...OURS, folders: ['BepInEx'], kept: ['dotnet'] }, 'all', true);
    expect(rows.at(-1)).toEqual({
      name: 'dotnet',
      what: 'kept: it is not what its name says',
      keep: true,
    });
  });
});

it('an empty plan has nothing to remove', () => {
  const empty = { ...OURS, mod_files: [], folders: [], root_files: [] };
  expect(hasAnything(empty)).toBe(false);
  expect(hasAnything({ ...empty, unrecognised_root_files: ['winhttp.dll'] })).toBe(true);
});

it('after a full removal, the kept root files the launcher did not recognise are named', () => {
  const plan = { ...OURS, unrecognised_root_files: ['winhttp.dll'], kept: ['dotnet'] };
  const snapshot = {
    launcher_version: '0.1.0',
    status: { kind: 'not_installed' as const },
    game: null,
    bepinex: { kind: 'absent' as const },
    mod_installed: null,
    foreign_plugins: [],
    remote: { kind: 'unknown' as const },
    warnings: [],
    notes: null,
    check_on_start: true,
  };
  expect(
    leftUnrecognised(plan, { snapshot, kept: ['dotnet', 'winhttp.dll'], backup_dir: null })
  ).toEqual(['winhttp.dll']);
  expect(leftUnrecognised(plan, { snapshot, kept: ['dotnet'], backup_dir: null })).toEqual([]);
});
