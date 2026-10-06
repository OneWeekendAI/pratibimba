import { defineConfig } from '@playwright/test';
import { CORE, NEXT_PORT } from './e2e/env';

// Real Core, real worker, real `next start`, on a fresh database. Build first:
//   cargo build && (cd examples/next && npm run build)
export default defineConfig({
  testDir: './e2e',
  workers: 1,
  timeout: 30_000,
  use: { baseURL: `http://localhost:${NEXT_PORT}` },
  webServer: [
    {
      command: 'node e2e/prepare.mjs && node e2e/core.mjs',
      url: `${CORE}/health`,
      reuseExistingServer: false,
    },
    {
      command: `npx next start -p ${NEXT_PORT}`,
      url: `http://localhost:${NEXT_PORT}/_prb/manifest/shop`,
      env: { PRB_CORE_URL: CORE },
      reuseExistingServer: false,
    },
  ],
});
