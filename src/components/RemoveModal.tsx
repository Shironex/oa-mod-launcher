import { useEffect, useState } from 'react';
import type { RemovalOutcome, RemovalPlan } from '../bindings';
import { sentence } from '../lib/format';
import {
  defaultMode,
  foreignNames,
  foreignRows,
  hasAnything,
  hasMod,
  keptReason,
  leftUnrecognised,
  needsSecondConfirmation,
  nothingSelected,
  removalRows,
  shown,
  unrecognisedHelp,
  type RemovalMode,
} from '../state/removal';
import { message, useLauncher } from '../state/store';
import { Icon } from './Icon';
import { Modal } from './Modal';

/** Steam's own repair is no substitute, and people reach for it first. */
const STEAM_VERIFY =
  'Steam can verify the game files, which restores the files of the game but does not delete extra ones. It does not replace this step.';

type Step = 'choose' | 'confirm' | 'working' | 'done';

/**
 * The remove dialog. It asks the Rust side what a removal would do, shows
 * that list, and deletes nothing before a button that says what it deletes
 * was pressed. A full removal that takes somebody else's files along asks a
 * second time.
 */
export function RemoveModal() {
  const openDialog = useLauncher(store => store.openDialog);
  const removalPlan = useLauncher(store => store.removalPlan);
  const removeMod = useLauncher(store => store.removeMod);
  const removeAll = useLauncher(store => store.removeAll);
  const openFolder = useLauncher(store => store.openFolder);

  const [plan, setPlan] = useState<RemovalPlan | null>(null);
  /** Why the plan could not be made, or why the removal failed. */
  const [problem, setProblem] = useState<string | null>(null);
  const [mode, setMode] = useState<RemovalMode>('all');
  const [include, setInclude] = useState(false);
  const [step, setStep] = useState<Step>('choose');
  const [outcome, setOutcome] = useState<RemovalOutcome | null>(null);

  useEffect(() => {
    let open = true;
    removalPlan().then(
      found => {
        if (!open) return;
        setPlan(found);
        setMode(defaultMode(found));
      },
      error => open && setProblem(message(error))
    );
    return () => {
      open = false;
    };
  }, [removalPlan]);

  // A running removal cannot be closed away: its answer has to be seen.
  const close = () => {
    if (step !== 'working') openDialog(null);
  };

  const remove = async () => {
    setStep('working');
    setProblem(null);
    try {
      if (mode === 'mod') await removeMod();
      else setOutcome(await removeAll(include));
      setStep('done');
    } catch (error) {
      setProblem(message(error));
      setStep('choose');
      // The game folder may look different now: Steam may have begun an
      // update, or a part of the removal may be done. The list follows it.
      removalPlan().then(setPlan, () => undefined);
    }
  };

  const onConfirm = () => {
    if (!plan) return;
    if (mode === 'all' && step === 'choose' && needsSecondConfirmation(plan)) setStep('confirm');
    else void remove();
  };

  if (!plan) {
    return (
      <Modal label="Remove OA Mod" onClose={close} step={plan ? step : 'loading'}>
        <h2 className="m-h">Put the game back to clean?</h2>
        {problem ? (
          <>
            <div className="warnbox" role="alert">
              <Icon name="warn" />
              <span>{sentence(problem)} Nothing was changed.</span>
            </div>
            <div className="btns">
              <button type="button" className="sbtn" data-autofocus onClick={close}>
                Close
              </button>
            </div>
          </>
        ) : (
          <p className="m-sub">Reading the game folder.</p>
        )}
      </Modal>
    );
  }

  if (step === 'working') {
    return (
      <Modal label="Remove OA Mod" onClose={close} step={plan ? step : 'loading'}>
        <h2 className="m-h">Removing</h2>
        <p className="m-sub" role="status">
          {mode === 'all' && plan.foreign.length > 0
            ? 'Copying the other plugins to the backups, then deleting. Keep this window open.'
            : 'Deleting the files. Keep this window open.'}
        </p>
      </Modal>
    );
  }

  if (step === 'done') {
    const left = outcome ? leftUnrecognised(plan, outcome) : [];
    return (
      <Modal label="Remove OA Mod" onClose={close} step={plan ? step : 'loading'}>
        <h2 className="m-h">{mode === 'mod' ? 'OA Mod is removed' : 'Removed'}</h2>
        <p className="m-sub" role="status">
          {mode === 'mod'
            ? 'BepInEx and every other plugin stayed, with their settings.'
            : left.length > 0
              ? 'BepInEx and OA Mod are gone, but the game is not clean yet.'
              : 'BepInEx and OA Mod are gone from the game folder. Your saves were not touched.'}
        </p>
        {outcome?.backup_dir && (
          <div className="warnbox calm">
            <Icon name="check" />
            <span>
              The other plugins and their settings were copied first. The copy is in{' '}
              <span className="mono sel">{outcome.backup_dir}</span>
            </span>
          </div>
        )}
        {outcome && outcome.kept.length > 0 && (
          <>
            <p className="dl-h">Kept in the game folder</p>
            <ul className="dl sel">
              {outcome.kept.map(name => (
                <li className="keep" key={name}>
                  <b>{shown(name)}</b>
                  <span>{keptReason(plan, name)}</span>
                </li>
              ))}
            </ul>
          </>
        )}
        {left.length > 0 && (
          <div className="warnbox">
            <Icon name="warn" />
            <span>
              <b>{left.join(', ')} stayed.</b> While {left.length > 1 ? 'they are' : 'it is'} there
              without BepInEx, the launcher reports an unsupported BepInEx and Install stops with
              that reason. To clear that, open Remove mod again and tick the box that includes{' '}
              {left.length > 1 ? 'them' : 'it'}.
            </span>
          </div>
        )}
        {mode === 'all' && <p className="small">{STEAM_VERIFY}</p>}
        <div className="btns">
          {outcome?.backup_dir && (
            <button type="button" className="sbtn" onClick={() => void openFolder('backups')}>
              <Icon name="folder" />
              Open backups
            </button>
          )}
          <button type="button" className="sbtn" data-autofocus onClick={close}>
            Close
          </button>
        </div>
      </Modal>
    );
  }

  if (step === 'confirm') {
    const names = foreignNames(plan);
    return (
      <Modal label="Remove everything" onClose={close} step={step}>
        <h2 className="m-h">Delete these too?</h2>
        <p className="m-sub">
          {names.length > 0
            ? 'Removing everything takes other plugins along. They are not part of OA Mod.'
            : 'This BepInEx was not installed by this launcher. Removing everything deletes it too.'}
        </p>
        {plan.foreign.length > 0 && (
          <>
            <p className="dl-h">Other people&apos;s files in this folder</p>
            <ul className="dl sel">
              {foreignRows(plan).map(row => (
                <li key={row.name}>
                  <b>{row.name}</b>
                  <span>{row.what}</span>
                </li>
              ))}
            </ul>
            <div className="warnbox">
              <Icon name="warn" />
              <span>
                <b>
                  {names.length > 0
                    ? `${names.join(', ')} ${names.length > 1 ? 'go' : 'goes'} too.`
                    : 'Their settings go too.'}
                </b>{' '}
                A copy is made in the launcher&apos;s backups folder before anything is deleted, but
                the game will start without them.
              </span>
            </div>
          </>
        )}
        <div className="btns">
          <button type="button" className="sbtn" data-autofocus onClick={() => setStep('choose')}>
            Back
          </button>
          <button type="button" className="sbtn red" onClick={() => void remove()}>
            Yes, delete everything
          </button>
        </div>
      </Modal>
    );
  }

  const anything = hasAnything(plan);
  const names = foreignNames(plan);
  const option = (value: RemovalMode, title: string, sub: string, disabled = false) => (
    <button
      type="button"
      className="opt"
      role="radio"
      aria-checked={mode === value}
      disabled={disabled}
      onClick={() => setMode(value)}
    >
      <i />
      <b>{title}</b>
      <span>{sub}</span>
    </button>
  );

  return (
    <Modal label="Remove OA Mod" onClose={close} step={plan ? step : 'loading'}>
      <h2 className="m-h">Put the game back to clean?</h2>
      <p className="m-sub">
        {anything
          ? 'This deletes files from the game folder. It cannot be undone, but Install brings the mod back.'
          : 'There is nothing of BepInEx or OA Mod in the game folder.'}
      </p>
      {anything && (
        <>
          <div className="opts" role="radiogroup" aria-label="What to remove">
            {option(
              'all',
              'Remove everything',
              names.length > 0
                ? 'BepInEx, OA Mod and every other plugin'
                : 'BepInEx and OA Mod: a clean game'
            )}
            {option(
              'mod',
              'Remove only OA Mod',
              hasMod(plan) ? 'Keep BepInEx and other plugins' : 'OA Mod is not installed',
              !hasMod(plan)
            )}
          </div>
          <p className="dl-h">
            {mode === 'all'
              ? 'Will be deleted from the game folder'
              : 'What happens in the game folder'}
          </p>
          <ul className="dl sel">
            {removalRows(plan, mode, include).map(row => (
              <li className={row.keep ? 'keep' : undefined} key={row.name}>
                <b>{row.name}</b>
                <span>{row.what}</span>
              </li>
            ))}
          </ul>
          {mode === 'all' && plan.unrecognised_root_files.length > 0 && (
            <button
              type="button"
              className="check"
              role="checkbox"
              aria-checked={include}
              onClick={() => setInclude(!include)}
            >
              <i>
                <Icon name="check" />
              </i>
              <span>
                Also delete {plan.unrecognised_root_files.join(', ')}
                <span>{unrecognisedHelp(plan)}</span>
              </span>
            </button>
          )}
          {mode === 'all' && names.length > 0 && (
            <div className="warnbox">
              <Icon name="warn" />
              <span>
                <b>
                  {names.join(', ')} {names.length > 1 ? 'go' : 'goes'} too.
                </b>{' '}
                Other plugins in this folder are copied to the launcher&apos;s backups, then deleted
                with their settings.
              </span>
            </div>
          )}
          {mode === 'mod' && (
            <div className="warnbox calm">
              <Icon name="check" />
              <span>The game still starts with BepInEx. It is not a clean game yet.</span>
            </div>
          )}
          {mode === 'all' && <p className="small">{STEAM_VERIFY}</p>}
        </>
      )}
      {problem && (
        <div className="warnbox" role="alert">
          <Icon name="warn" />
          <span>{sentence(problem)}</span>
        </div>
      )}
      <div className="btns">
        <button type="button" className="sbtn" data-autofocus onClick={close}>
          {anything ? 'Cancel' : 'Close'}
        </button>
        {anything && (
          <button
            type="button"
            className="sbtn red"
            disabled={mode === 'all' && nothingSelected(plan, include)}
            title={
              mode === 'all' && nothingSelected(plan, include)
                ? 'Tick the box above: nothing else is left to delete'
                : undefined
            }
            onClick={onConfirm}
          >
            {mode === 'mod'
              ? 'Delete only OA Mod'
              : plan.folders.length > 0
                ? 'Delete BepInEx and OA Mod'
                : 'Delete these files'}
          </button>
        )}
      </div>
    </Modal>
  );
}
