// Run with: pnpm test:scripts
//
// The signing tests start the real Tauri CLI. Without it (no `pnpm install`)
// they are skipped with a message, not failed.

import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import {
  DEV_KEY,
  PASSWORD_VARIABLE,
  findTauriCli,
  isBase64,
  isDevKey,
  isGithubUrl,
  manifestSigningKey,
  signFile,
} from './sign.mjs';

const skip = findTauriCli() ? false : 'the Tauri CLI is not installed: run `pnpm install`';

test('base64 is recognised', () => {
  assert.ok(isBase64('dW50cnVzdGVk\n'));
  assert.ok(isBase64('YQ=='));
  assert.ok(!isBase64(''));
  assert.ok(!isBase64('not base64!'));
  assert.ok(!isBase64('abc'));
});

test('only the development key inside the repository is the dev key', () => {
  const directory = mkdtempSync(join(tmpdir(), 'oa-sign-'));
  try {
    const copy = join(directory, 'dev.key');
    writeFileSync(copy, readFileSync(DEV_KEY));
    assert.ok(isDevKey(DEV_KEY));
    assert.ok(!isDevKey(copy));
    assert.ok(!isDevKey(join(directory, 'missing.key')));
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test('the dev key is the default for local files and never for a GitHub release', () => {
  const release = 'https://github.com/Shironex/oa-mod/releases/download/v1.0.0/';
  const local = 'http://127.0.0.1:8123/';
  assert.ok(isGithubUrl(release));
  assert.ok(!isGithubUrl(local));
  assert.ok(!isGithubUrl('not a url'));
  assert.equal(manifestSigningKey({ baseUrl: local }), DEV_KEY);
  assert.equal(manifestSigningKey({ baseUrl: local, signKey: 'k.key' }), 'k.key');
  assert.equal(manifestSigningKey({ baseUrl: local, noSign: true }), null);
  assert.throws(() => manifestSigningKey({ baseUrl: release }), /--sign-key/);
  assert.equal(manifestSigningKey({ baseUrl: release, signKey: 'k.key' }), 'k.key');
  assert.equal(manifestSigningKey({ baseUrl: release, noSign: true }), null);
});

test('a key outside the repository needs its password in the environment', () => {
  const directory = mkdtempSync(join(tmpdir(), 'oa-sign-'));
  const saved = process.env[PASSWORD_VARIABLE];
  try {
    const key = join(directory, 'other.key');
    const file = join(directory, 'a.txt');
    writeFileSync(key, readFileSync(DEV_KEY));
    writeFileSync(file, 'hello');
    delete process.env[PASSWORD_VARIABLE];
    assert.throws(() => signFile({ file, key }), new RegExp(PASSWORD_VARIABLE));
    assert.throws(() => signFile({ file, key, passwordEnv: 'OA_NO_SUCH_VARIABLE' }), /OA_NO_SUCH/);
    assert.throws(() => signFile({ file: join(directory, 'missing'), key }), /does not exist/);
  } finally {
    if (saved !== undefined) process.env[PASSWORD_VARIABLE] = saved;
    rmSync(directory, { recursive: true, force: true });
  }
});

test('signing with the dev key leaves a base64 signature next to the file', { skip }, () => {
  const directory = mkdtempSync(join(tmpdir(), 'oa-sign-'));
  try {
    const file = join(directory, 'manifest.json');
    writeFileSync(file, '{"a":1}\n');
    const signature = signFile({ file, key: DEV_KEY });
    assert.equal(signature, `${file}.sig`);
    assert.ok(isBase64(readFileSync(signature, 'utf8')));
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
