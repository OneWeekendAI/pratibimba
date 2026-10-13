// The deployed entry point: the proxy (index.js) and the beacon Durable Object (beacon.js).
// index.js stays free of `cloudflare:workers` so node --test can load it.
export { default } from './index.js';
export { SiteBeacon } from './beacon.js';
