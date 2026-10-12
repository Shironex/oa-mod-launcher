// The page as markup: rendered to a string, so no browser is needed.

import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it } from 'vitest';
import { version } from '../package.json';
import { App } from './App';

it('shows the launcher name and its version', () => {
  const html = renderToStaticMarkup(<App />);

  expect(html).toContain('OA Mod Launcher');
  expect(html).toContain(`Version ${version}`);
  expect(version).toMatch(/^\d+\.\d+\.\d+$/);
});
