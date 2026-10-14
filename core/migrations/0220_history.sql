-- F3: visitor history. The site file's `history:` block (learner features from the visitor's
-- own stored orders and item events); NULL when the site does not use history in the learner.
ALTER TABLE sites ADD COLUMN history JSONB;
