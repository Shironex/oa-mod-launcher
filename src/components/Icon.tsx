// The launcher's icons: line drawings on a 24 by 24 grid, drawn with the
// current text colour. A handful of paths is smaller than an icon package.

const PATHS = {
  check: 'M5 12.5l4.5 4.5L19 7.5',
  clock: 'M12 3.5a8.5 8.5 0 100 17 8.5 8.5 0 000-17zM12 7.5V12l3 2',
  warn: 'M12 4l9 15.5H3zM12 10v4.5M12 17v.2',
  close: 'M6 6l12 12M18 6L6 18',
  minimize: 'M6 12h12',
  maximize: 'M6.5 6.5h11v11h-11z',
  folder: 'M3.5 7.5h6l2 2h9v9h-17z',
  info: 'M12 3.5a8.5 8.5 0 100 17 8.5 8.5 0 000-17zM12 11v5M12 8v.2',
  gear: 'M12 9a3 3 0 100 6 3 3 0 000-6zM12 3.5v3M12 17.5v3M3.5 12h3M17.5 12h3M6 6l2 2M16 16l2 2M18 6l-2 2M8 16l-2 2',
} as const;

export type IconName = keyof typeof PATHS;

/** A decorative icon. The text next to it carries the meaning. */
export function Icon({ name }: { name: IconName }) {
  return (
    <svg className="ico" viewBox="0 0 24 24" aria-hidden="true" focusable="false">
      <path d={PATHS[name]} />
    </svg>
  );
}
