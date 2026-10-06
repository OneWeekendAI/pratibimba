import { createPratibimba } from '@pratibimba/next';

// One instance for the app. PRB_CORE_URL is where Pratibimba Core listens, server-side only.
export const prb = createPratibimba({
  core: process.env.PRB_CORE_URL ?? 'http://127.0.0.1:8080',
  site: 'shop',
});
