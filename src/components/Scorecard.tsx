import type { Folder, Snapshot } from '../bindings';
import type { Dialog } from '../state/store';
import type { Action, RightView, SlipView, View } from '../state/view';
import { Icon } from './Icon';
import { MainButton } from './MainButton';

interface ScorecardProps {
  view: View;
  snapshot: Snapshot | null;
  onAction: (action: Action) => void;
  onOpenDialog: (dialog: Dialog) => void;
  onOpenFolder: (folder: Folder) => void;
}

/** What the launcher does, for somebody who sees it for the first time. */
const GUIDE = [
  'Install puts BepInEx and OA Mod into the game folder. Nothing else on this PC is changed.',
  'Play starts the game through Steam, with the mod in it.',
  'Everyone in a match needs the same OA Mod. Compare the number on the button.',
  'Remove mod puts the game back to how Steam installed it.',
];

/**
 * The card along the bottom of the window. The left half says where things
 * stand and holds the one main action; the right half holds the line the
 * newest release came with, and the help.
 */
export function Scorecard({
  view,
  snapshot,
  onAction,
  onOpenDialog,
  onOpenFolder,
}: ScorecardProps) {
  return (
    <>
      {view.slip && <Slip slip={view.slip} onAction={onAction} />}
      <div className="card">
        <ScorecardPages
          view={view}
          snapshot={snapshot}
          onAction={onAction}
          onOpenDialog={onOpenDialog}
          onOpenFolder={onOpenFolder}
        />
      </div>
    </>
  );
}

/**
 * The yellow tag tied to the card: a newer launcher was found. It never takes
 * the focus. Its text is a status, and a failed update an alert. "Later" puts
 * it away until the launcher is started again; during the download it cannot
 * be put away, so a failure is always seen.
 */
function Slip({ slip, onAction }: { slip: SlipView; onAction: (action: Action) => void }) {
  return (
    <div className={slip.failed ? 'slip warn' : 'slip'} role="region" aria-label="Launcher update">
      <Icon name={slip.failed ? 'warn' : 'up'} />
      {/* The key makes a failure a new element, so it is announced as an alert. */}
      <span key={String(slip.failed)} role={slip.failed ? 'alert' : 'status'}>
        {slip.text}
      </span>
      {slip.install && (
        <button type="button" className="lnk" onClick={() => onAction({ type: 'update_launcher' })}>
          {slip.install}
        </button>
      )}
      {slip.canDismiss && (
        <button
          type="button"
          className="lnk"
          onClick={() => onAction({ type: 'dismiss_launcher_update', version: slip.version })}
        >
          Later
        </button>
      )}
    </div>
  );
}

function ScorecardPages({ view, snapshot, onAction, onOpenDialog, onOpenFolder }: ScorecardProps) {
  return (
    <>
      <section className="pg l" aria-label="Your scorecard">
        <div className="ph">
          <span>Your scorecard</span>
          <span>{view.fact}</span>
        </div>
        <p className="st" role="status" aria-live="polite">
          {view.sentence}
        </p>
        {view.detail && <p className="detail sel">{view.detail}</p>}
        <MainButton cta={view.cta} onAction={onAction} />
        <div className="ex">
          {view.links.map(link => (
            <button
              type="button"
              className="lnk"
              key={link.label}
              onClick={() => onAction(link.action)}
            >
              {link.label}
            </button>
          ))}
          {view.pill && (
            <span className={`pill ${view.pill.tone}`}>
              {view.pill.tone === 'ok' ? (
                <i />
              ) : (
                <Icon name={view.pill.tone === 'warn' ? 'warn' : 'clock'} />
              )}
              {view.pill.text}
            </span>
          )}
        </div>
        {view.holes && (
          <ol
            className="holes"
            style={{ gridTemplateColumns: `repeat(${view.holes.length}, 1fr)` }}
          >
            {view.holes.map(hole => (
              <li key={hole.title} className={hole.state === 'next' ? undefined : hole.state}>
                <b>{hole.title}</b>
                <span>{hole.mark}</span>
              </li>
            ))}
          </ol>
        )}
        <div className="notes">
          {view.notes.map(note => (
            <p className={note.calm ? 'note calm' : 'note'} key={note.label}>
              <b>{note.label}</b>
              {note.text}
            </p>
          ))}
        </div>
        <div className="pf">
          <span>
            <button type="button" className="lnk" onClick={() => onOpenDialog('settings')}>
              Settings
            </button>
            {snapshot?.game && (
              <button type="button" className="lnk" onClick={() => onOpenFolder('game')}>
                Game folder
              </button>
            )}
            {view.canRemove && (
              <button
                type="button"
                className="lnk red"
                disabled={view.locked !== null}
                title={view.locked ?? undefined}
                onClick={() => onOpenDialog('remove')}
              >
                Remove mod
              </button>
            )}
          </span>
          {snapshot && <span className="mono">launcher {snapshot.launcher_version} · Windows</span>}
        </div>
      </section>
      <section className="pg r" aria-label="About OA Mod">
        <RightPage right={view.right} />
      </section>
    </>
  );
}

function RightPage({ right }: { right: RightView }) {
  if (right.kind === 'find_game') {
    return (
      <>
        <div className="ph">
          <span>Where the game lives</span>
          <span>3 clicks</span>
        </div>
        <ol className="help">
          <li>Open Steam and go to your Library.</li>
          <li>
            Right click <b>Golf With Your Friends</b>, then Manage, then Browse local files.
          </li>
          <li>Choose that folder here.</li>
        </ol>
        <p className="note calm">
          The right folder has <span className="mono">Golf With Your Friends.exe</span> in it. The
          launcher remembers it.
        </p>
      </>
    );
  }
  const guide = (
    <ol className={right.kind === 'release' ? 'help tight' : 'help'}>
      {GUIDE.map(line => (
        <li key={line}>{line}</li>
      ))}
    </ol>
  );
  if (right.kind === 'guide') {
    return (
      <>
        <div className="ph">
          <span>How it works</span>
          <span>4 things</span>
        </div>
        {guide}
      </>
    );
  }
  return (
    <>
      <div className="ph">
        <span>What changed in OA Mod</span>
        <span>newest release</span>
      </div>
      <div className="news">
        <h3>
          {right.title}
          {right.tag && (
            <span className={right.tag === 'new' ? 'tag new' : 'tag'}>{right.tag}</span>
          )}
        </h3>
        <p className="sel">{right.notes}</p>
      </div>
      <div className="ph">
        <span>How it works</span>
      </div>
      {guide}
    </>
  );
}
