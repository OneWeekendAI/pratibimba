-- A webhook for traffic share changes: `variant.share_changed`, sent whenever a live variant's
-- share changes (an owner's edit, a confirmed agent proposal, or the site file).
ALTER TABLE webhook_subscriptions DROP CONSTRAINT IF EXISTS webhook_subscriptions_events_check;
ALTER TABLE webhook_subscriptions ADD CONSTRAINT webhook_subscriptions_events_check CHECK (cardinality(events) > 0 AND events <@ ARRAY[
    'decision', 'exposure', 'conversion', 'variant.approved', 'variant.retired', 'variant.share_changed', 'auto_rollback']);
