-- The page a slot lives on (a URL path such as /product/x), declared in the site file or by an agent.
-- NULL: undeclared.
ALTER TABLE slots ADD COLUMN page TEXT CHECK (page IS NULL OR (page LIKE '/%' AND length(page) <= 512));
