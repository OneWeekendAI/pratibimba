-- F2: sorted slots. A `sorted` slot's variants are items, and a decision serves an order of them.
ALTER TABLE slots DROP CONSTRAINT IF EXISTS slots_kind_check;
ALTER TABLE slots ADD CONSTRAINT slots_kind_check CHECK (kind IN ('swap', 'picker', 'sorted'));
-- `{visible, order: [item keys]}`: the cascade depth and the declared default order.
ALTER TABLE slots ADD COLUMN sorted JSONB;
ALTER TABLE slots ADD CONSTRAINT slots_sorted_has_spec CHECK ((kind = 'sorted') = (sorted IS NOT NULL));
ALTER TABLE slots ADD CONSTRAINT slots_sorted_not_in_session CHECK (NOT (kind = 'sorted' AND in_session));
-- The order served, variant ids best first; `variant_id` is its first element.
ALTER TABLE decisions ADD COLUMN ranking BIGINT[];
