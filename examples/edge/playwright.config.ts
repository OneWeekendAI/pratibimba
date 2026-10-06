import { defineConfig } from '@playwright/test';

export const CORE_PORT = 18082;
export const CORE = `http://127.0.0.1:${CORE_PORT}`;

// Real Core and worker (see e2e/core.mjs), the edge worker on Node, a static origin.
export default defineConfig({
  testDir: './e2e',
  workers: 1,
  timeout: 30_000,
  use: { baseURL: 'http://localhost:3200' },
  webServer: [
    { command: 'node e2e/core.mjs', url: `${CORE}/health`, reuseExistingServer: false },
    { command: 'node edge.mjs', url: 'http://localhost:3200/_prb/manifest/edge-demo', env: { PRB_CORE_URL: CORE }, reuseExistingServer: false },
  ],
});
