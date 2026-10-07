-- C1: installed plugins. A plugin is a compiled-in, named extension of a known kind
-- (crate::plugins::CATALOGUE); this table records which ones a site installed, with what config,
-- by whom. `deanonymiser` is copied from the catalogue so the site warning is one query.
CREATE TABLE plugins (
    site_id          TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    name             TEXT NOT NULL,
    kind             TEXT NOT NULL CHECK (kind IN ('attributor', 'event_sink')),
    config           JSONB NOT NULL DEFAULT '{}',
    deanonymiser     BOOLEAN NOT NULL,
    -- An attributor plugin's attributor, which the worker uses unless a slot names its own.
    attributor       TEXT CHECK (attributor IN ('last_touch', 'linear')),
    -- An event sink's webhook subscription; uninstalling deletes it.
    subscription_id  BIGINT REFERENCES webhook_subscriptions(id) ON DELETE SET NULL,
    installed_by     TEXT NOT NULL CHECK (length(btrim(installed_by)) > 0),
    installed_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (site_id, name),
    CONSTRAINT plugins_attributor_kind CHECK ((kind = 'attributor') = (attributor IS NOT NULL))
);
-- One attributor per site.
CREATE UNIQUE INDEX plugins_one_attributor ON plugins (site_id) WHERE kind = 'attributor';
