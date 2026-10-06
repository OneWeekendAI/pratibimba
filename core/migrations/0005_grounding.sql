-- The site's facts.md: the statements an agent may rely on when it writes copy. The grounding
-- check flags any figure in an agent draft that these facts do not contain. NULL: none given.
ALTER TABLE sites ADD COLUMN facts TEXT CHECK (facts IS NULL OR length(facts) <= 65536);
