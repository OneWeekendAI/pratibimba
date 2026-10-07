import { createPratibimba } from '@pratibimba/next';

// One instance for the app. PRB_CORE_URL is Pratibimba Core, called server-side only.
// PRB_TIMEOUT_MS: how long a render waits for a decision before it shows the baseline
// (the SDK default is 300 ms; raise it when Core is far from this server).
export const prb = createPratibimba({
  core: process.env.PRB_CORE_URL ?? 'https://pratibimbaapi.meetdev.in',
  site: 'shop',
  timeoutMs: process.env.PRB_TIMEOUT_MS ? Number(process.env.PRB_TIMEOUT_MS) : undefined,
});
