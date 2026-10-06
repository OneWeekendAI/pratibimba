-- Phase 2.5: the taste picker and in-session adaptation.
--
-- A slot is a `swap` (the variant's HTML replaces the slot's) or a `picker` (the SDK renders the
-- owner's options; its two variants `none` and `picker` test whether showing it pays).
-- `in_session` slots ship their payloads and linear weights in the manifest so the SDK can
-- re-score them on the device. Picker slots are never re-scored: the visitor is using them.
ALTER TABLE slots
    ADD COLUMN kind        TEXT NOT NULL DEFAULT 'swap' CHECK (kind IN ('swap', 'picker')),
    ADD COLUMN in_session  BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN picker      JSONB,                    -- {"max_picks": 1-3, "options": [{"key", "label"}]}
    ADD CONSTRAINT slots_picker_has_options CHECK ((kind = 'picker') = (picker IS NOT NULL)),
    ADD CONSTRAINT slots_picker_not_in_session CHECK (NOT (kind = 'picker' AND in_session));

-- A `pick` event's option keys. Only a pick stores anything; a skip is a plain event.
ALTER TABLE events
    ADD COLUMN picks TEXT[] CHECK (picks IS NULL OR cardinality(picks) BETWEEN 1 AND 3);
CREATE INDEX events_picks ON events (site_id, subject, received_at DESC) WHERE picks IS NOT NULL;

-- On-device decisions arrive through the event stream and are written here as `client`.
-- `divergence` is |client score - server posterior mean| for the same features; a large one
-- means the shipped weights were stale, and the server's next full decision wins.
ALTER TABLE decisions
    ADD COLUMN source      TEXT NOT NULL DEFAULT 'server' CHECK (source IN ('server', 'client')),
    ADD COLUMN divergence  REAL;
