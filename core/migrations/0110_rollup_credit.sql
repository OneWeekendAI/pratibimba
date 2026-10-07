-- B1b: rollups carry the credit their converted exposures took, so a linear slot's insights sum
-- shares instead of counting every credited decision as a whole conversion (the same rule
-- results.rs follows). 0 on pending and not-converted cells; equal to n under last touch.
ALTER TABLE rollup_hourly ADD COLUMN credit DOUBLE PRECISION NOT NULL DEFAULT 0;
ALTER TABLE rollup_daily ADD COLUMN credit DOUBLE PRECISION NOT NULL DEFAULT 0;
-- Existing converted cells were counted whole, which is exact under last touch.
UPDATE rollup_hourly SET credit = n WHERE outcome = 'converted';
UPDATE rollup_daily SET credit = n WHERE outcome = 'converted';
