-- Phase 2.5: the insights pipeline. Rollups the dashboard reads instead of raw events,
-- segments (hand-authored or discovered), and the visitor-local clock.
--
-- Nothing here holds a demographic field: segment predicates are checked against a closed
-- allowlist of behaviour and situation features in `insights::segments`.

-- The visitor's UTC offset in minutes east, as the SDK reported it with the event
-- (-getTimezoneOffset()). NULL means unknown; the rollups then fall back to UTC.
ALTER TABLE events
    ADD COLUMN tz_offset_min SMALLINT CHECK (tz_offset_min IS NULL OR tz_offset_min BETWEEN -720 AND 840);

-- `is_night_local` per site; the window wraps midnight when start > end. Default 22:00-05:00.
ALTER TABLE sites
    ADD COLUMN night_start_hour SMALLINT NOT NULL DEFAULT 22 CHECK (night_start_hour BETWEEN 0 AND 23),
    ADD COLUMN night_end_hour   SMALLINT NOT NULL DEFAULT 5  CHECK (night_end_hour BETWEEN 0 AND 23);

-- A segment is a durable name for a group of visit contexts. `draft` is a discovery suggestion
-- (not rolled up, never used in a decision); `active` is tracked by the rollups; `audience` is
-- active plus the permission to be used in a decision. Segment 0 is implicit: everyone.
CREATE TABLE segments (
    id             BIGSERIAL PRIMARY KEY,
    site_id        TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    name           TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    predicate      JSONB NOT NULL,
    predicate_key  TEXT NOT NULL,                  -- canonical form, so discovery never duplicates
    origin         TEXT NOT NULL CHECK (origin IN ('authored', 'discovered')),
    status         TEXT NOT NULL CHECK (status IN ('draft', 'active', 'audience', 'archived')),
    evidence       JSONB NOT NULL DEFAULT '{}',    -- what discovery saw, from rollups
    created_by     TEXT,
    promoted_by    TEXT CHECK (promoted_by IS NULL OR length(promoted_by) BETWEEN 1 AND 128),
    promoted_at    TIMESTAMPTZ,
    -- Backfill cursor: the next UTC day a background pass must recompute for this segment.
    -- NULL means the rollups are complete for it.
    backfill_from  DATE,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (site_id, predicate_key),
    CONSTRAINT segments_audience_has_promoter CHECK (status <> 'audience' OR promoted_by IS NOT NULL)
);

-- One row per (hour, local hour, segment, slot, variant, context cell, outcome): the count of
-- exposed decisions. `ctx` is the bitmask of the decision's boolean context (1 returning,
-- 2 mobile, 4 from_ad); `dow_local` is 0 = Sunday. Outcome is the decision's settled result.
-- Partitioned by month on hour_utc; the job creates a month's partition before writing it.
CREATE TABLE rollup_hourly (
    site_id     TEXT NOT NULL,
    hour_utc    TIMESTAMPTZ NOT NULL,
    hour_local  SMALLINT NOT NULL CHECK (hour_local BETWEEN 0 AND 23),
    dow_local   SMALLINT NOT NULL CHECK (dow_local BETWEEN 0 AND 6),
    segment_id  BIGINT NOT NULL,                   -- 0 = everyone
    slot_id     BIGINT NOT NULL,
    variant_id  BIGINT NOT NULL,
    ctx         SMALLINT NOT NULL,
    outcome     TEXT NOT NULL CHECK (outcome IN ('converted', 'not_converted', 'pending')),
    n           BIGINT NOT NULL CHECK (n > 0),
    tz_known    BIGINT NOT NULL DEFAULT 0,         -- how many of n carried a reported offset
    PRIMARY KEY (site_id, hour_utc, hour_local, dow_local, segment_id, slot_id, variant_id, ctx, outcome)
) PARTITION BY RANGE (hour_utc);
CREATE INDEX rollup_hourly_local ON rollup_hourly (site_id, hour_local, segment_id);
CREATE INDEX rollup_hourly_outcome ON rollup_hourly (site_id, segment_id, slot_id, outcome);

-- The same at a UTC-day grain; the lift headline and the learnings feed read it.
CREATE TABLE rollup_daily (
    site_id     TEXT NOT NULL,
    day         DATE NOT NULL,
    segment_id  BIGINT NOT NULL,
    slot_id     BIGINT NOT NULL,
    variant_id  BIGINT NOT NULL,
    ctx         SMALLINT NOT NULL,
    outcome     TEXT NOT NULL CHECK (outcome IN ('converted', 'not_converted', 'pending')),
    n           BIGINT NOT NULL CHECK (n > 0),
    PRIMARY KEY (site_id, day, segment_id, slot_id, variant_id, ctx, outcome)
) PARTITION BY RANGE (day);
CREATE INDEX rollup_daily_segment ON rollup_daily (site_id, segment_id, slot_id, outcome);

-- The journey sketch: a visitor's UTC day as an ordered, bounded list of (slot, variant)
-- exposures with the day's final outcome, hashed to 64 bits and counted.
CREATE TABLE rollup_journey (
    site_id      TEXT NOT NULL,
    day          DATE NOT NULL,
    fingerprint  BIGINT NOT NULL,
    steps        JSONB NOT NULL,                   -- [{"slot":id,"variant":id}, ...]
    converted    BOOLEAN NOT NULL,
    visitors     BIGINT NOT NULL CHECK (visitors > 0),
    PRIMARY KEY (site_id, day, fingerprint, converted)
);

-- Cohort retention: visitors first seen in cohort_week (by segment of their first-week visits)
-- who were active again in return_week. One matrix per site.
CREATE TABLE rollup_retention (
    site_id      TEXT NOT NULL,
    cohort_week  DATE NOT NULL,
    return_week  DATE NOT NULL,
    segment_id   BIGINT NOT NULL,
    visitors     BIGINT NOT NULL CHECK (visitors >= 0),
    PRIMARY KEY (site_id, cohort_week, return_week, segment_id)
);

-- The retention job's own summary: which ISO weeks each subject was active in, and which
-- segments its exposures matched that week. Maintained incrementally from new events.
CREATE TABLE insight_visitor_weeks (
    site_id   TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    subject   TEXT NOT NULL,
    week      DATE NOT NULL,
    segments  BIGINT[] NOT NULL DEFAULT '{}',
    PRIMARY KEY (site_id, subject, week)
);

-- Job watermarks and the last nightly runs.
CREATE TABLE insight_jobs (
    name     TEXT PRIMARY KEY,
    ran_at   TIMESTAMPTZ NOT NULL,
    cursor   TIMESTAMPTZ NOT NULL
);

CREATE INDEX decisions_exposed ON decisions (exposed_at) WHERE exposed_at IS NOT NULL;
CREATE INDEX decisions_settled ON decisions (settled_at) WHERE settled_at IS NOT NULL;
CREATE INDEX events_received ON events (received_at);
