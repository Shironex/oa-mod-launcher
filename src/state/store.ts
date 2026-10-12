// The one store of the page: the last snapshot from the Rust side, the
// progress of a running install, and the actions the buttons call.
//
// The page decides nothing about the game folder. Every action calls a command
// and stores the snapshot it answers with.

import { create } from 'zustand';
import {
  commands,
  events,
  type Folder,
  type Progress,
  type RemovalOutcome,
  type RemovalPlan,
  type Snapshot,
} from '../bindings';
import type { Preview } from '../dev/preview';
import { detailsText, type Action, type LauncherUpdateState } from './view';

/** The dialog that covers the window, if any. */
export type Dialog = 'settings' | 'about' | 'remove' | null;

interface LauncherStore {
  snapshot: Snapshot | null;
  progress: Progress | null;
  checking: boolean;
  /** Why the last command failed, and which one it was. */
  error: string | null;
  failed: Action | null;
  fatal: string | null;
  /** The running install began in a game folder without BepInEx. */
  fresh: boolean;
  /** The game was started while its first, slow start was still ahead. */
  firstRun: boolean;
  /** The launcher's own update. */
  launcher: LauncherUpdateState;
  /** The launcher version whose tag was put away since this launcher started. */
  dismissedLauncher: string | null;
  dialog: Dialog;
  /** Set while the page shows a fixed state from `src/dev/preview.ts`. */
  preview: Preview | null;

  /** Read the local state, subscribe to events, and check for updates. */
  start: () => Promise<void>;
  run: (action: Action) => Promise<void>;
  openDialog: (dialog: Dialog) => void;
  setCheckOnStart: (enabled: boolean) => Promise<void>;
  resetGameFolder: () => Promise<void>;
  openFolder: (folder: Folder) => Promise<void>;
  /** The three steps of the remove dialog. Each rejects with a sentence. */
  removalPlan: () => Promise<RemovalPlan>;
  removeMod: () => Promise<void>;
  removeAll: (includeUnrecognised: boolean) => Promise<RemovalOutcome>;
  /** `quiet` keeps a failed check to itself: nobody asked for the one on start. */
  checkLauncherUpdate: (quiet?: boolean) => Promise<void>;
  installLauncherUpdate: () => Promise<void>;
}

const NO_LAUNCHER_UPDATE: LauncherUpdateState = {
  update: null,
  checking: false,
  installing: false,
  progress: null,
  error: null,
};

/** A rejected command carries a sentence from the Rust side. */
export function message(error: unknown): string {
  return typeof error === 'string' ? error : 'Something went wrong in the launcher.';
}

/**
 * Whether the game is in its first, slow start. The snapshot cannot say so
 * while the game runs, so the page remembers that the start came from the
 * state that warns about it.
 */
export function firstRunAfter(before: boolean, snapshot: Snapshot): boolean {
  if (snapshot.status.kind === 'first_run_needed') return true;
  return snapshot.status.kind === 'game_running' ? before : false;
}

export const useLauncher = create<LauncherStore>((set, get) => {
  // React runs effects twice in development. One subscription and one update
  // check are enough.
  let started = false;

  const accept = (snapshot: Snapshot) =>
    set(state => ({ snapshot, firstRun: firstRunAfter(state.firstRun, snapshot) }));

  /** Read the disk again after something failed half way. */
  const reload = async () => {
    try {
      accept(await commands.snapshot());
    } catch {
      // The failure that led here is already on screen.
    }
  };

  /** Call a command that answers with a snapshot. A failure is kept for Retry. */
  const attempt = async (action: Action, call: () => Promise<Snapshot>) => {
    set({ error: null, failed: null });
    try {
      accept(await call());
    } catch (error) {
      set({ error: message(error), failed: action });
    }
  };

  /** The page marks a step as running itself: the command answers when it is over. */
  const mark = (status: Snapshot['status']) => {
    const before = get().snapshot;
    if (before) set({ snapshot: { ...before, status } });
  };

  const check = async () => {
    set({ checking: true });
    try {
      accept(await commands.checkForUpdates());
    } catch (error) {
      set({ error: message(error), failed: { type: 'check' } });
    } finally {
      set({ checking: false });
    }
  };

  const install = async () => {
    set({
      error: null,
      failed: null,
      progress: null,
      fresh: get().snapshot?.bepinex.kind === 'absent',
    });
    mark({ kind: 'installing', phase: 'checking' });
    try {
      accept(await commands.install());
    } catch (error) {
      set({ error: message(error), failed: { type: 'install' } });
      await reload();
    } finally {
      set({ progress: null });
    }
  };

  const lookAgain = async () => {
    set({ error: null, failed: null });
    try {
      accept(await commands.refresh());
      set({ fatal: null });
    } catch (error) {
      set({ fatal: message(error) });
    }
  };

  const setLauncher = (change: Partial<LauncherUpdateState>) =>
    set({ launcher: { ...get().launcher, ...change } });

  return {
    snapshot: null,
    progress: null,
    checking: false,
    error: null,
    failed: null,
    fatal: null,
    fresh: false,
    firstRun: false,
    launcher: NO_LAUNCHER_UPDATE,
    dismissedLauncher: null,
    dialog: null,
    preview: null,

    start: async () => {
      if (started) return;
      started = true;
      if (import.meta.env.DEV) {
        const { previewFromLocation } = await import('../dev/preview');
        const preview = previewFromLocation();
        if (preview) {
          const { dialog = null, plan: _plan, outcome: _outcome, ...state } = preview;
          set({ ...state, dialog, preview });
          return;
        }
      }

      // The setting lives in the state file. When that cannot be read the
      // setting is unknown, and the launcher's own update is still looked
      // for: a launcher in that state is the one that needs it most.
      let checkOnStart = true;
      let modCheck = false;
      try {
        await events.installProgress.listen(event => set({ progress: event.payload }));
        await events.snapshotChanged.listen(event => accept(event.payload));
        const snapshot = await commands.snapshot();
        accept(snapshot);
        checkOnStart = snapshot.check_on_start;
        modCheck = checkOnStart;
      } catch (error) {
        set({ fatal: message(error) });
      }

      // Two separate questions to two separate places, asked side by side.
      // Neither waits for the other and neither fails with the other.
      const own = (async () => {
        try {
          await events.launcherUpdateProgress.listen(event =>
            setLauncher({ progress: event.payload })
          );
        } catch {
          // Without the event the download still runs, it only shows no percent.
        }
        if (checkOnStart) await get().checkLauncherUpdate(true);
      })();
      if (modCheck) await check();
      await own;
    },

    run: async action => {
      switch (action.type) {
        case 'clear_error':
          set({ error: null, failed: null });
          return;
        case 'copy_details':
          await navigator.clipboard.writeText(detailsText(get())).catch(() => undefined);
          return;
        case 'dismiss_launcher_update':
          set({ dismissedLauncher: action.version });
          return;
      }
      // A preview has no Rust side to ask.
      if (get().preview) return;
      switch (action.type) {
        case 'check':
          return check();
        case 'install':
          return install();
        case 'look_again':
          return lookAgain();
        case 'play':
          return attempt(action, commands.startGame);
        case 'pick_folder':
          return attempt(action, commands.pickGameFolder);
        case 'open_logs':
          return get().openFolder('logs');
        case 'update_launcher':
          return get().installLauncherUpdate();
      }
    },

    openDialog: dialog => set({ dialog }),

    setCheckOnStart: async enabled => {
      if (get().preview) return;
      try {
        accept(await commands.setCheckOnStart(enabled));
      } catch (error) {
        set({ error: message(error), failed: null });
      }
    },

    resetGameFolder: async () => {
      if (get().preview) return;
      try {
        accept(await commands.resetGameFolder());
        set({ error: null, failed: null });
      } catch (error) {
        set({ error: message(error), failed: null });
      }
    },

    openFolder: async folder => {
      if (get().preview) return;
      try {
        await commands.openFolder(folder);
      } catch (error) {
        set({ error: message(error), failed: null });
      }
    },

    removalPlan: async () => {
      const { preview } = get();
      if (preview) {
        if (preview.plan) return preview.plan;
        throw 'This preview has no removal plan.';
      }
      return commands.removalPreview();
    },

    removeMod: async () => {
      if (get().preview) return;
      mark({ kind: 'removing' });
      try {
        accept(await commands.removeMod());
      } catch (error) {
        await reload();
        throw error;
      }
    },

    removeAll: async includeUnrecognised => {
      const { preview, snapshot } = get();
      if (preview) {
        if (preview.outcome) return preview.outcome;
        if (snapshot) return { snapshot, kept: [], backup_dir: null };
        throw 'This preview has no game folder.';
      }
      mark({ kind: 'removing' });
      try {
        const outcome = await commands.removeAll(includeUnrecognised);
        accept(outcome.snapshot);
        return outcome;
      } catch (error) {
        await reload();
        throw error;
      }
    },

    checkLauncherUpdate: async (quiet = false) => {
      if (get().preview || get().launcher.checking || get().launcher.installing) return;
      setLauncher({ checking: true, error: null });
      try {
        setLauncher({ update: await commands.checkLauncherUpdate(), checking: false });
      } catch (error) {
        setLauncher({ checking: false, error: quiet ? null : message(error) });
      }
    },

    installLauncherUpdate: async () => {
      if (get().preview || get().launcher.installing) return;
      setLauncher({ installing: true, progress: null, error: null });
      try {
        // Answers only when it did not work: after a download that passed its
        // check the launcher is closed and started again as the new version.
        await commands.installLauncherUpdate();
      } catch (error) {
        setLauncher({ installing: false, progress: null, error: message(error) });
      }
    },
  };
});
