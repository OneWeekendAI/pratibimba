-- S12b: responsive variants. An uploaded PNG, JPEG or WebP wider than a step of the width ladder
-- (320, 640, 960, 1280, 1920) is also stored resized to each smaller step, under
-- `<site>/<sha256>.w<width>`. `widths` lists the steps stored, ascending; empty for an image too
-- small, a type Core does not resize (SVG, GIF, AVIF) or bytes it could not decode.
ALTER TABLE assets ADD COLUMN widths INT[] NOT NULL DEFAULT '{}';
