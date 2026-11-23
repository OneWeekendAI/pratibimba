import { useEffect, useMemo, useState } from 'react';
import { PICKLES } from '../data/shop';
import CartReminder from './CartReminder';
import ShelfGrid from './ShelfGrid';
import ShelfFilter from './ShelfFilter';

const COUNTS = {
  all: PICKLES.length,
  veg: PICKLES.filter((p) => p.veg).length,
  nonveg: PICKLES.filter((p) => !p.veg).length,
};

// #/shop, #/shop?veg=1 (v1 links still work), #/shop?diet=nonveg
function dietFromRoute(route) {
  const q = route.indexOf('?');
  const params = new URLSearchParams(q === -1 ? '' : route.slice(q));
  const diet = params.get('diet');
  if (diet === 'veg' || params.get('veg') === '1' || params.get('veg') === 'true') return 'veg';
  if (diet === 'nonveg') return 'nonveg';
  return 'all';
}

const SPOKEN = { all: 'all jars', veg: 'veg jars', nonveg: 'non-veg jars' };

export default function Shop({ route, cartCount }) {
  const [diet, setDiet] = useState(() => dietFromRoute(route));
  const [pulse, setPulse] = useState(0);
  const [said, setSaid] = useState('');

  useEffect(() => {
    setDiet(dietFromRoute(route));
  }, [route]);

  useEffect(() => {
    try {
      localStorage.setItem('pickly_veg_only', diet === 'veg' ? '1' : '0');
    } catch {
      /* ignore storage error */
    }
  }, [diet]);

  const choose = (next) => {
    if (next === diet) return;
    setDiet(next);
    setPulse((n) => n + 1);
    setSaid(`Showing ${COUNTS[next]} of ${COUNTS.all}: ${SPOKEN[next]}.`);
    // Keep the filter in the URL without a hashchange (which would scroll to top).
    const hash = next === 'veg' ? '#/shop?veg=1' : next === 'nonveg' ? '#/shop?diet=nonveg' : '#/shop';
    try {
      window.history.replaceState(null, '', hash);
    } catch {
      /* ignore */
    }
  };

  const isBack = useMemo(() => {
    if (diet === 'veg') return (p) => !p.veg;
    if (diet === 'nonveg') return (p) => p.veg;
    return null;
  }, [diet]);

  return (
    <div className="shop-page shelf-page">
      <CartReminder count={cartCount} />

      <div className="wrap shelf-page-wrap">
        <header className="shelf-page-head">
          <div>
            <p className="shelf-eyebrow">The pantry</p>
            <h1 className="h-display shelf-title">Every jar on the shelf</h1>
            <p className="shelf-sub">
              {COUNTS.veg} veg, {COUNTS.nonveg} non-veg. Pick one up to read its label.
            </p>
          </div>
          <ShelfFilter value={diet} onChange={choose} counts={COUNTS} />
        </header>
        <p className="sr-only" role="status" aria-live="polite">
          {said}
        </p>

        <ShelfGrid id="product-grid" pickles={PICKLES} isBack={isBack} pulse={pulse} variant="full" />
      </div>
    </div>
  );
}
