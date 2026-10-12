import type { Snapshot } from '../bindings';
import { percent, sentence } from '../lib/format';
import { useLauncher } from '../state/store';
import { launcherStatusText, type LauncherUpdateState } from '../state/view';
import { Icon, type IconName } from './Icon';
import { Modal } from './Modal';

type Section = 'settings' | 'about';

const SECTIONS: { id: Section; label: string; icon: IconName }[] = [
  { id: 'settings', label: 'Game and mod', icon: 'gear' },
  { id: 'about', label: 'About', icon: 'info' },
];

interface SettingsModalProps {
  section: Section;
  snapshot: Snapshot | null;
  /** The launcher's own update. Left out, the row only names the version. */
  launcher?: LauncherUpdateState;
  /** Why nothing in the game folder may be changed right now, or null. */
  locked: string | null;
  /** Whether the game folder holds anything the launcher could remove. */
  canRemove: boolean;
  /** The sentence of a command that failed, such as a folder that was refused. */
  error: string | null;
}

/** Settings: where the game lives, the update check, the way to the removal, and About. */
export function SettingsModal({
  section,
  snapshot,
  launcher,
  locked,
  canRemove,
  error,
}: SettingsModalProps) {
  const openDialog = useLauncher(store => store.openDialog);
  const run = useLauncher(store => store.run);
  const setCheckOnStart = useLauncher(store => store.setCheckOnStart);
  const resetGameFolder = useLauncher(store => store.resetGameFolder);
  const openFolder = useLauncher(store => store.openFolder);
  const checkOnStart = snapshot?.check_on_start ?? true;
  const version = snapshot?.launcher_version;

  const side = (
    <>
      <h2>Settings</h2>
      <div role="tablist" aria-label="Settings sections" aria-orientation="vertical">
        {SECTIONS.map(entry => (
          <button
            type="button"
            role="tab"
            key={entry.id}
            className="m-item"
            aria-selected={entry.id === section}
            onClick={() => openDialog(entry.id)}
          >
            <Icon name={entry.icon} />
            {entry.label}
          </button>
        ))}
      </div>
      {version && <span className="m-foot">launcher {version}</span>}
    </>
  );

  return (
    <Modal label="Settings" onClose={() => openDialog(null)} side={side}>
      {section === 'settings' ? (
        <div role="tabpanel">
          <h2 className="m-h">Game and mod</h2>
          <p className="m-sub">Where the game lives and how the mod stays current.</p>
          <div>
            <div className="m-row">
              <div>
                <b>Game folder</b>
                <span className="path sel">
                  {snapshot?.game?.dir ?? 'Golf With Your Friends was not found.'}
                </span>
                {locked && <span>{locked}</span>}
                {error && (
                  <span className="fail" role="alert">
                    {sentence(error)}
                  </span>
                )}
              </div>
              <span className="side">
                <button
                  type="button"
                  className="sbtn"
                  disabled={locked !== null}
                  onClick={() => void run({ type: 'pick_folder' })}
                >
                  <Icon name="folder" />
                  Change
                </button>
                {/* Only when a folder was chosen by hand: otherwise there is nothing to forget. */}
                {snapshot?.game_dir_chosen && (
                  <button
                    type="button"
                    className="sbtn"
                    disabled={locked !== null}
                    title="Forget the folder chosen by hand and use the one Steam names"
                    onClick={() => void resetGameFolder()}
                  >
                    Reset to Steam
                  </button>
                )}
              </span>
            </div>
            <div className="m-row">
              <div>
                <b id="check-on-start">Look for a new OA Mod on start</b>
                <span>Reads the newest release each time the launcher opens.</span>
              </div>
              <button
                type="button"
                role="switch"
                className="tog"
                aria-checked={checkOnStart}
                aria-labelledby="check-on-start"
                onClick={() => void setCheckOnStart(!checkOnStart)}
              >
                {checkOnStart ? 'On' : 'Off'}
                <i />
              </button>
            </div>
            <div className="m-row">
              <div>
                <b>Put the game back to clean</b>
                <span>
                  {canRemove
                    ? 'Deletes OA Mod, or BepInEx with it. Asks first and shows the list.'
                    : 'There is nothing in the game folder to remove.'}
                </span>
              </div>
              <button
                type="button"
                className="sbtn quiet"
                disabled={!canRemove || locked !== null}
                onClick={() => openDialog('remove')}
              >
                Remove mod
              </button>
            </div>
          </div>
        </div>
      ) : (
        <div role="tabpanel">
          <h2 className="m-h">About</h2>
          <p className="m-sub">OA Mod Launcher{version ? ` ${version}` : ''}</p>
          <div>
            <LauncherVersionRow version={version ?? 'unknown'} launcher={launcher} />
            <div className="m-row">
              <div>
                <b>What it talks to</b>
                <span>
                  Only GitHub, for the newest OA Mod, for BepInEx and for the launcher itself.
                  Nothing is uploaded, and the log stays on this PC.
                </span>
              </div>
            </div>
            <div className="m-row">
              <div>
                <b>Log and backups</b>
                <span>
                  The log has one line per thing the launcher did: the file to send when something
                  fails. The backups hold other plugins, copied before a full removal.
                </span>
              </div>
              <span className="side">
                <button type="button" className="sbtn" onClick={() => void openFolder('logs')}>
                  <Icon name="folder" />
                  Open log folder
                </button>
                <button type="button" className="sbtn" onClick={() => void openFolder('backups')}>
                  <Icon name="folder" />
                  Open backups
                </button>
              </span>
            </div>
            <div className="m-row">
              <div>
                <b>Fonts</b>
                <span>
                  Bagel Fat One, Atkinson Hyperlegible Next and Mono, under the SIL Open Font
                  License 1.1.
                </span>
              </div>
            </div>
          </div>
        </div>
      )}
    </Modal>
  );
}

/**
 * The launcher's own version and update. One button: it checks, and once a
 * newer launcher was found it installs that one. What it shows is what the
 * Rust side answered, the page asks nobody else.
 */
function LauncherVersionRow({
  version,
  launcher,
}: {
  version: string;
  launcher?: LauncherUpdateState;
}) {
  const checkLauncherUpdate = useLauncher(store => store.checkLauncherUpdate);
  const installLauncherUpdate = useLauncher(store => store.installLauncherUpdate);

  const newer = launcher?.update?.kind === 'available' ? launcher.update : null;
  const busy = launcher ? launcher.checking || launcher.installing : false;
  const done = percent(launcher?.progress?.received ?? 0, launcher?.progress?.total ?? 0);

  return (
    <div className="m-row">
      <div>
        <b>Launcher version</b>
        <span role="status" aria-live="polite">
          {launcher ? launcherStatusText(version, launcher) : version}
        </span>
        {launcher?.installing && (
          <span
            className="m-bar"
            role="progressbar"
            aria-label="Launcher update progress"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={done}
          >
            <i style={{ width: `${done}%` }} />
          </span>
        )}
        {newer?.notes && !launcher?.installing && <span className="rel sel">{newer.notes}</span>}
        {launcher?.error && (
          <span className="fail" role="alert">
            {sentence(launcher.error)}
          </span>
        )}
        <span className="path sel">github.com/Shironex/oa-mod-launcher</span>
      </div>
      <button
        type="button"
        className={newer ? 'sbtn tee' : 'sbtn'}
        disabled={busy}
        onClick={() => void (newer ? installLauncherUpdate() : checkLauncherUpdate())}
      >
        {newer && <Icon name="up" />}
        {newer ? `Update to ${newer.version}` : 'Check for updates'}
      </button>
    </div>
  );
}
