-- EC4 (edge-cache.md, Decisions 1): each erasure's tombstone hash is pushed to the edge store
-- (KV key gone/<site>/<subject_hash>) so the edge Worker refuses the subject. `edge_pushed_at`
-- is when that push succeeded; NULL means not yet (no edge store, or the push failed), and every
-- worker pass with an edge store pushes the NULL rows, so a failed push or an erasure made
-- before the edge was configured still reaches it.
ALTER TABLE erasures ADD COLUMN edge_pushed_at TIMESTAMPTZ;
CREATE INDEX erasures_edge_pending ON erasures (id) WHERE edge_pushed_at IS NULL;
