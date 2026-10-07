// The planted ground truth for cohort.mjs: the hero's click probability per exposure, by the
// visitor's context. See the table in cohort.mjs's header; analyze.mjs scores learning by it.
// `evening` is the visitor's local hour >= 17 (the run's zones never reach past midnight).
export const TRUTH_TABLE = {
  baseline: (c) => (c.returning ? 0.15 : 0.05),
  'filter-angle': (c) => (c.from_ad ? 0.16 : 0.05),
  'price-upfront': (c) => (c.mobile ? 0.2 : 0.04),
  'tasting-notes': (c) => (c.mobile ? 0.04 : c.evening ? 0.22 : 0.07),
};
