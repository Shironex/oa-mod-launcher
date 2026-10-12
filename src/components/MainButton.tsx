import type { Action, CtaView } from '../state/view';

interface MainButtonProps {
  cta: CtaView;
  onAction: (action: Action) => void;
}

/**
 * The one main action of the scorecard: Choose, Install, Play, Update, Retry,
 * or the progress of a running install. A flag, the word, a putt line to the
 * cup, and what it acts on.
 *
 * The line is drawn twice. The outline is always there and carries the text
 * for a screen reader. The yellow layer lies over it and is hidden from one:
 * complete when the button can be pressed, and cut off at the percentage while
 * it works, so the button fills from the left. The percentage is text; the
 * fill and the progress value repeat it.
 */
export function MainButton({ cta, onAction }: MainButtonProps) {
  const action = cta.action;
  const busy = cta.style === 'busy';
  const done = cta.percent ?? 0;
  const line = (
    <>
      <i className="pen" />
      <b>{cta.word}</b>
      <span className="line" />
      <i className="hole" />
      <span className="v">{cta.sub}</span>
    </>
  );

  return (
    <button
      type="button"
      className={`act ${cta.style}`}
      disabled={!action}
      onClick={action ? () => onAction(action) : undefined}
    >
      <span className="a-in base">{line}</span>
      {cta.style !== 'off' && (
        <span
          className="a-in fill"
          style={
            busy ? { clipPath: `inset(0 ${100 - done}% 0 0 round calc(14 * var(--u)))` } : undefined
          }
          aria-hidden="true"
        >
          {line}
        </span>
      )}
      {busy && (
        <span
          role="progressbar"
          aria-label="Install progress"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={done}
        />
      )}
    </button>
  );
}
