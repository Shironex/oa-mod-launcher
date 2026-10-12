// What the remove dialog says for a removal plan from the Rust side.
//
// The plan decides what is deleted. This file only words it: which choice is
// selected first, the rows of the list, and whether a full removal has to be
// confirmed a second time. Pure functions, so each has a test.

import type { ForeignItem, RemovalOutcome, RemovalPlan } from '../bindings';
import { bytes } from '../lib/format';

/** `all` removes everything BepInEx brought, `mod` only OA Mod. */
export type RemovalMode = 'all' | 'mod';

export interface RemovalRow {
  name: string;
  what: string;
  /** The row names something that stays. */
  keep?: boolean;
}

const WHAT: Record<string, string> = {
  BepInEx: 'OA Mod, every other plugin, all plugin settings, logs',
  dotnet: 'the runtime BepInEx brought along',
  'winhttp.dll': 'what loads BepInEx when the game starts',
  'doorstop_config.ini': 'its settings',
  '.doorstop_version': 'its version stamp',
  'changelog.txt': 'the BepInEx changelog',
};

/** A path of the plan as Windows shows it. */
export function shown(path: string): string {
  return path.replaceAll('/', '\\');
}

/** Whether the plan holds anything "remove only OA Mod" would delete. */
export function hasMod(plan: RemovalPlan): boolean {
  return plan.mod_files.length > 0;
}

/** Whether the plan holds anything at all to delete. */
export function hasAnything(plan: RemovalPlan): boolean {
  return (
    hasMod(plan) ||
    plan.folders.length > 0 ||
    plan.root_files.length > 0 ||
    plan.unrecognised_root_files.length > 0
  );
}

/**
 * The choice that is selected when the dialog opens. It follows the plan:
 * with other people's files there, or a BepInEx the launcher did not install,
 * the careful choice comes first. Without the mod there is only one choice.
 */
export function defaultMode(plan: RemovalPlan): RemovalMode {
  return plan.mod_only_recommended && hasMod(plan) ? 'mod' : 'all';
}

/**
 * Whether "remove everything" asks a second time: when it would take
 * somebody else's plugins along, or a BepInEx the launcher did not install.
 */
export function needsSecondConfirmation(plan: RemovalPlan): boolean {
  return plan.foreign.length > 0 || plan.mod_only_recommended;
}

/** The plugins and patchers of other people, by name, without their settings files. */
export function foreignNames(plan: RemovalPlan): string[] {
  return plan.foreign
    .filter(item => item.kind !== 'config')
    .map(item => item.path.split('/').at(-1) ?? item.path);
}

function foreignRow(item: ForeignItem): RemovalRow {
  const kind = { plugin: 'another plugin', patcher: 'a patcher', config: 'settings' }[item.kind];
  return {
    name: shown(item.path),
    what: `${kind}, ${bytes(item.bytes ?? 0)}: copied to the backups first, then deleted`,
  };
}

/** Other people's files, each with what happens to it. */
export function foreignRows(plan: RemovalPlan): RemovalRow[] {
  return plan.foreign.map(foreignRow);
}

/**
 * Whether "remove everything" would delete nothing as it stands: all there is
 * are root files the launcher does not recognise, and they are not included.
 */
export function nothingSelected(plan: RemovalPlan, includeUnrecognised: boolean): boolean {
  return (
    !includeUnrecognised &&
    !hasMod(plan) &&
    plan.folders.length === 0 &&
    plan.root_files.length === 0
  );
}

/** The list under the two choices. It follows the choice. */
export function removalRows(
  plan: RemovalPlan,
  mode: RemovalMode,
  includeUnrecognised: boolean
): RemovalRow[] {
  if (mode === 'mod') {
    return [
      ...plan.mod_files.map(file => ({ name: shown(file), what: 'deleted' })),
      { name: 'BepInEx and other plugins', what: 'kept, with their settings', keep: true },
    ];
  }
  return [
    ...plan.folders.map(folder => ({
      name: `${shown(folder)}\\`,
      what: WHAT[folder] ?? 'deleted with everything in it',
    })),
    ...plan.root_files.map(file => ({ name: file, what: WHAT[file] ?? 'deleted' })),
    ...plan.unrecognised_root_files.map(file => {
      const withProxy = plan.kept_with_proxy.includes(file);
      if (includeUnrecognised) {
        return {
          name: file,
          what: withProxy
            ? 'deleted too, together with winhttp.dll'
            : 'deleted too, although the launcher does not recognise it',
        };
      }
      return { name: file, what: `kept: ${keptReason(plan, file)}`, keep: true };
    }),
    ...plan.kept.map(name => ({
      name: shown(name),
      what: 'kept: it is not what its name says',
      keep: true,
    })),
    ...foreignRows(plan),
  ];
}

/**
 * Why a full removal leaves `name` in the game folder. The loader files are
 * ones the launcher knows, and they stay all the same next to a `winhttp.dll`
 * it does not recognise: that file reads them.
 */
export function keptReason(plan: RemovalPlan, name: string): string {
  if (plan.kept_with_proxy.includes(name)) {
    return 'it belongs to winhttp.dll, which this launcher does not recognise';
  }
  if (plan.unrecognised_root_files.includes(name)) return 'not a file this launcher recognises';
  return 'it is not what its name says';
}

/**
 * What the tick box says under its first line: why the files stay, and what
 * staying means.
 */
export function unrecognisedHelp(plan: RemovalPlan): string {
  const several = plan.unrecognised_root_files.length > 1;
  const withProxy = plan.kept_with_proxy;
  const why =
    withProxy.length > 0
      ? `The launcher does not recognise winhttp.dll, so it stays unless you tick this, and ${withProxy.join(
          ' and '
        )} ${withProxy.length > 1 ? 'stay' : 'stays'} with it: winhttp.dll reads ${
          withProxy.length > 1 ? 'them' : 'it'
        }, so they go or stay together.`
      : several
        ? 'The launcher does not recognise these files, so they stay unless you tick this.'
        : 'The launcher does not recognise this file, so it stays unless you tick this.';
  return `${why} While ${several ? 'they stay' : 'it stays'} without BepInEx, the launcher reports an unsupported BepInEx, until a second removal includes ${
    several ? 'them' : 'it'
  }.`;
}

/**
 * Whether a full removal left root files of BepInEx behind that the launcher
 * did not recognise, or that stayed with a `winhttp.dll` it did not
 * recognise. While they stay, the launcher reports an unsupported BepInEx,
 * until a second removal includes them.
 */
export function leftUnrecognised(plan: RemovalPlan, outcome: RemovalOutcome): string[] {
  return outcome.kept.filter(name => plan.unrecognised_root_files.includes(name));
}
