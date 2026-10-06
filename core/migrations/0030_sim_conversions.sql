-- Phase 3: the simulator's sandbox sites and inbound (platform) conversion sources.

-- A sandbox is a copy of a real site under its own id ("<site>~sim", never a valid slug, so
-- no real site can collide with one). Everything simulated is keyed by the sandbox id: its own
-- slots, variants, models, decisions and events. Real results and models are per site and per
-- slot, so they cannot see it. The sandbox runs on its own clock, `sim_now`; the real worker
-- and the site list skip sandboxes.
ALTER TABLE sites
    ADD COLUMN sandbox_of  TEXT REFERENCES sites(id) ON DELETE CASCADE,
    ADD COLUMN sim_now     TIMESTAMPTZ,
    ADD CONSTRAINT sites_sandbox_has_clock CHECK ((sandbox_of IS NULL) = (sim_now IS NULL));

-- Platform conversion sources. Secrets are kept in the clear because verifying an HMAC needs
-- them; the API never returns them.
ALTER TABLE sites
    ADD COLUMN stripe_signing_secret      TEXT,
    ADD COLUMN conversion_signing_secret  TEXT,
    -- Goals a platform reports. A browser event with one of these names is a hint, never a
    -- conversion; only a platform event credits it.
    ADD COLUMN authoritative_goals        TEXT[] NOT NULL DEFAULT '{}',
    -- How long past its window a decision on an authoritative goal waits for a late webhook
    -- before it settles as a miss.
    ADD COLUMN platform_grace_secs        INTEGER NOT NULL DEFAULT 3600 CHECK (platform_grace_secs >= 0);

-- Where an event came from. Platform events carry the platform's own id (a Stripe checkout
-- session, an order id) and the time the platform says it happened; their row id is derived
-- from (site, source, external id), so a redelivery is absorbed by the primary key.
ALTER TABLE events
    ADD COLUMN source       TEXT NOT NULL DEFAULT 'browser' CHECK (source IN ('browser', 'stripe', 'conversion')),
    ADD COLUMN external_id  TEXT,
    ADD COLUMN occurred_at  TIMESTAMPTZ,
    ADD CONSTRAINT events_platform_has_external_id CHECK ((source = 'browser') = (external_id IS NULL));
CREATE INDEX events_hints ON events (site_id, subject, name, received_at DESC) WHERE source = 'browser' AND decision_id IS NOT NULL;
