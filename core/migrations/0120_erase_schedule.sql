-- B2: right-to-erase tombstones at once and schedules the events' removal (backend.md,
-- "Right-to-erase"). The erasure row stays forever as the tombstone; `purge_after` is when the
-- worker deletes the events, `purged_at` when it did.
ALTER TABLE erasures
    ADD COLUMN purge_after TIMESTAMPTZ,
    ADD COLUMN purged_at   TIMESTAMPTZ;
-- Erasures made before this migration deleted their events on the spot.
UPDATE erasures SET purge_after = created_at, purged_at = created_at;
ALTER TABLE erasures ALTER COLUMN purge_after SET NOT NULL;
CREATE INDEX erasures_due ON erasures (purge_after) WHERE purged_at IS NULL;

-- Every stored form of an erased subject whose events still wait for removal. This is the only
-- place a raw subject sits beside its erasure, and only until the purge deletes both the events
-- and these rows.
CREATE TABLE erased_subjects (
    erasure_id  BIGINT NOT NULL REFERENCES erasures(id) ON DELETE CASCADE,
    site_id     TEXT NOT NULL,
    subject     TEXT NOT NULL,
    PRIMARY KEY (erasure_id, subject)
);
CREATE INDEX erased_subjects_lookup ON erased_subjects (site_id, subject);

-- What every read path sees: the events of subjects not waiting for removal. Only ingest, the
-- erase and the purge touch `events` itself. A later migration that adds a column to `events`
-- must recreate this view, since `e.*` is expanded when the view is made.
CREATE VIEW live_events AS
    SELECT e.* FROM events e
     WHERE NOT EXISTS (SELECT 1 FROM erased_subjects x WHERE x.site_id = e.site_id AND x.subject = e.subject);
