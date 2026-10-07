-- D1: experiments, a fixed-allocation A/B overlay on one slot (see core/src/experiments.rs).
-- `arms` is [{variant_id, weight}] in variant-id order. An experiment runs from starts_at until
-- the earlier of stops_at and stopped_at (a person's early stop).
CREATE TABLE experiments (
    id          BIGSERIAL PRIMARY KEY,
    site_id     TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    slot_id     BIGINT NOT NULL REFERENCES slots(id) ON DELETE CASCADE,
    name        TEXT NOT NULL CHECK (length(btrim(name)) > 0),
    metric      TEXT NOT NULL,
    arms        JSONB NOT NULL,
    starts_at   TIMESTAMPTZ NOT NULL,
    stops_at    TIMESTAMPTZ NOT NULL,
    stopped_at  TIMESTAMPTZ,
    stopped_by  TEXT,
    created_by  TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT experiments_window CHECK (stops_at > starts_at),
    CONSTRAINT experiments_stopped CHECK ((stopped_at IS NULL) = (stopped_by IS NULL))
);
CREATE INDEX experiments_slot ON experiments (slot_id, starts_at);

-- Which experiment was running when the decision was made, and who chose the treated arm:
-- the bandit (also when every experiment arm was closed to the visitor) or the experiment's hash.
ALTER TABLE decisions
    ADD COLUMN experiment_id BIGINT REFERENCES experiments(id) ON DELETE SET NULL,
    ADD COLUMN allocation TEXT NOT NULL DEFAULT 'bandit' CHECK (allocation IN ('bandit', 'experiment'));
CREATE INDEX decisions_experiment ON decisions (experiment_id) WHERE experiment_id IS NOT NULL;
