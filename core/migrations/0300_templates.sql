-- P1: templated copy. `requires` names the template paths a variant cannot do without: a
-- visitor missing one is not offered the arm (decide closes it, as an audience gate does). The
-- baseline is the page's own content, offered to everyone, so it never requires anything.
ALTER TABLE variants ADD COLUMN requires TEXT[] NOT NULL DEFAULT '{}';
ALTER TABLE variants ADD CONSTRAINT variants_baseline_requires_nothing CHECK (NOT is_baseline OR cardinality(requires) = 0);
-- The copy fields a decision showed, filled from the visitor's history; NULL when nothing was templated.
ALTER TABLE decisions ADD COLUMN filled JSONB;
