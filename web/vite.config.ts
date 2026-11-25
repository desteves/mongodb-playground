/// <reference types="vitest/config" />
import { readFileSync } from 'node:fs';
import { defineConfig, type Plugin } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

/**
 * Put what the first paint needs into the HTML itself, so it does not wait for two more requests (PageSpeed's
 * "render-blocking requests"): the stylesheet becomes a <style> block and the tiny theme script is inlined (the server
 * adds the script's hash to the Content-Security-Policy). Production build only; `vite dev` keeps the separate files.
 */
function inlineFirstPaint(): Plugin {
  return {
    name: 'inline-first-paint',
    apply: 'build',
    enforce: 'post',
    generateBundle(_options, bundle) {
      const page = bundle['index.html'];
      if (!page || page.type !== 'asset' || typeof page.source !== 'string') return;
      let html = page.source;
      html = html.replace(/<link rel="stylesheet"[^>]*href="\/(assets\/[^"]+\.css)"[^>]*>/g, (tag, file: string) => {
        const css = bundle[file];
        if (!css || css.type !== 'asset' || typeof css.source !== 'string') return tag;
        delete bundle[file];
        return `<style>${css.source}</style>`;
      });
      const theme = readFileSync(new URL('./public/theme-init.js', import.meta.url), 'utf8').trim();
      html = html.replace('<script src="/theme-init.js"></script>', `<script>${theme}</script>`);
      page.source = html;
    },
  };
}

export default defineConfig({
  plugins: [svelte(), inlineFirstPaint()],
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
