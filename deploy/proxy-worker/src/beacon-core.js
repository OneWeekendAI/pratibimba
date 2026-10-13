// The version beacon's logic (EC2), apart from the Durable Object around it so it runs under
// node --test. One beacon per site holds the newest snapshot Core pushed. A Durable Object is
// strongly consistent: once Core's push returns, every reader anywhere sees that version, which
// Workers KV does not promise for up to a minute.

/** `storage` is a Durable Object's `ctx.storage` (get/put). */
export function beacon(storage) {
  let memo; // the snapshot, read once per object lifetime
  const load = async () => {
    if (memo === undefined) memo = (await storage.get('snapshot')) ?? null;
    return memo;
  };
  return {
    /** `{version}` of the newest snapshot, or `{version: null}` before the first push. */
    async version() {
      const s = await load();
      return { version: s ? s.version : null };
    },
    async snapshot() {
      return load();
    },
    /** Keep `snap` when it is newer than what is held; a stale or repeated push changes nothing. */
    async put(snap) {
      const s = await load();
      if (s && s.version >= snap.version) return { kept: false, version: s.version };
      await storage.put('snapshot', snap);
      memo = snap;
      return { kept: true, version: snap.version };
    },
  };
}
