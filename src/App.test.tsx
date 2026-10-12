// The page as markup: rendered to a string, so no browser is needed.

import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { version } from '../package.json';
import { App, Window } from './App';
import { PREVIEW_NAMES, previewNamed } from './dev/preview';

function render(name: string): string {
  const preview = previewNamed(name);
  if (!preview) throw new Error(`no preview named ${name}`);
  const { dialog = null, plan: _plan, outcome: _outcome, ...state } = preview;
  return renderToStaticMarkup(<Window state={{ ...state, dialog }} />);
}

describe('the window', () => {
  it('before the first snapshot: the name, and nothing to press', () => {
    const html = renderToStaticMarkup(<App />);
    expect(html).toContain('OA MOD');
    expect(html).toContain('Looking for the game.');
    expect(version).toMatch(/^\d+\.\d+\.\d+$/);
  });

  it.each(PREVIEW_NAMES)('preview %s renders with one sentence and one main action', name => {
    const html = render(name);
    expect(html.match(/class="st"/g)).toHaveLength(1);
    expect(html.match(/class="act /g)).toHaveLength(1);
  });

  it('shows what the snapshot provides and no invented release list', () => {
    const html = render('up-to-date');
    expect(html).toContain('OA Mod 0.2.0 is installed and up to date.');
    expect(html).toContain('Brackets for up to 8 players');
    expect(html).toContain('launcher 0.1.0 · Windows');
    expect(html).not.toContain('releases');
  });

  it('while the game runs, the main action and Remove mod are disabled', () => {
    const html = render('running');
    expect(html).toMatch(/<button type="button" class="act off" disabled="">/);
    expect(html).toMatch(/class="lnk red" disabled="" title="Close the game first\."/);
  });

  it('without a game folder the footer offers neither the folder nor the removal', () => {
    const html = render('game-not-found');
    expect(html).not.toContain('Game folder</button>');
    expect(html).not.toContain('Remove mod');
    expect(html).toContain('Where the game lives');
  });
});
