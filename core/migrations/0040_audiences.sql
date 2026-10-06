-- Promoted audiences gate decide. A variant restricted to an audience is served only to
-- subjects whose decision context matches the audience's predicate; the baseline is never
-- restricted, so every visitor always has something to see. The gate fails closed: a variant
-- whose audience is no longer an audience (archived) is served to nobody until it is lifted.
ALTER TABLE variants ADD COLUMN audience_id BIGINT REFERENCES segments(id);
ALTER TABLE variants ADD CONSTRAINT variants_baseline_is_ungated CHECK (NOT (is_baseline AND audience_id IS NOT NULL));

-- The audiences a decision evaluated: [{id, name, matched, variants}] (backend.md, "decisions":
-- "audience match set"). NULL on decisions made before audiences could gate.
ALTER TABLE decisions ADD COLUMN audiences JSONB;
