import { defineConfig } from 'vite';

export default defineConfig({
  base: '/',
  server: {
    port: 5174,
    strictPort: true,
    proxy: {
      // Pratibimba Core (phase 2). Harmless while Core is not running.
      '/_prb': {
        target: 'http://localhost:8080',
        changeOrigin: true,
        rewrite: (p) => p.replace(/^\/_prb/, ''),
      },
    },
  },
});
