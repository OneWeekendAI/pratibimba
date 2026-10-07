-- E4 delayed feedback: a credited conversion is recorded at once but taught to the model only
-- when the decision's attribution window closes, alongside the misses (see worker.rs,
-- settle_batch). `learn_pending` marks a credit whose lesson is still owed. It defaults to false,
-- so every decision credited by an older worker (which learned at the click) is never taught
-- twice, whatever order the old and new workers ran in.
ALTER TABLE decisions ADD COLUMN learn_pending BOOLEAN NOT NULL DEFAULT false;
CREATE INDEX decisions_learn_pending ON decisions (created_at) WHERE learn_pending;
