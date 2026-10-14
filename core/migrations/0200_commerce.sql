-- F1: first-class commerce events.
--
-- An event may carry an order: `order_id` (the idempotency key; Core derives the event id from
-- it, so one order is one row), `value` and `currency` (ISO 4217, upper-case), and `items`.
ALTER TABLE events ADD COLUMN order_id TEXT CHECK (length(order_id) BETWEEN 1 AND 256);
ALTER TABLE events ADD COLUMN value NUMERIC(20, 4) CHECK (value >= 0);
ALTER TABLE events ADD COLUMN currency TEXT CHECK (currency ~ '^[A-Z]{3}$');
ALTER TABLE events ADD COLUMN items JSONB;
ALTER TABLE events ADD CONSTRAINT events_value_has_currency CHECK ((value IS NULL) = (currency IS NULL));

-- The view was created with e.*, which Postgres expands once: recreate it to see the new columns.
CREATE OR REPLACE VIEW live_events AS
    SELECT e.* FROM events e
     WHERE NOT EXISTS (SELECT 1 FROM erased_subjects x WHERE x.site_id = e.site_id AND x.subject = e.subject);

-- Revenue is counted in the site's currency; an order in another one is kept and reported, not
-- summed.
ALTER TABLE sites ADD COLUMN currency TEXT NOT NULL DEFAULT 'USD' CHECK (currency ~ '^[A-Z]{3}$');

-- The revenue a decision was credited (its attributor's share of each order in its window), and
-- when it last changed (the rollup's dirty-hour scan reads it).
ALTER TABLE decisions ADD COLUMN revenue NUMERIC(20, 4) CHECK (revenue >= 0);
ALTER TABLE decisions ADD COLUMN revenue_at TIMESTAMPTZ;
CREATE INDEX decisions_revenue_at ON decisions (revenue_at) WHERE revenue_at IS NOT NULL;

-- What the slot's bandit learns: `conversion` (the metric, as before) or `revenue` (the credited
-- revenue over `revenue_cap`, capped at 1).
ALTER TABLE slots ADD COLUMN goal TEXT NOT NULL DEFAULT 'conversion' CHECK (goal IN ('conversion', 'revenue'));
ALTER TABLE slots ADD COLUMN revenue_cap NUMERIC(20, 4) CHECK (revenue_cap > 0);
ALTER TABLE slots ADD CONSTRAINT slots_revenue_goal_has_cap CHECK (goal <> 'revenue' OR revenue_cap IS NOT NULL);

-- Rollups carry revenue and its sum of squares per cell (decision-level), for revenue per visitor
-- and its confidence sequence.
ALTER TABLE rollup_hourly ADD COLUMN revenue DOUBLE PRECISION NOT NULL DEFAULT 0;
ALTER TABLE rollup_hourly ADD COLUMN revenue_sq DOUBLE PRECISION NOT NULL DEFAULT 0;
ALTER TABLE rollup_daily ADD COLUMN revenue DOUBLE PRECISION NOT NULL DEFAULT 0;
ALTER TABLE rollup_daily ADD COLUMN revenue_sq DOUBLE PRECISION NOT NULL DEFAULT 0;
