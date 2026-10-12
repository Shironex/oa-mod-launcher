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
import { detailsText, type Action } from './view';

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
}

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

  return {
    snapshot: null,
    progress: null,
    checking: false,
    error: null,
    failed: null,
    fatal: null,
    fresh: false,
    firstRun: false,
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

      try {
        await events.installProgress.listen(event => set({ progress: event.payload }));
        await events.snapshotChanged.listen(event => accept(event.payload));
        const snapshot = await commands.snapshot();
        accept(snapshot);
        if (snapshot.check_on_start) await check();
      } catch (error) {
        set({ fatal: message(error) });
      }
    },

    run: async action => {
      switch (action.type) {
        case 'clear_error':
          set({ error: null, failed: null });
          return;
        case 'copy_details':
          await navigator.clipboard.writeText(detailsText(get())).catch(() => undefined);
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
  };
});
