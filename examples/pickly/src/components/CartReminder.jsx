import { useEffect, useRef } from 'react';
import { SHOP } from '../data/shop';
import { useReducedMotion } from '../lib/motion';

/* Kraft tags hung on a length of twine above the shelf. They sway when a jar lands in the cart. */
export default function CartReminder({ count = 0 }) {
  const reduced = useReducedMotion();
  const line = useRef(null);

  useEffect(() => {
    if (reduced) return undefined;
    const sway = () => {
      line.current?.querySelectorAll('.shelf-tag').forEach((el, i) =>
        el.animate?.(
          [
            { transform: 'rotate(var(--tag-lean))' },
            { transform: `rotate(calc(var(--tag-lean) + ${i ? -5 : 5}deg))`, offset: 0.25 },
            { transform: `rotate(calc(var(--tag-lean) - ${i ? -2.5 : 2.5}deg))`, offset: 0.55 },
            { transform: 'rotate(calc(var(--tag-lean) + 0.8deg))', offset: 0.8 },
            { transform: 'rotate(var(--tag-lean))' },
          ],
          { duration: 1100, delay: i * 70, easing: 'cubic-bezier(.33,0,.4,1)' }
        )
      );
    };
    window.addEventListener('pickly:jar-landed', sway);
    return () => window.removeEventListener('pickly:jar-landed', sway);
  }, [reduced]);

  return (
    <div className="shelf-twine" ref={line}>
      <svg className="shelf-twine-line" viewBox="0 0 1000 24" preserveAspectRatio="none" aria-hidden="true">
        <path d="M0 4 Q 500 22 1000 4" />
      </svg>
      <div className="wrap shelf-twine-tags">
        {count > 0 && (
          <aside className="cart-reminder-band cart-reminder-active shelf-tag shelf-tag--cart" style={{ '--tag-lean': '-2deg' }}>
            <span>
              You have {count} {count === 1 ? 'jar' : 'jars'} ·{' '}
            </span>
            <a href="#/cart" className="cart-reminder-link">
              Review cart
            </a>
          </aside>
        )}
        <section id="cart-reminder" className="cart-reminder-band shelf-tag" style={{ '--tag-lean': '1.2deg' }}>
          <div className="cart-reminder-inner">
            <span>{SHOP.shippingNote}</span>
            <a href="#/shop" className="cart-reminder-link">
              Shop pickles
            </a>
          </div>
        </section>
      </div>
    </div>
  );
}
