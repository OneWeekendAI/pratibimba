-- The site's brand/style file (brand.md), as text, declared in the site file. Read-only over /api.
ALTER TABLE sites ADD COLUMN brand TEXT;
