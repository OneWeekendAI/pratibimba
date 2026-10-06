-- Pratibimba Phase 0 schema: the five central tables (sites, slots, variants, events,
-- decisions) plus the two the worker cannot run without (models, pending_events).

CREATE TABLE sites (
    id          TEXT PRIMARY KEY,                     -- slug, e.g. "demo"
    origin      TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE slots (
    id                       BIGSERIAL PRIMARY KEY,
    site_id                  TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    key                      TEXT NOT NULL,           -- "hero"
    selector                 TEXT NOT NULL,           -- CSS selector the script hides and swaps
    metric                   TEXT NOT NULL DEFAULT 'click',  -- the event name that counts as success
    attribution_window_secs  INTEGER NOT NULL DEFAULT 1800 CHECK (attribution_window_secs > 0),
    decision_ttl_secs        INTEGER NOT NULL DEFAULT 60 CHECK (decision_ttl_secs >= 0),
    created_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (site_id, key)
);

CREATE TABLE variants (
    id           BIGSERIAL PRIMARY KEY,
    slot_id      BIGINT NOT NULL REFERENCES slots(id) ON DELETE CASCADE,
    key          TEXT NOT NULL,
    payload      TEXT,                                -- HTML fragment; NULL = the page's own content
    is_baseline  BOOLEAN NOT NULL DEFAULT false,
    author       TEXT NOT NULL DEFAULT 'human' CHECK (author IN ('human', 'ai')),
    status       TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'approved', 'retired')),
    approved_by  TEXT,
    approved_at  TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (slot_id, key),
    -- Nothing reaches a visitor without a named approver.
    CONSTRAINT variants_approved_has_approver CHECK (status <> 'approved' OR approved_by IS NOT NULL)
);
CREATE UNIQUE INDEX variants_one_baseline ON variants (slot_id) WHERE is_baseline;

CREATE TABLE models (
    slot_id     BIGINT PRIMARY KEY REFERENCES slots(id) ON DELETE CASCADE,
    policy      TEXT NOT NULL DEFAULT 'lints',
    version     BIGINT NOT NULL DEFAULT 0,
    state       JSONB NOT NULL DEFAULT '{}',
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE decisions (
    id              UUID PRIMARY KEY,
    site_id         TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    slot_id         BIGINT NOT NULL REFERENCES slots(id) ON DELETE CASCADE,
    variant_id      BIGINT NOT NULL REFERENCES variants(id),
    subject         TEXT NOT NULL,
    pool            BIGINT[] NOT NULL,                -- approved arms at decision time
    features        JSONB NOT NULL,                   -- by name, e.g. {"bias":1,"returning":0,...}
    scores          JSONB NOT NULL,                   -- variant id -> sampled score
    policy          TEXT NOT NULL,
    policy_version  BIGINT NOT NULL,
    seed            BIGINT NOT NULL,                  -- u64 stored bit-for-bit
    in_holdout      BOOLEAN NOT NULL DEFAULT false,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    exposed_at      TIMESTAMPTZ,                      -- set when the script reports it was shown
    outcome         SMALLINT CHECK (outcome IN (0, 1)),
    outcome_at      TIMESTAMPTZ,
    settled_at      TIMESTAMPTZ                       -- exactly-once guard for learning
);
CREATE INDEX decisions_slot_subject ON decisions (slot_id, subject, created_at DESC);
CREATE INDEX decisions_unsettled ON decisions (created_at) WHERE settled_at IS NULL;

CREATE TABLE events (
    id           UUID PRIMARY KEY,                    -- client-generated idempotency key
    site_id      TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    subject      TEXT NOT NULL,
    name         TEXT NOT NULL,
    slot_id      BIGINT REFERENCES slots(id) ON DELETE SET NULL,
    decision_id  UUID,                                -- not an FK: a stale id must not fail ingest
    received_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX events_site_subject ON events (site_id, subject, received_at);

CREATE TABLE pending_events (
    id        BIGSERIAL PRIMARY KEY,
    event_id  UUID NOT NULL REFERENCES events(id) ON DELETE CASCADE
);
