import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  plugins: [tailwindcss(), react()],
  // Tauri prints Rust errors into the same terminal: keep them on screen.
  clearScreen: false,
  server: {
    port: 15190,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**', '**/target/**', '**/crates/**'] },
  },
  build: {
    outDir: 'dist',
    target: 'es2023',
  },
  test: {
    include: ['src/**/*.test.{ts,tsx}'],
  },
});
