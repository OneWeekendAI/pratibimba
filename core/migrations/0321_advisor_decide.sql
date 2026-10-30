-- A1 (B8): a person approves an advisor card, which pauses every approved non-baseline variant
-- of its slot. Each paused variant gets an approvals row, action 'pause', naming the person.
ALTER TABLE approvals DROP CONSTRAINT approvals_action_check;
ALTER TABLE approvals ADD CONSTRAINT approvals_action_check
    CHECK (action IN ('approve', 'reject', 'comment', 'retire', 'keep', 'share', 'pause'));
-- The card a pause came from; NULL for every other row.
ALTER TABLE approvals ADD COLUMN advisor_card_id BIGINT REFERENCES advisor_cards(id) ON DELETE SET NULL;
