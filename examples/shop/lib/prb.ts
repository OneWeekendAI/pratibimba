import { createPratibimba } from '@pratibimba/next';

// One instance for the app. PRB_CORE_URL is Pratibimba Core, called server-side only.
// PRB_TIMEOUT_MS: how long a render waits for a decision before it shows the baseline
// (the SDK default is 300 ms; raise it when Core is far from this server).
// PRB_PERSIST=1: keep the subject in a one-year cookie, so a visitor who comes back is
// `returning`. A real store sets this only behind a consent banner; the cohort script
// (scripts/cohort.mjs) turns it on to have returning visitors at all.
export const prb = createPratibimba({
  core: process.env.PRB_CORE_URL ?? 'http://localhost:8080',
  site: 'shop',
  timeoutMs: process.env.PRB_TIMEOUT_MS ? Number(process.env.PRB_TIMEOUT_MS) : undefined,
  persist: process.env.PRB_PERSIST === '1',
});
