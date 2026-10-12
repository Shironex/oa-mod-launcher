// @ts-check

import eslint from '@eslint/js';
import { defineConfig } from 'eslint/config';
import tseslint from 'typescript-eslint';
import eslintConfigPrettier from 'eslint-config-prettier/flat';
import globals from 'globals';
import reactHooks from 'eslint-plugin-react-hooks';

export default defineConfig(
  eslint.configs.recommended,
  tseslint.configs.recommended,
  eslintConfigPrettier,
  {
    languageOptions: {
      ecmaVersion: 2023,
      sourceType: 'module',
      globals: { ...globals.browser, ...globals.node },
    },
    rules: {
      '@typescript-eslint/no-unused-vars': [
        'error',
        { argsIgnorePattern: '^_', varsIgnorePattern: '^_' },
      ],
      '@typescript-eslint/no-explicit-any': 'error',
      // The page talks to the Rust side only. A network call from here would
      // bypass the https rule and the checks that live there.
      'no-restricted-globals': [
        'error',
        { name: 'fetch', message: 'All network access is in the Rust core.' },
        { name: 'XMLHttpRequest', message: 'All network access is in the Rust core.' },
        { name: 'WebSocket', message: 'All network access is in the Rust core.' },
      ],
    },
  },
  {
    files: ['src/**/*.{ts,tsx}'],
    plugins: { 'react-hooks': reactHooks },
    rules: {
      'react-hooks/rules-of-hooks': 'error',
      'react-hooks/exhaustive-deps': 'error',
    },
  },
  {
    ignores: ['node_modules/**', 'dist/**', 'target/**', 'src-tauri/gen/**', 'src/bindings.ts'],
  }
);
