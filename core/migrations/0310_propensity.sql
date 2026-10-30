-- A1: the probability the policy gave the arm it served, for the advisor's doubly robust uplift
-- (core/src/advisor/propensity.rs). Filled by the worker after a row settles; NULL kind = not yet.
--   replay      share of seeded Thompson replays of the row's own model version that pick its arm
--   single      the pool held one arm: 1
--   holdout     served the baseline by the holdout hash; not a policy choice (propensity NULL)
--   experiment  the experiment's weight for the arm among the arms open to the visitor
--   none        cannot be recomputed: on-device policy, cold start, or no model/features (NULL)
ALTER TABLE decisions
    ADD COLUMN propensity REAL CHECK (propensity IS NULL OR propensity > 0 AND propensity <= 1),
    ADD COLUMN propensity_kind TEXT CHECK (propensity_kind IN ('replay', 'single', 'holdout', 'experiment', 'none'));
CREATE INDEX decisions_propensity_todo ON decisions (settled_at) WHERE propensity_kind IS NULL AND settled_at IS NOT NULL;
