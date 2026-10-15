-- F4: copy slots. A `copy` slot's variants are plain-text fields; payload holds them as a JSON object.
ALTER TABLE slots DROP CONSTRAINT IF EXISTS slots_kind_check;
ALTER TABLE slots ADD CONSTRAINT slots_kind_check CHECK (kind IN ('swap', 'picker', 'sorted', 'copy'));
-- `{fields: [{name, max, required}]}`: the declared fields.
ALTER TABLE slots ADD COLUMN copy JSONB;
ALTER TABLE slots ADD CONSTRAINT slots_copy_has_spec CHECK ((kind = 'copy') = (copy IS NOT NULL));
ALTER TABLE slots ADD CONSTRAINT slots_copy_not_in_session CHECK (NOT (kind = 'copy' AND in_session));
