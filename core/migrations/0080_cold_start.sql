-- Batch A4: cold start. A slot gives every approved arm an equal turn until it has had this
-- many exposures (holdout excluded), then LinTS decides. 0 switches it off.
ALTER TABLE slots ADD COLUMN cold_start_exposures INTEGER NOT NULL DEFAULT 100
    CHECK (cold_start_exposures BETWEEN 0 AND 100000);

-- Set on a decision cold start chose: {"exposures": k, "until": n} at the time it was made.
ALTER TABLE decisions ADD COLUMN cold_start JSONB;

-- The two counts a cold-start decide reads, both bounded by the slot.
CREATE INDEX decisions_slot_learned_exposed ON decisions (slot_id) WHERE exposed_at IS NOT NULL AND NOT in_holdout;
CREATE INDEX decisions_slot_cold_start ON decisions (slot_id, variant_id) WHERE cold_start IS NOT NULL;
