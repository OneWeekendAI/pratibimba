-- P1: the per-site settings of the template facts (owner decision 2026-10-23: every default is a
-- per-site setting). `{top_item_days?, visit?, session_gap_minutes?}`; NULL or an absent key is
-- the default (180 days; a visit is a distinct UTC day with any event; a 30-minute session gap).
-- Bounds are checked at the door (history::Settings::validate), as for `sites.history`.
ALTER TABLE sites ADD COLUMN history_settings JSONB;
