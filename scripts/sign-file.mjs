// Signs one file with the Tauri CLI and leaves `<file>.sig` next to it.
//
//   node scripts/sign-file.mjs --file <path> --key <path to .key file> \
//     [--password-env <NAME>]
//
// The password is never an argument (it would land in the shell history). It is
// read from the environment variable `--password-env`, by default
// TAURI_SIGNING_PRIVATE_KEY_PASSWORD, and reaches the CLI only through the
// environment of the child process. An empty password is accepted only for the
// development key in dev-keys/.

import { argument } from './lib/manifest.mjs';
import { PASSWORD_VARIABLE, signFile } from './lib/sign.mjs';

const file = argument('file');
const key = argument('key');
if (!file || !key) {
  console.error(
    'usage: sign-file.mjs --file <path> --key <path to .key file> [--password-env <NAME>]'
  );
  process.exit(2);
}

try {
  const signature = signFile({
    file,
    key,
    passwordEnv: argument('password-env', PASSWORD_VARIABLE),
  });
  console.log(`${file}\n${signature}`);
} catch (error) {
  console.error(error.message);
  process.exit(1);
}
