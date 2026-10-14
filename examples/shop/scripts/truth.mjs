// The planted ground truth for cohort.mjs: the hero's click probability per exposure, by the
// visitor's context. See the table in cohort.mjs's header; analyze.mjs scores learning by it.
// `evening` is the visitor's local hour >= 17 (the run's zones never reach past midnight).
export const TRUTH_TABLE = {
  baseline: (c) => (c.returning ? 0.15 : 0.05),
  'filter-angle': (c) => (c.from_ad ? 0.16 : 0.05),
  'price-upfront': (c) => (c.mobile ? 0.2 : 0.04),
  'tasting-notes': (c) => (c.mobile ? 0.04 : c.evening ? 0.22 : 0.07),
};

// F2: the planted ground truth for the sorted product grid. A visitor scans the rendered cards
// top to bottom and clicks a card with its probability here (at most one click: a cascade, the
// model Core learns with). Mobile visitors want the decoction kit (item C), desktop visitors the
// espresso (item A, also the default order's head); the filter blend is lukewarm everywhere.
// So the learned order should put kaapi-decoction first on mobile and monday-espresso first on
// desktop.
export const GRID_TRUTH = {
  'monday-espresso': (c) => (c.mobile ? 0.04 : 0.18),
  'estate-filter': () => 0.06,
  'kaapi-decoction': (c) => (c.mobile ? 0.2 : 0.04),
};
