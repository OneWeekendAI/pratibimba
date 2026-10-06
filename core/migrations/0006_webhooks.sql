-- Phase 2: webhooks out. A subscription names a URL and the event types it wants; the secret
-- signs every payload (HMAC-SHA256) and is shown once, when the subscription is created.
CREATE TABLE webhook_subscriptions (
    id          BIGSERIAL PRIMARY KEY,
    site_id     TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    url         TEXT NOT NULL CHECK (url ~ '^https?://' AND length(url) <= 2048),
    events      TEXT[] NOT NULL CHECK (cardinality(events) > 0 AND events <@ ARRAY[
                    'decision', 'exposure', 'conversion', 'variant.approved', 'variant.retired', 'auto_rollback']),
    secret      TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX webhook_subscriptions_site ON webhook_subscriptions (site_id);

-- The webhook outbox and its delivery record, one row per (event, subscriber). Rows are written
-- in the same transaction as the change they report, so an event is never lost and never sent
-- for a change that rolled back. The idempotency key is per event, shared by its subscribers and
-- kept across retries.
CREATE TABLE webhook_deliveries (
    id               BIGSERIAL PRIMARY KEY,
    subscription_id  BIGINT NOT NULL REFERENCES webhook_subscriptions(id) ON DELETE CASCADE,
    event            TEXT NOT NULL,
    idempotency_key  UUID NOT NULL,
    payload          JSONB NOT NULL,
    status           TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'delivered', 'failed')),
    attempts         INTEGER NOT NULL DEFAULT 0,
    next_attempt_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_status      INTEGER,
    last_error       TEXT,
    delivered_at     TIMESTAMPTZ,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (subscription_id, idempotency_key)
);
CREATE INDEX webhook_deliveries_due ON webhook_deliveries (next_attempt_at) WHERE status = 'pending';
CREATE INDEX webhook_deliveries_sub ON webhook_deliveries (subscription_id, id DESC);
