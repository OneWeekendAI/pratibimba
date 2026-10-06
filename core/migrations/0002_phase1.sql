-- Phase 1: holdout, guardrails, the approval gate, versioned model state, learnings.

ALTER TABLE sites
    ADD COLUMN holdout_fraction        REAL NOT NULL DEFAULT 0.10
        CHECK (holdout_fraction >= 0 AND holdout_fraction <= 0.5),
    -- Auto-rollback pauses a variant whose lift interval sits wholly below -rollback_margin.
    ADD COLUMN rollback_margin         REAL NOT NULL DEFAULT 0.01 CHECK (rollback_margin >= 0),
    ADD COLUMN rollback_min_exposures  INTEGER NOT NULL DEFAULT 200 CHECK (rollback_min_exposures > 0),
    -- Claims a variant may never make ("guaranteed", "free"); checked before approval.
    ADD COLUMN forbidden_claims        TEXT[] NOT NULL DEFAULT '{}';

ALTER TABLE variants DROP CONSTRAINT variants_status_check;
ALTER TABLE variants
    ADD CONSTRAINT variants_status_check CHECK (status IN ('draft', 'approved', 'paused', 'rejected', 'retired')),
    -- The planned traffic share: an eligibility ceiling, hard-capped at half of all visitors.
    ADD COLUMN traffic_share           REAL CHECK (traffic_share IS NULL OR (traffic_share > 0 AND traffic_share <= 0.5)),
    -- The slot's model version when this variant went live; auto-rollback restores it.
    ADD COLUMN approved_model_version  BIGINT,
    ADD COLUMN paused_at               TIMESTAMPTZ,
    ADD COLUMN pause_reason            JSONB;

-- The audit log of the gate: who approved, rejected or commented on what, against which checks.
CREATE TABLE approvals (
    id             BIGSERIAL PRIMARY KEY,
    variant_id     BIGINT NOT NULL REFERENCES variants(id) ON DELETE CASCADE,
    action         TEXT NOT NULL CHECK (action IN ('approve', 'reject', 'comment')),
    actor          TEXT NOT NULL CHECK (length(actor) BETWEEN 1 AND 128),
    comment        TEXT,
    traffic_share  REAL,
    checks         JSONB,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX approvals_variant ON approvals (variant_id, created_at);

-- Every model write (an update or a restore) appends its state, so a decision can be replayed
-- against exactly the state that scored it, and a rollback has something to restore.
CREATE TABLE model_versions (
    slot_id     BIGINT NOT NULL REFERENCES slots(id) ON DELETE CASCADE,
    version     BIGINT NOT NULL,
    state       JSONB NOT NULL,
    reason      TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (slot_id, version)
);
INSERT INTO model_versions (slot_id, version, state, reason) SELECT slot_id, version, state, 'migrated' FROM models;

-- Declarative sentences the system writes when something durable happens.
CREATE TABLE learnings (
    id          BIGSERIAL PRIMARY KEY,
    site_id     TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    slot_id     BIGINT REFERENCES slots(id) ON DELETE CASCADE,
    variant_id  BIGINT REFERENCES variants(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL,
    sentence    TEXT NOT NULL,
    data        JSONB NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX learnings_site ON learnings (site_id, created_at DESC);

CREATE INDEX decisions_site_created ON decisions (site_id, created_at DESC);
CREATE INDEX events_decision ON events (decision_id) WHERE decision_id IS NOT NULL;
