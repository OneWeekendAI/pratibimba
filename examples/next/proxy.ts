import { prb } from './lib/prb';

export const proxy = prb.proxy;

export const config = {
  // Pages only: not Next's own assets, not the /_prb door itself.
  matcher: ['/((?!_next/|_prb/|favicon.ico).*)'],
};
