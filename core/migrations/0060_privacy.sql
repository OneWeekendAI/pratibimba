-- Phase 4: privacy. Consent modes, the minimum-collection whitelist, the visitor summary and
-- right-to-erase (backend.md, "Privacy" and "Data model").

-- `cookieless`: the stored subject is a pseudonym that changes every UTC day (see
-- `privacy_salts`) and nothing cookie-derived is used. `full`: the subject is stored as the
-- client sent it. `consent_aware`: full for a request that says the visitor consented,
-- cookieless otherwise. Sites that existed before this migration keep the behaviour they had
-- (`full`); a new site starts at the documented default, `consent_aware`.
ALTER TABLE sites
    ADD COLUMN privacy_mode TEXT NOT NULL DEFAULT 'full'
        CHECK (privacy_mode IN ('cookieless', 'consent_aware', 'full')),
    -- The optional request fields this site collects. NULL: every field. A field outside the
    -- list is dropped at ingest, never written. The names are checked in `privacy::FIELDS`.
    ADD COLUMN fields_allowed TEXT[];
ALTER TABLE sites ALTER COLUMN privacy_mode SET DEFAULT 'consent_aware';

-- One salt per site per UTC day. A cookieless pseudonym is sha256(salt, site, client id); the
-- previous day's salt is deleted when the next day's is made, so yesterday's pseudonyms can
-- never be linked to today's, or back to a client id. Backups leave this table's rows out.
CREATE TABLE privacy_salts (
    site_id  TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    day      DATE NOT NULL,
    salt     TEXT NOT NULL,
    PRIMARY KEY (site_id, day)
);

-- A row per subject: a small summary of what the visitor did, updated in the ingest commit.
-- `recent` is the last 20 event names, oldest first. The full history lives in `events`.
CREATE TABLE visitors (
    site_id     TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    subject     TEXT NOT NULL,
    first_seen  TIMESTAMPTZ NOT NULL,
    last_seen   TIMESTAMPTZ NOT NULL,
    events      BIGINT NOT NULL CHECK (events > 0),
    recent      TEXT[] NOT NULL CHECK (cardinality(recent) BETWEEN 1 AND 20),
    PRIMARY KEY (site_id, subject)
);

-- Right-to-erase: the audit row and the tombstone in one. `subject_hash` is
-- sha256(site, 0, subject) in hex; the subject itself is never kept. A request carrying a
-- subject with a row here is refused, so an old cookie cannot bring the visitor back.
CREATE TABLE erasures (
    id            BIGSERIAL PRIMARY KEY,
    site_id       TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    subject_hash  TEXT NOT NULL CHECK (subject_hash ~ '^[0-9a-f]{64}$'),
    actor         TEXT NOT NULL CHECK (length(btrim(actor)) BETWEEN 1 AND 128),
    reason        TEXT CHECK (reason IS NULL OR length(reason) <= 2000),
    counts        JSONB NOT NULL DEFAULT '{}',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX erasures_tombstone ON erasures (site_id, subject_hash);

-- An erasure deletes the visitor's events, and the rollups read a decision's UTC offset from
-- its events. The offset is copied here first, so a later rebuild puts the (now anonymous)
-- decision in the same local hour it was counted in.
ALTER TABLE decisions
    ADD COLUMN tz_offset_min SMALLINT CHECK (tz_offset_min IS NULL OR tz_offset_min BETWEEN -720 AND 840);
