-- A1: the advisor's cards (core/src/advisor/). The advisor never changes live traffic; it writes
-- these rows only, and a person approves or rejects each one (B8). One card type in A1:
-- 'pause_slot', "this slot makes no difference". `evidence` holds the claim sentence and the
-- numbers behind it ({claim, rows, arms, held_out, visits_per_week, state, window, p, margin}).
CREATE TABLE advisor_cards (
    id          BIGSERIAL PRIMARY KEY,
    site_id     TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    slot_id     BIGINT NOT NULL REFERENCES slots(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN ('pause_slot')),
    state       TEXT NOT NULL CHECK (state IN ('early', 'confident')),
    evidence    JSONB NOT NULL,
    status      TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'rejected')),
    decided_by  TEXT,
    decided_at  TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT advisor_cards_decided CHECK ((status = 'pending') = (decided_by IS NULL))
);
-- One open card per slot and kind (as retire_requests_one_open).
CREATE UNIQUE INDEX advisor_cards_one_open ON advisor_cards (slot_id, kind) WHERE status = 'pending';
CREATE INDEX advisor_cards_site ON advisor_cards (site_id, created_at);

-- The advisor's per-site thresholds (owner decision 2026-10-23: every default is a per-site
-- setting): {min_rows?, margin?, held_out_days?, fdr_q?, cards_per_week?, quiet_days?}; NULL or an
-- absent key is the default (300, 0.3 orders per 100, 14, 0.10, 3, 30). Bounds are checked at
-- the door (advisor::pause::Settings::validate).
ALTER TABLE sites ADD COLUMN advisor_settings JSONB;
