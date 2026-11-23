/// <reference types="vitest/config" />
import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  build: {
    rollupOptions: {
      // CodeMirror changes rarely; its own file stays cached across deploys of the app code.
      output: {
        manualChunks(id) {
          if (id.includes('node_modules/@codemirror') || id.includes('node_modules/codemirror') || id.includes('node_modules/@lezer')) return 'codemirror';
        },
      },
    },
  },
  test: { environment: 'node', include: ['src/**/*.test.ts'] },
  server: {
    port: 5173,
    proxy: { '/api': 'http://localhost:8080' },
  },
});
