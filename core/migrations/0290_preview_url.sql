-- S13: the page the dashboard's Preview room opens for a site (NULL: its origin, at /).
ALTER TABLE sites ADD COLUMN preview_url TEXT CHECK (preview_url IS NULL OR (preview_url ~ '^https?://' AND length(preview_url) <= 2048));
