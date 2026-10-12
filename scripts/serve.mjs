// A static file server on loopback, for testing the launcher without GitHub.
//
//   node scripts/serve.mjs --root <folder> [--port 8123] [--throttle 200]
//
// It serves the files of `--root` on 127.0.0.1 only: a manifest with its
// signature, the mod DLL and the BepInEx zip next to it. `--throttle` limits
// zip and DLL downloads to that many kilobytes per second, so the progress
// display can be watched: a local download is otherwise over before the
// window repaints. See docs/development.md for the whole local setup.
//
// Not a general web server: GET only, no directory listings, nothing outside
// the root.

import { createReadStream, existsSync, statSync } from 'node:fs';
import { createServer } from 'node:http';
import { extname, join, normalize, resolve, sep } from 'node:path';
import { argument } from './lib/manifest.mjs';

const root = resolve(argument('root', '.'));
const port = Number(argument('port', '8123'));
const throttle = Number(argument('throttle', '0'));

const TYPES = {
  '.json': 'application/json',
  '.zip': 'application/zip',
  '.dll': 'application/octet-stream',
};
/** The downloads `--throttle` slows down: the two the launcher shows progress for. */
const THROTTLED = new Set(['.zip', '.dll']);

function send(response, status, text) {
  response.writeHead(status, { 'Content-Type': 'text/plain', 'Content-Length': text.length });
  response.end(text);
}

const server = createServer((request, response) => {
  const path = decodeURIComponent(new URL(request.url ?? '/', 'http://127.0.0.1').pathname);
  const file = normalize(join(root, path));
  const inside = file === root || file.startsWith(root + sep);

  if (request.method !== 'GET') return send(response, 405, 'GET only');
  if (!inside || !existsSync(file) || !statSync(file).isFile()) {
    console.log(`404 ${path}`);
    return send(response, 404, 'not found');
  }

  const size = statSync(file).size;
  response.writeHead(200, {
    'Content-Type': TYPES[extname(file)] ?? 'application/octet-stream',
    'Content-Length': size,
    'Cache-Control': 'no-store',
  });
  console.log(`200 ${path} (${size} bytes)`);

  if (!(throttle > 0) || !THROTTLED.has(extname(file))) {
    createReadStream(file).pipe(response);
    return;
  }

  // Ten slices a second, each a tenth of the allowed bytes.
  const slice = Math.max(1, Math.floor((throttle * 1024) / 10));
  const stream = createReadStream(file, { highWaterMark: slice });
  stream.on('data', chunk => {
    response.write(chunk);
    stream.pause();
    setTimeout(() => stream.resume(), 100);
  });
  stream.on('end', () => response.end());
  stream.on('error', () => response.destroy());
  response.on('close', () => stream.destroy());
});

server.listen(port, '127.0.0.1', () => {
  console.log(`serving ${root} at http://127.0.0.1:${port}/`);
  if (throttle > 0) console.log(`zip and dll downloads limited to ${throttle} kB/s`);
});
