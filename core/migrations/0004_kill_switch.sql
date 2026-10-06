-- The site-wide kill switch. While `paused_at` is set every slot on the site serves only its
-- baseline (the page's own content): one click in the dashboard, one click to resume.
ALTER TABLE sites
    ADD COLUMN paused_at  TIMESTAMPTZ,
    ADD COLUMN paused_by  TEXT CHECK (paused_by IS NULL OR length(paused_by) BETWEEN 1 AND 128),
    ADD CONSTRAINT sites_paused_has_actor CHECK (paused_at IS NULL OR paused_by IS NOT NULL);
