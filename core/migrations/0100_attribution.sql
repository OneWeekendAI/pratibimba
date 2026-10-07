-- B1: pluggable attribution. The site picks an attributor; a slot may override it (NULL follows
-- the site). `credit` is the share of a conversion a decision took (1 under last touch).
ALTER TABLE sites ADD COLUMN attribution TEXT NOT NULL DEFAULT 'last_touch' CHECK (attribution IN ('last_touch', 'linear'));
ALTER TABLE slots ADD COLUMN attribution TEXT CHECK (attribution IN ('last_touch', 'linear'));
ALTER TABLE decisions ADD COLUMN credit DOUBLE PRECISION CHECK (credit > 0 AND credit <= 1);
