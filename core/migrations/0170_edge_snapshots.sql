-- EC1 (edge-cache.md): the last per-site snapshot Core published to the edge store
-- (Cloudflare KV). `version` rises by one each time the snapshot's content changes;
-- `content_hash` is what decides "changed", so a trigger that changed nothing writes nothing.
-- Empty on a self-hosted Core with no edge store configured: nothing is ever built there.
CREATE TABLE edge_snapshots (
    site_id       TEXT PRIMARY KEY REFERENCES sites(id) ON DELETE CASCADE,
    version       BIGINT NOT NULL CHECK (version > 0),
    content_hash  TEXT NOT NULL,
    bytes         INTEGER NOT NULL,
    published_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
