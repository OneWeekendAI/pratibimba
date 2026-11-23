import { forwardRef } from 'react';
import { AnimatePresence, motion } from 'motion/react';
import { PICKLES, SHOP } from '../data/shop';
import { inr } from '../lib/helpers';
import { setQty, removeItem } from '../lib/cart';
import { useReducedMotion, SPRING_SOFT } from '../lib/motion';
import { Minus, Plus, Close, ArrowHand, Leaf, Drumstick, Back } from './icons';
import ParcelEmpty from './ParcelEmpty';

const SHIPPING_FLAT = 60;

/* Tags hang at slightly different angles; fixed per jar so they never jitter. */
const tilt = (id) => {
  let h = 0;
  for (let i = 0; i < id.length; i += 1) h = (h * 31 + id.charCodeAt(i)) % 997;
  return ((h % 7) - 3) * 0.8;
};

const JarSlot = forwardRef(function JarSlot({ item, reduced }, ref) {
  const MAX = 6;
  return (
    <motion.li
      ref={ref}
      className="pc-slot"
      layout={reduced ? false : 'position'}
      initial={false}
      exit={reduced ? { opacity: 0, transition: { duration: 0 } } : { x: 160, rotate: 7, opacity: 0, transition: { duration: 0.42, ease: [0.55, 0, 0.85, 0.35] } }}
      transition={SPRING_SOFT}
    >
      <a href={`#/product/${item.id}`} className="pc-jar" aria-label={`${item.name}, view jar`}>
        <img src={item.image} alt="" loading="lazy" />
      </a>
      <div className="pc-plank" aria-hidden="true" />
      {/* twine looped round the jar's foot, over the plank lip, down to its tag */}
      <svg className="pc-string" viewBox="0 0 40 64" aria-hidden="true" focusable="false">
        <path d="M8 3 C14 7 26 7 32 3" className="pc-string-loop" />
        <path d="M20 6 C19 22 21 40 20 58" />
        <circle cx="20" cy="7" r="3.2" className="pc-string-knot" />
      </svg>
      <div className="pc-tag" style={{ '--tilt': `${tilt(item.id)}deg` }}>
        <span className="pc-tag-hole" aria-hidden="true" />
        <button
          type="button"
          className="pc-remove"
          onClick={() => removeItem(item.id)}
          aria-label={`Remove ${item.name} from cart`}
        >
          <Close size={15} aria-hidden="true" />
        </button>
        <h2 className="pc-tag-name">
          <a href={`#/product/${item.id}`}>{item.name}</a>
        </h2>
        <p className="pc-tag-meta">
          {item.veg ? <Leaf size={13} title="Veg" /> : <Drumstick size={13} title="Non-veg" />}
          <span>{item.weightGrams} g · {inr(item.price)}</span>
        </p>
        <div className="pc-qty" role="group" aria-label={`${item.name} quantity`}>
          <button
            type="button"
            onClick={() => setQty(item.id, item.qty - 1)}
            aria-label={item.qty === 1 ? `Remove ${item.name}` : `Decrease ${item.name} quantity`}
          >
            <Minus size={16} aria-hidden="true" />
          </button>
          <span className="pc-qty-n" aria-live="polite">{item.qty}</span>
          <button
            type="button"
            onClick={() => setQty(item.id, Math.min(MAX, item.qty + 1))}
            aria-label={`Increase ${item.name} quantity`}
            disabled={item.qty >= MAX}
          >
            <Plus size={16} aria-hidden="true" />
          </button>
        </div>
        <p className="pc-tag-line">{inr(item.lineTotal)}</p>
      </div>
    </motion.li>
  );
});

export default function CartPage({ cart }) {
  const reduced = useReducedMotion();
  const lineItems = (cart.items || [])
    .map((item) => {
      const pickle = PICKLES.find((p) => p.id === item.id);
      if (!pickle) return null;
      return {
        ...pickle,
        qty: item.qty,
        lineTotal: pickle.price * item.qty,
      };
    })
    .filter(Boolean);

  const subtotal = lineItems.reduce((acc, it) => acc + it.lineTotal, 0);
  const total = subtotal > 0 ? subtotal + SHIPPING_FLAT : 0;
  const jarCount = lineItems.reduce((a, b) => a + b.qty, 0);

  if (!lineItems.length) {
    return <ParcelEmpty title="Your jar is empty" line="Not a drop of oil on the counter yet." />;
  }

  return (
    <div className="pc-cart wrap">
      <header className="pc-head">
        <span className="eyebrow">On the counter</span>
        <h1 className="h-display">
          {jarCount} {jarCount === 1 ? 'jar' : 'jars'}, ready to wrap
        </h1>
      </header>

      <div className="pc-cart-layout">
        <section
          className="pc-crate"
          aria-label="Jars in your cart"
          data-size={lineItems.length <= 2 ? 'few' : lineItems.length <= 3 ? 'some' : 'many'}
        >
          <ul className="pc-slots">
            <AnimatePresence initial={false} mode="popLayout">
              {lineItems.map((item) => (
                <JarSlot key={item.id} item={item} reduced={reduced} />
              ))}
            </AnimatePresence>
          </ul>
        </section>

        <aside className="pc-slip" aria-label="Order total">
          <p className="pc-slip-head">Counter slip</p>
          <ul className="pc-slip-lines">
            {lineItems.map((it) => (
              <li key={it.id}>
                <span className="pc-slip-item">{it.nativeName} <small>× {it.qty}</small></span>
                <span className="pc-hand">{inr(it.lineTotal)}</span>
              </li>
            ))}
          </ul>
          <dl className="pc-slip-sums">
            <div><dt>Subtotal</dt><dd className="pc-hand">{inr(subtotal)}</dd></div>
            <div><dt>Shipping, flat</dt><dd className="pc-hand">{inr(SHIPPING_FLAT)}</dd></div>
            <div className="pc-slip-total"><dt>Total</dt><dd className="pc-hand">{inr(total)}</dd></div>
          </dl>
          <p className="pc-fine">{SHOP.shippingNote}</p>
          <a href="#/checkout" className="pc-cta">
            <span>Wrap it up</span>
            <ArrowHand size={20} aria-hidden="true" />
          </a>
          <a href="#/shop" className="pc-back pc-back-slip">
            <Back size={15} aria-hidden="true" /> Back to the shelf
          </a>
        </aside>
      </div>
    </div>
  );
}
