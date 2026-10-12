import { useCallback, useEffect, type CSSProperties } from 'react';
import { RemoveModal } from './components/RemoveModal';
import { Scorecard } from './components/Scorecard';
import { SettingsModal } from './components/SettingsModal';
import { TitleBar } from './components/TitleBar';
import { useLauncher, type Dialog } from './state/store';
import { deriveView, type Action, type ViewInput } from './state/view';

/** What the window is drawn from: the input of the view, and the open dialog. */
export interface WindowState extends ViewInput {
  dialog: Dialog;
}

/** The store, connected to the window. */
export function App() {
  const snapshot = useLauncher(store => store.snapshot);
  const progress = useLauncher(store => store.progress);
  const checking = useLauncher(store => store.checking);
  const error = useLauncher(store => store.error);
  const failed = useLauncher(store => store.failed);
  const fatal = useLauncher(store => store.fatal);
  const fresh = useLauncher(store => store.fresh);
  const firstRun = useLauncher(store => store.firstRun);
  const launcher = useLauncher(store => store.launcher);
  const dismissedLauncher = useLauncher(store => store.dismissedLauncher);
  const dialog = useLauncher(store => store.dialog);
  const start = useLauncher(store => store.start);

  useEffect(() => {
    void start();
  }, [start]);

  return (
    <Window
      state={{
        snapshot,
        progress,
        checking,
        error,
        failed,
        fatal,
        fresh,
        firstRun,
        launcher,
        dismissedLauncher,
        dialog,
      }}
    />
  );
}

/** The window for one state. It reads nothing but its argument. */
export function Window({ state }: { state: WindowState }) {
  const { snapshot, dialog } = state;
  const run = useLauncher(store => store.run);
  const openDialog = useLauncher(store => store.openDialog);
  const openFolder = useLauncher(store => store.openFolder);

  const view = deriveView(state);
  const onAction = useCallback((action: Action) => void run(action), [run]);

  return (
    // `--b` is where the ball lies between the tee and the cup.
    <div className="win" style={{ '--b': view.ball } as CSSProperties}>
      <div className={view.flag ? 'scene' : 'scene noflag'}>
        <i className="sun" />
        <i className="hill-a" />
        <i className="hill-b" />
        <i className="putt" />
        <i className="cup" />
        <i className="pole" />
        <i className="pennant" />
        <i className="ball" />
      </div>
      <TitleBar />

      {/* While a dialog is open the window behind it is out of reach. */}
      <main className="desk" inert={dialog !== null}>
        <Scorecard
          view={view}
          snapshot={snapshot}
          onAction={onAction}
          onOpenDialog={openDialog}
          onOpenFolder={folder => void openFolder(folder)}
        />
      </main>

      {(dialog === 'settings' || dialog === 'about') && (
        <SettingsModal
          section={dialog}
          snapshot={snapshot}
          launcher={state.launcher}
          locked={view.locked}
          canRemove={view.canRemove}
          error={state.error}
        />
      )}
      {dialog === 'remove' && <RemoveModal />}
    </div>
  );
}
