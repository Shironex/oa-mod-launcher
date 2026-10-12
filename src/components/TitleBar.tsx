import { getCurrentWindow } from '@tauri-apps/api/window';
import { Icon } from './Icon';

/**
 * The top strip of the frameless window: a drag area across the full width,
 * the name at the top left and the three window buttons at the top right.
 *
 * The window is only asked for inside a click, so the page also renders in a
 * plain browser, where the previews are looked at.
 */
export function TitleBar() {
  return (
    <>
      <div className="drag" data-tauri-drag-region />
      <p className="cap">
        <i className="pen" />
        <b>OA MOD</b>
        <span>tournament launcher</span>
      </p>
      <div className="ctl">
        <button
          type="button"
          aria-label="Minimise"
          onClick={() => void getCurrentWindow().minimize()}
        >
          <Icon name="minimize" />
        </button>
        <button
          type="button"
          aria-label="Maximise or restore"
          onClick={() => void getCurrentWindow().toggleMaximize()}
        >
          <Icon name="maximize" />
        </button>
        <button
          type="button"
          className="x"
          aria-label="Close"
          onClick={() => void getCurrentWindow().close()}
        >
          <Icon name="close" />
        </button>
      </div>
    </>
  );
}
