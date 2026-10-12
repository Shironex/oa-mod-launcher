import { version } from '../package.json';

/** A placeholder: the name and the version, until the real window is built. */
export function App() {
  return (
    <main className="flex h-full flex-col items-center justify-center gap-3">
      <h1 className="font-display text-5xl tracking-wide">OA Mod Launcher</h1>
      <p className="font-mono text-sm text-dim">Version {version}</p>
    </main>
  );
}
