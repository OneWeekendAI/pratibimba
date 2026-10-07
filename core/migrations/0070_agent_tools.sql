-- Phase 4: the MCP slot tools and retirement proposals.

-- A slot an agent created over MCP (`create_slot`). Its baseline starts as a DRAFT, so the
-- slot is in no manifest and decides nothing until a person approves that baseline in the
-- dashboard, which is what activates the slot.
ALTER TABLE slots
    ADD COLUMN created_by  TEXT,
    ADD COLUMN mcp_call_id BIGINT REFERENCES mcp_calls(id) ON DELETE SET NULL;

-- The human answers to a retirement proposal sit in the variant's comment thread.
ALTER TABLE approvals DROP CONSTRAINT approvals_action_check;
ALTER TABLE approvals ADD CONSTRAINT approvals_action_check CHECK (action IN ('approve', 'reject', 'comment', 'retire', 'keep'));

-- `retire_variant` over MCP only PROPOSES: the variant keeps serving until a person confirms.
-- At most one open proposal per variant.
CREATE TABLE retire_requests (
    id           BIGSERIAL PRIMARY KEY,
    variant_id   BIGINT NOT NULL REFERENCES variants(id) ON DELETE CASCADE,
    mcp_call_id  BIGINT REFERENCES mcp_calls(id) ON DELETE SET NULL,
    agent        TEXT NOT NULL CHECK (length(btrim(agent)) > 0),
    reason       TEXT NOT NULL CHECK (length(btrim(reason)) > 0),
    status       TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'confirmed', 'declined')),
    decided_by   TEXT,
    decided_at   TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT retire_requests_decided CHECK ((status = 'pending') = (decided_by IS NULL))
);
CREATE UNIQUE INDEX retire_requests_one_open ON retire_requests (variant_id) WHERE status = 'pending';
