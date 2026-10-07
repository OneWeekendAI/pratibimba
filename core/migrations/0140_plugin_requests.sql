-- C2: an agent's install_plugin lands here, pending, until a person confirms (which installs
-- the plugin with the requested config) or declines (with a reason the agent reads back).
CREATE TABLE plugin_requests (
    id           BIGSERIAL PRIMARY KEY,
    site_id      TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    config       JSONB NOT NULL DEFAULT '{}',
    mcp_call_id  BIGINT REFERENCES mcp_calls(id) ON DELETE SET NULL,
    agent        TEXT NOT NULL CHECK (length(btrim(agent)) > 0),
    reason       TEXT NOT NULL CHECK (length(btrim(reason)) > 0),
    status       TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'confirmed', 'declined')),
    decided_by   TEXT,
    decided_at   TIMESTAMPTZ,
    comment      TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT plugin_requests_decided CHECK ((status = 'pending') = (decided_by IS NULL))
);
CREATE UNIQUE INDEX plugin_requests_one_open ON plugin_requests (site_id, name) WHERE status = 'pending';
