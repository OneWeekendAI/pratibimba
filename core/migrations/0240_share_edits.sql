-- G2: a live variant's traffic share is editable. A person edits it (action 'share' in the
-- approvals trail, with the value it replaced); an agent only proposes one (`share_requests`).
ALTER TABLE approvals DROP CONSTRAINT approvals_action_check;
ALTER TABLE approvals ADD CONSTRAINT approvals_action_check
    CHECK (action IN ('approve', 'reject', 'comment', 'retire', 'keep', 'share'));
-- The share a 'share' row replaced (NULL: the variant was uncapped).
ALTER TABLE approvals ADD COLUMN previous_share REAL;

-- MCP `update_slot` with `traffic_share` only PROPOSES: the variant keeps its share until a
-- person confirms. At most one open proposal per variant.
CREATE TABLE share_requests (
    id            BIGSERIAL PRIMARY KEY,
    variant_id    BIGINT NOT NULL REFERENCES variants(id) ON DELETE CASCADE,
    mcp_call_id   BIGINT REFERENCES mcp_calls(id) ON DELETE SET NULL,
    agent         TEXT NOT NULL CHECK (length(btrim(agent)) > 0),
    reason        TEXT NOT NULL CHECK (length(btrim(reason)) > 0),
    traffic_share REAL NOT NULL CHECK (traffic_share > 0 AND traffic_share <= 0.5),
    status        TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'confirmed', 'declined')),
    decided_by    TEXT,
    decided_at    TIMESTAMPTZ,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT share_requests_decided CHECK ((status = 'pending') = (decided_by IS NULL))
);
CREATE UNIQUE INDEX share_requests_one_open ON share_requests (variant_id) WHERE status = 'pending';
