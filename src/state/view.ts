// What the window shows for a given launcher state.
//
// The Rust side decides the status. This file only turns a snapshot, plus
// what the page itself knows (a running check, the sentence of a command that
// failed), into the words, the main action and the notes of the scorecard. It
// is a pure function, so every status has a test and none depends on a running
// launcher.
//
// Every state has a sentence of its own. None is told apart by colour alone.

import {
  UNKNOWN_BUILD,
  type BepInEx,
  type Phase,
  type Progress,
  type Snapshot,
  type Warning,
} from '../bindings';
import { megabytes, megabytesOf, percent, sentence } from '../lib/format';

/** Something a button on the main screen asks the store to do. */
export type Action =
  | { type: 'pick_folder' }
  | { type: 'look_again' }
  | { type: 'check' }
  | { type: 'install' }
  | { type: 'play' }
  | { type: 'open_logs' }
  | { type: 'copy_details' }
  | { type: 'clear_error' };

export interface CtaView {
  /** `primary` is the yellow button, `busy` fills with progress, `off` is disabled. */
  style: 'primary' | 'busy' | 'off';
  /** The action word, in the display face. */
  word: string;
  /** What it acts on, at the end of the putt line. */
  sub: string;
  /** How far the button is filled, 0 to 100. Only for `busy`. */
  percent?: number;
  /** Missing when the button is disabled. */
  action?: Action;
}

export interface PillView {
  tone: 'ok' | 'wait' | 'warn';
  text: string;
}

export interface NoteView {
  label: string;
  text: string;
  /** A note that reassures. The others ask for attention. */
  calm?: boolean;
}

export interface HoleView {
  title: string;
  /** `done`, the percentage of the step in play, or what the step is. */
  mark: string;
  state: 'done' | 'now' | 'next';
}

/** The right half of the card. */
export type RightView =
  /** How to find the game folder. */
  | { kind: 'find_game' }
  /** The one line the newest release came with, above the help. */
  | { kind: 'release'; title: string; tag: 'new' | 'installed' | null; notes: string }
  /** The help alone: there is nothing to say about a release. */
  | { kind: 'guide' };

export interface View {
  /** The few words at the top right of the scorecard. */
  fact: string;
  /** Where the ball lies between the tee (0) and the cup (1). */
  ball: number;
  /** The flag is red while the mod is in the game. */
  flag: boolean;
  sentence: string;
  /** A technical line under the sentence, in the monospace face. */
  detail?: string;
  cta: CtaView;
  /** Quiet actions under the main one. */
  links: { label: string; action: Action }[];
  pill?: PillView;
  /** The steps of an install, as the holes of the card. */
  holes?: HoleView[];
  notes: NoteView[];
  /** Why nothing in the game folder may be changed right now, or null. */
  locked: string | null;
  /** Whether the game folder holds anything the launcher could remove. */
  canRemove: boolean;
  right: RightView;
}

export interface ViewInput {
  snapshot: Snapshot | null;
  progress: Progress | null;
  /** An update check is running. */
  checking: boolean;
  /** Why the last command failed, as a sentence from the Rust side. */
  error: string | null;
  /** What the failed command was, so Retry can ask for it again. */
  failed: Action | null;
  /** Why the launcher could not be reached at all. */
  fatal: string | null;
  /** The running install began in a game folder without BepInEx. */
  fresh: boolean;
  /** The game was started while its first, slow start was still ahead. */
  firstRun: boolean;
}

/** The parts every status decides for itself. The rest is added after it. */
type Core = Omit<View, 'locked' | 'canRemove' | 'right'>;

const INSTALL_LOCK_NOTE: NoteView = {
  label: 'Locked',
  text: 'Install, update and remove wait until you close the game. Windows keeps the mod files locked while it runs.',
};

const STEAM_BUSY_NOTE: NoteView = {
  label: 'Steam is busy',
  text: 'Steam is downloading, updating or checking the game. Install, update and remove wait until it has finished.',
};

export function deriveView(input: ViewInput): View {
  const { snapshot, fatal } = input;
  if (fatal || !snapshot) return startView(fatal);

  const kind = snapshot.status.kind;
  let core = statusView(input, snapshot);
  const working = kind === 'installing' || kind === 'removing';

  if (!working && kind !== 'error' && kind !== 'game_not_found') {
    core = withRemote(core, snapshot, input.checking);
  }
  if (!working) {
    const notes = [...core.notes, ...snapshot.warnings.map(warningNote)];
    if (snapshot.game?.steam_busy && kind !== 'game_running') notes.push(STEAM_BUSY_NOTE);
    core = { ...core, notes };
  }
  if (input.error && !working) core = withError(core, input.error, input.failed, kind);

  return {
    ...core,
    locked: lockReason(snapshot),
    canRemove:
      snapshot.game !== null &&
      (snapshot.bepinex.kind !== 'absent' || snapshot.mod_installed !== null),
    right: rightView(snapshot),
  };
}

/** Before the first snapshot, and when the Rust side cannot be reached. */
function startView(fatal: string | null): View {
  return {
    fact: fatal ? 'launcher problem' : 'starting',
    ball: 0,
    flag: false,
    sentence: fatal ? 'The launcher could not start.' : 'Looking for the game.',
    detail: fatal ? sentence(fatal) : undefined,
    cta: fatal
      ? { style: 'primary', word: 'Retry', sub: 'try again', action: { type: 'look_again' } }
      : { style: 'off', word: 'Play', sub: 'not ready' },
    links: fatal ? [{ label: 'Copy details', action: { type: 'copy_details' } }] : [],
    notes: [],
    locked: null,
    canRemove: false,
    right: { kind: 'guide' },
  };
}

function statusView(input: ViewInput, snapshot: Snapshot): Core {
  const { status, remote, bepinex } = snapshot;
  const mod = snapshot.mod_installed;
  const newest = remote.kind === 'update_available' ? remote.version : null;
  const play: Action = { type: 'play' };

  switch (status.kind) {
    case 'error':
      return {
        fact: 'launcher problem',
        ball: 0.3,
        flag: false,
        sentence: 'The launcher cannot read what it needs. Nothing was changed.',
        detail: sentence(status.reason),
        cta: {
          style: 'primary',
          word: 'Open logs',
          sub: 'launcher folder',
          action: { type: 'open_logs' },
        },
        links: [
          { label: 'Look again', action: { type: 'look_again' } },
          { label: 'Copy details', action: { type: 'copy_details' } },
        ],
        pill: { tone: 'warn', text: 'it is tried again by itself' },
        notes: [],
      };

    case 'installing':
      return installingView(input, snapshot, status.phase);

    case 'removing':
      return {
        fact: 'removing',
        ball: 0.3,
        flag: false,
        sentence: 'Removing files from the game folder.',
        cta: { style: 'off', word: 'Removing', sub: 'please wait' },
        links: [],
        pill: { tone: 'wait', text: 'a few seconds' },
        notes: [{ label: 'Hold on', text: 'Keep this window open and do not start the game.' }],
      };

    case 'game_not_found':
      return {
        fact: 'game not found',
        ball: 0,
        flag: false,
        sentence: 'Golf With Your Friends was not found on this PC.',
        cta: {
          style: 'primary',
          word: 'Choose game folder',
          sub: 'browse',
          action: { type: 'pick_folder' },
        },
        links: [{ label: 'Look again', action: { type: 'look_again' } }],
        pill: { tone: 'warn', text: 'not in a Steam library' },
        notes: [
          { label: 'Tip', text: 'Pick the folder that has Golf With Your Friends.exe in it.' },
        ],
      };

    case 'game_running':
      if (mod && input.firstRun) {
        return {
          fact: 'game is running',
          ball: 0.94,
          flag: true,
          sentence: 'Golf With Your Friends is starting. The first start has not hung.',
          cta: { style: 'off', word: 'Setting up', sub: 'please wait' },
          links: [],
          pill: { tone: 'wait', text: 'the first start takes 2 to 5 minutes' },
          notes: [
            {
              label: 'Normal',
              text: 'A black game window and a console full of text are fine. It happens once and needs an internet connection. Do not close the game.',
            },
          ],
        };
      }
      return {
        fact: 'game is running',
        ball: 0.96,
        flag: mod !== null,
        sentence: mod
          ? 'Golf With Your Friends is running.'
          : 'Golf With Your Friends is running without the mod.',
        cta: { style: 'off', word: 'Game is running', sub: mod ? modText(mod) : 'no mod' },
        links: [],
        pill: { tone: 'wait', text: 'close the game first' },
        notes: [INSTALL_LOCK_NOTE],
      };

    case 'clean':
      return {
        fact: 'clean',
        ball: 0,
        flag: false,
        sentence: 'All clean. The game is back to how Steam installed it.',
        cta: installCta(snapshot, 'Install', installSub(newest)),
        links: [{ label: 'Play without the mod', action: play }],
        pill: { tone: 'ok', text: 'BepInEx removed · OA Mod removed' },
        notes: [
          {
            label: 'Removed',
            text: 'BepInEx, its runtime and the files that load it. Your saves were not touched.',
            calm: true,
          },
        ],
      };

    case 'not_installed':
      if (bepinex.kind === 'unsupported') {
        // Install stays within reach: what an install that was interrupted
        // left behind reads as unsupported too, and the next install clears
        // it before it looks again. Anything else stops it with this reason.
        return {
          fact: 'needs a look',
          ball: 0.04,
          flag: false,
          sentence: 'The mod loader in the game folder cannot be used as it is.',
          detail: sentence(bepinex.reason),
          cta: installCta(snapshot, 'Install', 'try anyway'),
          links: [{ label: 'Play as it is', action: play }],
          pill: { tone: 'warn', text: 'unsupported BepInEx' },
          notes: [
            {
              label: 'Way out',
              text: 'Install first clears what an interrupted install left behind. If it still stops, Remove mod puts the game back to clean, and Install works after that.',
            },
          ],
        };
      }
      if (bepinex.kind === 'absent') {
        return {
          fact: 'game found',
          ball: 0.04,
          flag: false,
          sentence: 'Golf With Your Friends is here. OA Mod is not installed yet.',
          cta: installCta(snapshot, 'Install', installSub(newest)),
          links: [{ label: 'Play without the mod', action: play }],
          pill: { tone: 'wait', text: 'nothing installed' },
          notes: [
            {
              label: 'Gets',
              text: `${
                snapshot.bepinex_wanted ? `BepInEx ${snapshot.bepinex_wanted}` : 'BepInEx'
              } and ${newest ? modText(newest) : 'OA Mod'}. Two downloads${
                remote.kind === 'update_available' ? `, ${megabytes(remote.size ?? 0)}` : ''
              }, about a minute.`,
              calm: true,
            },
          ],
        };
      }
      return {
        fact: 'game found',
        ball: 0.04,
        flag: false,
        sentence: 'OA Mod is not installed. BepInEx is already in the game folder.',
        cta: installCta(snapshot, 'Install', installSub(newest)),
        links: [{ label: 'Play without OA Mod', action: play }],
        pill: { tone: 'ok', text: `${bepinexText(bepinex)} is installed` },
        notes: [
          {
            label: 'Gets',
            text: `${newest ? modText(newest) : 'OA Mod'} only. One small download.`,
            calm: true,
          },
        ],
      };

    case 'first_run_needed':
      return {
        fact: 'ready',
        ball: 0.9,
        flag: true,
        sentence: 'Installed. The first start takes a few minutes.',
        cta: { style: 'primary', word: 'Play', sub: 'first start', action: play },
        links: [],
        pill: { tone: 'ok', text: `${modText(mod)} · ${bepinexText(bepinex)}` },
        notes: [
          {
            label: 'First start',
            text: 'BepInEx builds its files the first time. The game can stay black or look frozen for 2 to 5 minutes, and it needs an internet connection. Let it finish. It only happens once.',
          },
        ],
      };

    case 'update_available': {
      if (mod === UNKNOWN_BUILD || !newest) {
        return {
          fact: 'unknown build',
          ball: 0.7,
          flag: true,
          sentence: 'The OA Mod in the game folder is not a build this launcher installed.',
          cta: installCta(snapshot, 'Update', newest ? `to ${newest}` : 'newest release'),
          links: [{ label: 'Play anyway', action: play }],
          pill: { tone: 'ok', text: `${bepinexText(bepinex)} stays` },
          notes: [
            {
              label: 'Unknown build',
              text: 'A local build, or a file copied by hand. Update replaces it with the released one. Everyone in a match needs the same OA Mod.',
            },
          ],
        };
      }
      return {
        fact: 'update waiting',
        ball: 0.7,
        flag: true,
        sentence: `OA Mod ${newest} is out. You have ${mod}.`,
        cta: installCta(snapshot, 'Update', `${mod} → ${newest}`),
        links: [{ label: `Play ${mod} anyway`, action: play }],
        pill: { tone: 'ok', text: `${bepinexText(bepinex)} stays` },
        notes: [
          {
            label: 'Same version',
            text: 'Everyone in a match needs the same OA Mod. Update before the tournament.',
          },
        ],
      };
    }

    case 'up_to_date':
      return {
        fact: 'ready',
        ball: 0.96,
        flag: true,
        sentence:
          remote.kind === 'up_to_date'
            ? `${modText(mod)} is installed and up to date.`
            : `${modText(mod)} is installed.`,
        cta: { style: 'primary', word: 'Play', sub: modText(mod), action: play },
        links: [],
        pill: {
          tone: 'ok',
          text:
            remote.kind === 'up_to_date'
              ? `${bepinexText(bepinex)} · checked`
              : bepinexText(bepinex),
        },
        notes: [],
      };
  }
}

/** The two layouts of the scorecard holes, and where each hole ends in the total. */
const FRESH = {
  phases: ['downloading_bepinex', 'installing_bepinex', 'downloading_mod', 'installing_mod'],
  holes: [
    ['BepInEx', 'download'],
    ['Unpack', 'BepInEx'],
    ['OA Mod', 'download'],
    ['Place', 'in the game'],
  ],
  ends: [40, 55, 90, 100],
} as const;
const MOD_ONLY = {
  phases: ['downloading_mod', 'installing_mod'],
  holes: [
    ['OA Mod', 'download'],
    ['Place', 'in the game'],
  ],
  ends: [85, 100],
} as const;

/** The steps of an install in the order they run. */
const PHASES: readonly Phase[] = [
  'checking',
  'downloading_bepinex',
  'installing_bepinex',
  'downloading_mod',
  'installing_mod',
];

function installingView(input: ViewInput, snapshot: Snapshot, polled: Phase): Core {
  // The snapshot learns the step from the poll, every few seconds. A progress
  // report arrives at once, so the later of the two is where the install is.
  const reported = input.progress?.phase ?? polled;
  const phase = PHASES.indexOf(reported) > PHASES.indexOf(polled) ? reported : polled;
  // A BepInEx step proves a fresh install even when nobody said so.
  const fresh = input.fresh || phase === 'downloading_bepinex' || phase === 'installing_bepinex';
  const plan = fresh ? FRESH : MOD_ONLY;
  const now = (plan.phases as readonly Phase[]).indexOf(phase);
  const newest = snapshot.remote.kind === 'update_available' ? snapshot.remote.version : null;

  // A report of an earlier step says nothing about this one.
  const progress = input.progress?.phase === phase ? input.progress : null;
  const received = progress?.received ?? 0;
  const total = progress?.total ?? 0;
  const part = percent(received, total);
  const from = now > 0 ? (plan.ends[now - 1] ?? 0) : 0;
  const done = now < 0 ? 0 : Math.round(from + (((plan.ends[now] ?? 100) - from) * part) / 100);

  const sentences: Record<Phase, string> = {
    checking: 'Reading the newest release.',
    downloading_bepinex: 'Downloading BepInEx.',
    installing_bepinex: 'Unpacking BepInEx into the game folder.',
    downloading_mod: `Downloading ${newest ? modText(newest) : 'OA Mod'}.`,
    installing_mod: 'Putting OA Mod into the game folder.',
  };
  const step = now < 0 ? 'getting ready' : `step ${now + 1} of ${plan.holes.length}`;

  return {
    fact: 'installing',
    ball: (done / 100) * 0.9,
    flag: false,
    sentence: sentences[phase],
    cta: {
      style: 'busy',
      word: !fresh && snapshot.mod_installed ? 'Updating' : 'Installing',
      sub: `${done}%`,
      percent: done,
    },
    links: [],
    pill: { tone: 'wait', text: total > 0 ? `${step} · ${megabytesOf(received, total)}` : step },
    holes: plan.holes.map(([title, what], index) => ({
      title: `${index + 1} · ${title}`,
      mark: index < now ? 'done' : index === now ? (total > 0 ? `${part}%` : 'working') : what,
      state: index < now ? 'done' : index === now ? 'now' : 'next',
    })),
    notes: [
      {
        label: 'Hold on',
        text: fresh
          ? 'Keep this window open. It takes about a minute.'
          : 'Keep this window open. It takes a few seconds.',
      },
    ],
  };
}

/** What the last update check adds to a state: a failed check, or a launcher that is too old. */
function withRemote(core: Core, snapshot: Snapshot, checking: boolean): Core {
  const { remote } = snapshot;
  // While the game runs nothing can be pressed, a new check included.
  const running = snapshot.status.kind === 'game_running';
  if (checking) return { ...core, pill: { tone: 'wait', text: 'checking for updates' } };
  if (remote.kind === 'offline') {
    return {
      ...core,
      links: running
        ? core.links
        : [...core.links, { label: 'Check again', action: { type: 'check' } }],
      notes: [
        ...core.notes,
        {
          label: 'Offline',
          text: `${sentence(remote.reason)} What is installed still works.`,
        },
      ],
    };
  }
  if (remote.kind === 'launcher_too_old') {
    return {
      ...core,
      notes: [
        ...core.notes,
        {
          label: 'Launcher',
          text: `This launcher (${snapshot.launcher_version}) is too old for the newest OA Mod. Get launcher ${remote.required} or newer from the page you got this one from.`,
        },
      ],
    };
  }
  return core;
}

/**
 * A command that failed: its sentence takes the place of the status sentence,
 * and Retry asks for the same thing again. The status under it is still what
 * the disk says. While the game runs nothing can be retried, so the sentence
 * is shown and the button stays off.
 */
function withError(
  core: Core,
  error: string,
  failed: Action | null,
  kind: Snapshot['status']['kind']
): Core {
  const retry = failed && kind !== 'game_running';
  return {
    ...core,
    fact: 'something went wrong',
    sentence: sentence(error),
    detail: undefined,
    cta: retry ? { style: 'primary', word: 'Retry', sub: 'try again', action: failed } : core.cta,
    links: [
      { label: 'Copy details', action: { type: 'copy_details' } },
      { label: 'Dismiss', action: { type: 'clear_error' } },
    ],
    pill: { tone: 'warn', text: 'not finished' },
  };
}

/** The main action of the states that install: off, with the reason, when it cannot run. */
function installCta(snapshot: Snapshot, word: string, sub: string): CtaView {
  if (snapshot.game?.steam_busy) return { style: 'off', word, sub: 'Steam is busy' };
  if (snapshot.remote.kind === 'launcher_too_old') {
    return { style: 'off', word, sub: 'launcher too old' };
  }
  return { style: 'primary', word, sub, action: { type: 'install' } };
}

function installSub(newest: string | null): string {
  return newest ? modText(newest) : 'newest release';
}

/** Why install, update, remove and the folder choice are out of reach, or null. */
function lockReason(snapshot: Snapshot): string | null {
  switch (snapshot.status.kind) {
    case 'installing':
      return 'An install is running.';
    case 'removing':
      return 'A removal is running.';
    case 'game_running':
      return 'Close the game first.';
    default:
      return snapshot.game?.steam_busy
        ? 'Steam is busy with the game. Wait until it has finished.'
        : null;
  }
}

function rightView(snapshot: Snapshot): RightView {
  if (snapshot.status.kind === 'game_not_found') return { kind: 'find_game' };
  if (!snapshot.notes) return { kind: 'guide' };
  const { remote } = snapshot;
  const mod = snapshot.mod_installed;
  if (remote.kind === 'update_available') {
    return { kind: 'release', title: modText(remote.version), tag: 'new', notes: snapshot.notes };
  }
  if (remote.kind === 'up_to_date' && mod && mod !== UNKNOWN_BUILD) {
    return { kind: 'release', title: modText(mod), tag: 'installed', notes: snapshot.notes };
  }
  return { kind: 'release', title: 'The newest OA Mod', tag: null, notes: snapshot.notes };
}

/** `OA Mod 0.1.0`, or the words for a build without a known version. */
function modText(version: string | null): string {
  if (!version) return 'OA Mod';
  return version === UNKNOWN_BUILD ? 'OA Mod (unknown build)' : `OA Mod ${version}`;
}

function bepinexText(bepinex: BepInEx): string {
  switch (bepinex.kind) {
    case 'ours':
    case 'adopted':
      return `BepInEx ${bepinex.label}`;
    case 'foreign':
      return 'BepInEx (another build)';
    case 'unsupported':
      return 'unsupported BepInEx';
    case 'absent':
      return 'no BepInEx';
  }
}

/** One note for something the player should know that stops nothing. */
export function warningNote(warning: Warning): NoteView {
  switch (warning.kind) {
    case 'game_build_untested':
      return {
        label: 'Untested',
        text: `OA Mod was not tried with this game build (${warning.build}). It may work anyway. If the game misbehaves, look for a newer OA Mod.`,
      };
    case 'foreign_bepinex':
      return {
        label: 'Other BepInEx',
        text: 'The game folder has a BepInEx build this launcher does not know. OA Mod goes into it as it is, and the launcher never replaces it.',
      };
    case 'slow_next_start':
      return {
        label: 'Slow start',
        text: 'The game was updated, so the next start takes a few minutes again while BepInEx rebuilds its files.',
      };
    case 'bepinex_differs':
      return {
        label: 'BepInEx',
        text: `The newest OA Mod names BepInEx ${warning.wanted}. This game has ${warning.installed}, and the launcher keeps it.`,
      };
  }
}

/**
 * What Copy details puts on the clipboard: the sentence, and the facts
 * somebody helping needs to see next to it.
 */
export function detailsText(input: ViewInput): string {
  const { snapshot } = input;
  const view = deriveView(input);
  const lines = [view.sentence, ...(view.detail ? [view.detail] : [])];
  if (snapshot) {
    lines.push(
      `OA Mod Launcher ${snapshot.launcher_version} on Windows`,
      `Status: ${snapshot.status.kind}`,
      `OA Mod: ${snapshot.mod_installed ?? 'not installed'}`,
      `BepInEx: ${bepinexText(snapshot.bepinex)}`,
      `Game folder: ${snapshot.game?.dir ?? 'not found'}`,
      `Game build: ${snapshot.game?.build_id ?? 'unknown'}`,
      `Update check: ${snapshot.remote.kind}`
    );
  }
  return lines.join('\n');
}
