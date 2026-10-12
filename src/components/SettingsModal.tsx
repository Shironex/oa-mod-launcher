import type { Snapshot } from '../bindings';
import { sentence } from '../lib/format';
import { useLauncher } from '../state/store';
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
  /** Why nothing in the game folder may be changed right now, or null. */
  locked: string | null;
  /** Whether the game folder holds anything the launcher could remove. */
  canRemove: boolean;
  /** The sentence of a command that failed, such as a folder that was refused. */
  error: string | null;
}

/** Settings: where the game lives, the update check, the way to the removal, and About. */
export function SettingsModal({ section, snapshot, locked, canRemove, error }: SettingsModalProps) {
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
                <button
                  type="button"
                  className="sbtn"
                  disabled={locked !== null}
                  title="Forget a folder chosen by hand and use the one Steam names"
                  onClick={() => void resetGameFolder()}
                >
                  Reset to Steam
                </button>
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
            <div className="m-row">
              <div>
                <b>Launcher version</b>
                <span>{version ?? 'unknown'}</span>
                <span className="path sel">github.com/Shironex/oa-mod-launcher</span>
              </div>
            </div>
            <div className="m-row">
              <div>
                <b>What it talks to</b>
                <span>
                  Only GitHub, for the newest OA Mod and for BepInEx. Nothing is uploaded, and the
                  log stays on this PC.
                </span>
              </div>
            </div>
            <div className="m-row">
              <div>
                <b>Launcher log</b>
                <span>
                  One line per thing the launcher did. The file to send when something fails.
                </span>
              </div>
              <button type="button" className="sbtn" onClick={() => void openFolder('logs')}>
                <Icon name="folder" />
                Open folder
              </button>
            </div>
            <div className="m-row">
              <div>
                <b>Backups</b>
                <span>Other plugins and their settings, copied here before a full removal.</span>
              </div>
              <button type="button" className="sbtn" onClick={() => void openFolder('backups')}>
                <Icon name="folder" />
                Open folder
              </button>
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
