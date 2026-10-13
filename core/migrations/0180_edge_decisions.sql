-- EC2 (edge-cache.md): decisions the edge Worker made from its snapshot and sent to Core.
-- `source = 'edge'` marks them; `edge_snapshot_version` is the snapshot version the Worker
-- decided from (edge_snapshots.version at the time), so a lagging PoP shows in the data.
ALTER TABLE decisions DROP CONSTRAINT decisions_source_check;
ALTER TABLE decisions ADD CONSTRAINT decisions_source_check CHECK (source IN ('server', 'client', 'edge'));
ALTER TABLE decisions ADD COLUMN edge_snapshot_version BIGINT;
ALTER TABLE decisions ADD CONSTRAINT decisions_edge_version CHECK ((source = 'edge') = (edge_snapshot_version IS NOT NULL));
