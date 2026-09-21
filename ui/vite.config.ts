import { fileURLToPath, URL } from 'node:url';

import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    // `@/components/...` is how every component imports its neighbours; the same
    // mapping lives in tsconfig.json > compilerOptions.paths, because Vite does
    // not read tsconfig and tsc does not read this file.
    alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
  },
  server: {
    // tauri.conf.json > build.devUrl points at this exact port. strictPort so a
    // leftover server on 1420 fails loudly instead of quietly moving to 1421,
    // which would leave the app showing a dead page.
    port: 1420,
    strictPort: true,
  },
  build: {
    // Two windows ship from this frontend: the settings window and the dictation
    // overlay. Vite derives each output path from the HTML file's own path, so
    // this emits dist/index.html and dist/hud.html — the two paths the Rust side
    // asks for via WebviewUrl::App.
    rollupOptions: { input: { main: 'index.html', hud: 'hud.html' } },
  },
});
