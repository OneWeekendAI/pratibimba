import { defineConfig } from '@playwright/test';

// The dashboard against real Core (build it first: cargo build; npm run build).
export default defineConfig({
  testDir: './e2e',
  workers: 1,
  timeout: 45_000,
  use: { baseURL: 'http://127.0.0.1:18083' },
  webServer: { command: 'node e2e/stack.mjs', url: 'http://127.0.0.1:3400/__ready', timeout: 240_000, reuseExistingServer: false },
});
