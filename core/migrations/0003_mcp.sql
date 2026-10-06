-- Phase 2: the MCP server. Per-site tokens, and an audit row for every tool call.

-- A token is scoped to one site. Only its SHA-256 is stored; the token is shown once.
CREATE TABLE mcp_tokens (
    id          BIGSERIAL PRIMARY KEY,
    site_id     TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    name        TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 128),
    token_hash  TEXT NOT NULL UNIQUE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at  TIMESTAMPTZ
);

-- The audit stub. Every tools/call writes one row, refused or not. A call that ran (ok or
-- error) always carries a justification; only a refused call may lack one.
CREATE TABLE mcp_calls (
    id             BIGSERIAL PRIMARY KEY,
    token_id       BIGINT NOT NULL REFERENCES mcp_tokens(id) ON DELETE CASCADE,
    site_id        TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    call_id        JSONB NOT NULL DEFAULT 'null',     -- the JSON-RPC request id
    tool           TEXT NOT NULL,
    agent          TEXT NOT NULL DEFAULT '',          -- self-reported, e.g. "Claude Code"
    on_behalf_of   TEXT,                               -- the person the agent works for
    justification  TEXT,
    arguments      JSONB NOT NULL DEFAULT '{}',
    outcome        TEXT NOT NULL CHECK (outcome IN ('ok', 'error', 'refused')),
    error          TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT mcp_calls_ran_with_reason CHECK (outcome = 'refused' OR (length(btrim(justification)) > 0 AND length(btrim(agent)) > 0))
);
CREATE INDEX mcp_calls_site ON mcp_calls (site_id, created_at DESC);

ALTER TABLE variants DROP CONSTRAINT variants_author_check;
ALTER TABLE variants
    ADD CONSTRAINT variants_author_check CHECK (author IN ('human', 'ai', 'agent')),
    -- The tool call that proposed this draft; its agent and justification feed the approval card.
    ADD COLUMN mcp_call_id BIGINT REFERENCES mcp_calls(id) ON DELETE SET NULL;
