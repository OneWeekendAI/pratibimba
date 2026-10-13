// The Durable Object class bound as BEACON (one object per site, idFromName(site)). See
// beacon-core.js for what it does and why it exists.
import { DurableObject } from 'cloudflare:workers';
import { beacon } from './beacon-core.js';

export class SiteBeacon extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.core = beacon(ctx.storage);
  }

  version() {
    return this.core.version();
  }

  snapshot() {
    return this.core.snapshot();
  }

  put(snap) {
    return this.core.put(snap);
  }
}
